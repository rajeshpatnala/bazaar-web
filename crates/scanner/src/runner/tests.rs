use std::sync::{Arc, Mutex};

use bazaar_core::candle::Interval;
use bazaar_core::config::{self, CalendarConfig};
use bazaar_core::instrument::{Exchange, InstrumentType};
use bazaar_regime::{RegimeFit, Trend, VolState};
use bazaar_strategies::{ExitAction, ExitError, ExitPlan, OpenPosition, StrategyMeta};
use chrono::{DateTime, TimeZone};
use chrono_tz::{Asia::Kolkata, Tz};

use super::*;
use crate::ports::BanListStatus;

struct FixedClock(DateTime<Tz>);
impl Clock for FixedClock {
    fn now_ist(&self) -> DateTime<Tz> {
        self.0
    }
}

struct FakeBan(BanListStatus);
impl BanList for FakeBan {
    fn status(&self) -> BanListStatus {
        self.0.clone()
    }
}

/// Fires a signal at every bar it sees and records what the view showed.
struct Fake {
    meta: StrategyMeta,
    rr: i64,
    /// Hours after the bar close that the signal stays valid.
    valid_hours: i64,
    seen: Arc<Mutex<Vec<(usize, NaiveDate)>>>,
}

impl Strategy for Fake {
    fn meta(&self) -> &StrategyMeta {
        &self.meta
    }
    fn on_bar(&mut self, view: &DataView<'_>, ctx: &StrategyContext<'_>) -> Option<Signal> {
        let last = view.last()?.close_time();
        self.seen
            .lock()
            .unwrap()
            .push((view.len(), last.date_naive()));
        let d = Decimal::from;
        Some(Signal {
            strategy_id: self.meta.id,
            instrument_token: ctx.instrument.instrument_token,
            strategy_version: 1,
            params_hash: "h".into(),
            signal_bar_time: last,
            entry_low: d(100),
            entry_high: d(102),
            stop: d(94),
            targets: vec![d(110)],
            risk_per_share: d(8),
            reference_rr: d(self.rr),
            exit_plan: ExitPlan {
                time_stop_bars: 10,
                rule_text: "t".into(),
            },
            facts: vec![],
            valid_until: last + chrono::Duration::hours(self.valid_hours),
        })
    }
    fn manage(
        &mut self,
        _: &DataView<'_>,
        _: &StrategyContext<'_>,
        _: &OpenPosition,
    ) -> Result<ExitAction, ExitError> {
        Ok(ExitAction::Hold)
    }
}

type Seen = Arc<Mutex<Vec<(usize, NaiveDate)>>>;

fn fake(id: &'static str, rr: i64, stock_trend: Trend, seen: &Seen) -> Box<dyn Strategy> {
    fake_with(id, rr, stock_trend, false, seen)
}

/// `weak`: every declared stock cell is also a weak cell (a `Fit::Penalised` hit).
fn fake_with(
    id: &'static str,
    rr: i64,
    stock_trend: Trend,
    weak: bool,
    seen: &Seen,
) -> Box<dyn Strategy> {
    Box::new(fake_raw(id, rr, stock_trend, weak, seen))
}

fn fake_struct(id: &'static str, seen: &Seen) -> Fake {
    fake_raw(id, 2, Trend::Up, false, seen)
}

fn fake_raw(id: &'static str, rr: i64, stock_trend: Trend, weak: bool, seen: &Seen) -> Fake {
    let cells: Vec<_> = [VolState::Low, VolState::Normal, VolState::High]
        .into_iter()
        .map(|v| (stock_trend, v))
        .collect();
    Fake {
        meta: StrategyMeta {
            id,
            name: id,
            version: 1,
            min_history_bars: 1,
            regime_fit: RegimeFit {
                market_trends: vec![Trend::Up, Trend::Range],
                weak_cells: if weak { cells.clone() } else { vec![] },
                stock_cells: cells,
            },
            penalty_weight: 0.5,
        },
        rr,
        valid_hours: 96,
        seen: Arc::clone(seen),
    }
}

fn day(i: usize) -> DateTime<Tz> {
    let base = Kolkata.with_ymd_and_hms(2025, 12, 29, 15, 30, 0).unwrap();
    base + chrono::Duration::days(i64::try_from(i).unwrap())
}

/// Steadily rising series: a settled Up trend for both market and stock.
fn rising(n: usize, vol: u64) -> Vec<Candle> {
    (0..n)
        .map(|i| {
            let c = Decimal::from(100) + Decimal::from(i64::try_from(i).unwrap()) / Decimal::TWO;
            let (h, l) = (c + Decimal::ONE, c - Decimal::ONE);
            Candle::new(day(i), Interval::Day, [c, h, l, c], vol, true).unwrap()
        })
        .collect()
}

fn entry(sym: &str, bars: Vec<Candle>) -> UniverseEntry {
    UniverseEntry {
        instrument: Instrument {
            instrument_token: 1,
            exchange_token: 1,
            tradingsymbol: sym.into(),
            name: sym.into(),
            exchange: Exchange::parse("NSE"),
            instrument_type: InstrumentType::parse("EQ"),
            segment: "NSE".into(),
            expiry: None,
            strike: None,
            tick_size: Decimal::new(5, 2),
            lot_size: 1,
        },
        bars,
        circuit: CircuitState::Free,
    }
}

struct Env {
    cfg: StrategiesConfig,
    regime: RegimeParams,
    cal: MarketCalendar,
}

fn env() -> Env {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config");
    let cfg: StrategiesConfig = config::load(&dir).unwrap();
    Env {
        regime: RegimeParams::from_config(&cfg).unwrap(),
        cal: MarketCalendar::from_config(&config::load::<CalendarConfig>(&dir).unwrap()),
        cfg,
    }
}

const N: usize = 320;

fn fresh_ban(symbols: &[&str]) -> FakeBan {
    FakeBan(BanListStatus::Available {
        symbols: symbols.iter().map(|s| (*s).to_owned()).collect(),
        as_of: Some(day(N - 1)),
    })
}

fn scan(
    e: &Env,
    ban: &FakeBan,
    strategies: &mut [Box<dyn Strategy>],
    uni: &[UniverseEntry],
    market: &[Candle],
) -> Result<ScanOutput, ScanError> {
    scan_at(e, ban, 24, day(N - 1), strategies, uni, market)
}

fn scan_at(
    e: &Env,
    ban: &FakeBan,
    max_age_hours: u32,
    now: DateTime<Tz>,
    strategies: &mut [Box<dyn Strategy>],
    uni: &[UniverseEntry],
    market: &[Candle],
) -> Result<ScanOutput, ScanError> {
    let pf = PortfolioView::default();
    let clock = FixedClock(now);
    let cx = ScanContext {
        cfg: e.cfg.scanner.as_ref().unwrap(),
        regime: &e.regime,
        calendar: &e.cal,
        portfolio: &pf,
        ban_list: ban,
        ban_list_max_age_hours: max_age_hours,
        clock: &clock,
    };
    run(&cx, strategies, uni, market, day(N - 1).date_naive())
}

fn syms(o: &ScanOutput) -> Vec<&str> {
    o.candidates
        .iter()
        .map(|c| c.instrument.tradingsymbol.as_str())
        .collect()
}

#[test]
fn a_clean_name_with_a_fitting_regime_becomes_a_candidate() {
    let (e, seen) = (env(), Arc::default());
    let mut s = vec![fake("a", 2, Trend::Up, &seen)];
    let m = rising(N, 1);
    let o = scan(
        &e,
        &fresh_ban(&[]),
        &mut s,
        &[entry("AAA", rising(N, 1_000_000))],
        &m,
    )
    .unwrap();
    assert_eq!(syms(&o), ["AAA"]);
    assert!(matches!(o.candidates[0].hits[0].fit, Fit::Allowed(_)));
}

#[test]
fn banned_illiquid_locked_short_and_dateless_names_never_appear() {
    let (e, seen) = (env(), Arc::default());
    let mut s = vec![fake("a", 2, Trend::Up, &seen)];
    let m = rising(N, 1);
    let mut locked = entry("LOCK", rising(N, 1_000_000));
    locked.circuit = CircuitState::Locked;
    let uni = [
        entry("good", rising(N, 1_000_000)),
        entry("BAN", rising(N, 1_000_000)),
        entry("THIN", rising(N, 1)),
        locked,
        entry("SHORT", rising(N, 1_000_000).split_off(N - 100)),
        entry("OLD", rising(N - 5, 1_000_000)),
    ];
    let o = scan(&e, &fresh_ban(&[" ban "]), &mut s, &uni, &m).unwrap();
    assert_eq!(syms(&o), ["good"]);
    let why: BTreeMap<_, _> = o.dropped.iter().cloned().collect();
    assert_eq!(why["BAN"], DropReason::Banned);
    assert!(matches!(why["THIN"], DropReason::Illiquid { .. }));
    assert_eq!(why["LOCK"], DropReason::CircuitLocked);
    assert!(matches!(
        why["SHORT"],
        DropReason::ShortHistory { bars: 100, .. }
    ));
    assert_eq!(why["OLD"], DropReason::NoBarOnDecisionDate);
}

#[test]
fn an_unavailable_or_stale_ban_list_drops_every_name_with_a_reason() {
    let (e, seen) = (env(), Arc::default());
    let mut s = vec![fake("a", 2, Trend::Up, &seen)];
    let m = rising(N, 1);
    let uni = [
        entry("AAA", rising(N, 1_000_000)),
        entry("BBB", rising(N, 1_000_000)),
    ];
    let down = FakeBan(BanListStatus::Unavailable { reason: "x".into() });
    let o = scan(&e, &down, &mut s, &uni, &m).unwrap();
    assert_eq!(o.candidates.len(), 0);
    assert_eq!(o.dropped.len(), 2);
    assert!(matches!(o.dropped[0].1, DropReason::BanListUnavailable(_)));
    let old = FakeBan(BanListStatus::Available {
        symbols: vec![],
        as_of: Some(day(N - 3)),
    });
    let o = scan(&e, &old, &mut s, &uni, &m).unwrap();
    assert!(matches!(o.dropped[0].1, DropReason::BanListStale { .. }));
    assert!(
        seen.lock().unwrap().is_empty(),
        "no strategy ran on dropped names"
    );
}

#[test]
fn same_family_counts_once_and_different_families_stay_apart() {
    let (e, seen) = (env(), Arc::default());
    let mut s = vec![
        fake("trend_pullback", 2, Trend::Up, &seen),
        fake("donchian_breakout", 3, Trend::Up, &seen),
        fake("custom_x", 1, Trend::Up, &seen),
        fake("custom_x", 1, Trend::Up, &seen),
    ];
    let m = rising(N, 1);
    let o = scan(
        &e,
        &fresh_ban(&[]),
        &mut s,
        &[entry("AAA", rising(N, 1_000_000))],
        &m,
    )
    .unwrap();
    let fams: Vec<_> = o.candidates[0]
        .hits
        .iter()
        .map(|h| h.family.as_str())
        .collect();
    assert_eq!(fams, ["breakout", "custom_x", "trend"]);
    assert_eq!(o.candidates[0].hits[1].merged_ids, ["custom_x"]);
}

#[test]
fn the_best_of_a_family_wins_and_the_loser_is_listed() {
    assert_eq!(family_of("bollinger_reversion"), "reversion");
    let (e, seen) = (env(), Arc::default());
    // Same family via the unmapped-id rule is not possible, so reuse one id with two rr values.
    let mut s = vec![
        fake("x", 1, Trend::Up, &seen),
        fake("x", 4, Trend::Up, &seen),
    ];
    let m = rising(N, 1);
    let o = scan(
        &e,
        &fresh_ban(&[]),
        &mut s,
        &[entry("AAA", rising(N, 1_000_000))],
        &m,
    )
    .unwrap();
    let hit = &o.candidates[0].hits[0];
    assert_eq!(hit.signal.reference_rr, Decimal::from(4));
    assert_eq!(hit.merged_ids.len(), 1);
}

#[test]
fn a_wrong_regime_cell_gives_no_signal_and_the_strategy_is_not_run() {
    let (e, seen) = (env(), Arc::default());
    let mut s = vec![fake("a", 2, Trend::Range, &seen)];
    let m = rising(N, 1);
    let o = scan(
        &e,
        &fresh_ban(&[]),
        &mut s,
        &[entry("AAA", rising(N, 1_000_000))],
        &m,
    )
    .unwrap();
    assert_eq!(o.candidates.len(), 0);
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn lookahead_bars_after_the_decision_bar_change_nothing() {
    let e = env();
    let m_short = rising(N, 1);
    let a_short = rising(N, 1_000_000);
    // Same series continued with 15 extra, very different bars after the decision date.
    let mut m_long = m_short.clone();
    let mut a_long = a_short.clone();
    for i in N..N + 15 {
        let p = Decimal::from(5);
        let c = Candle::new(day(i), Interval::Day, [p, p, p, p], 1, true).unwrap();
        m_long.push(c.clone());
        a_long.push(c);
    }
    let (seen_a, seen_b) = (Arc::default(), Arc::default());
    let mut sa = vec![fake("a", 2, Trend::Up, &seen_a)];
    let mut sb = vec![fake("a", 2, Trend::Up, &seen_b)];
    let ban = fresh_ban(&[]);
    let oa = scan(&e, &ban, &mut sa, &[entry("AAA", a_short)], &m_short).unwrap();
    let ob = scan(&e, &ban, &mut sb, &[entry("AAA", a_long)], &m_long).unwrap();
    assert_eq!(oa, ob);
    let want = vec![(N, day(N - 1).date_naive())];
    assert_eq!(*seen_a.lock().unwrap(), want);
    assert_eq!(
        *seen_b.lock().unwrap(),
        want,
        "the view stops at the decision bar"
    );
}

#[test]
fn run_errors_without_a_market_bar_and_build_needs_the_scanner_table() {
    let (e, seen) = (env(), Arc::default());
    let mut s = vec![fake("a", 2, Trend::Up, &seen)];
    assert!(matches!(
        scan(&e, &fresh_ban(&[]), &mut s, &[], &rising(10, 1)),
        Err(ScanError::NoMarketBar(_))
    ));
    let mut cfg = e.cfg.clone();
    assert!(build(&cfg).is_ok());
    cfg.scanner = None;
    assert!(matches!(build(&cfg), Err(ScanError::NoScannerConfig)));
}

fn falling(n: usize) -> Vec<Candle> {
    (0..n)
        .map(|i| {
            let c = Decimal::from(1000) - Decimal::from(i64::try_from(i).unwrap()) / Decimal::TWO;
            let (h, l) = (c + Decimal::ONE, c - Decimal::ONE);
            Candle::new(day(i), Interval::Day, [c, h, l, c], 1, true).unwrap()
        })
        .collect()
}

fn dropped(o: &ScanOutput, sym: &str) -> DropReason {
    o.dropped
        .iter()
        .find(|(s, _)| s == sym)
        .map(|(_, r)| r.clone())
        .unwrap()
}

#[test]
fn a_weak_cell_is_kept_as_penalised_with_its_penalty_weight() {
    let (e, seen) = (env(), Seen::default());
    let mut s = vec![fake_with("a", 2, Trend::Up, true, &seen)];
    let o = scan(
        &e,
        &fresh_ban(&[]),
        &mut s,
        &[entry("AAA", rising(N, 1_000_000))],
        &rising(N, 1),
    )
    .unwrap();
    let h = &o.candidates[0].hits[0];
    assert!(matches!(h.fit, Fit::Penalised(_)));
    assert!((h.penalty_weight - 0.5).abs() < f64::EPSILON);
}

#[test]
fn an_allowed_hit_beats_a_penalised_one_of_the_same_family_even_with_lower_rr() {
    let (e, seen) = (env(), Seen::default());
    let mut s = vec![
        fake_with("x", 4, Trend::Up, true, &seen),
        fake_with("x", 1, Trend::Up, false, &seen),
    ];
    let o = scan(
        &e,
        &fresh_ban(&[]),
        &mut s,
        &[entry("AAA", rising(N, 1_000_000))],
        &rising(N, 1),
    )
    .unwrap();
    let h = &o.candidates[0].hits[0];
    assert!(matches!(h.fit, Fit::Allowed(_)));
    assert_eq!(h.signal.reference_rr, Decimal::from(1));
}

#[test]
fn a_falling_market_gives_no_candidates_and_no_strategy_call() {
    let (e, seen) = (env(), Seen::default());
    let mut s = vec![fake("a", 2, Trend::Up, &seen)];
    let o = scan(
        &e,
        &fresh_ban(&[]),
        &mut s,
        &[entry("AAA", rising(N, 1_000_000))],
        &falling(N),
    )
    .unwrap();
    assert_eq!(o.candidates.len(), 0);
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn a_series_that_cannot_be_classified_is_dropped_as_regime_failed() {
    let (e, seen) = (env(), Seen::default());
    let mut s = vec![fake("a", 2, Trend::Up, &seen)];
    let mut bars = rising(N, 1_000_000);
    bars.swap(10, 11);
    let o = scan(
        &e,
        &fresh_ban(&[]),
        &mut s,
        &[entry("BAD", bars)],
        &rising(N, 1),
    )
    .unwrap();
    assert!(matches!(dropped(&o, "BAD"), DropReason::RegimeFailed(_)));
}

#[test]
fn circuit_unknown_and_locked_drop_the_name_and_free_passes() {
    let (e, seen) = (env(), Seen::default());
    let mut s = vec![fake("a", 2, Trend::Up, &seen)];
    let mut unknown = entry("UNK", rising(N, 1_000_000));
    unknown.circuit = CircuitState::Unknown;
    let mut locked = entry("LCK", rising(N, 1_000_000));
    locked.circuit = CircuitState::Locked;
    let uni = [unknown, locked, entry("FRE", rising(N, 1_000_000))];
    let o = scan(&e, &fresh_ban(&[]), &mut s, &uni, &rising(N, 1)).unwrap();
    assert_eq!(syms(&o), ["FRE"]);
    assert_eq!(dropped(&o, "UNK"), DropReason::CircuitStatusUnknown);
    assert_eq!(dropped(&o, "LCK"), DropReason::CircuitLocked);
}

#[test]
fn the_scanner_key_cannot_loosen_the_ban_list_age_limit() {
    assert_eq!(effective_ban_age_hours(720, 24, 24), 24);
    assert_eq!(effective_ban_age_hours(12, 24, 24), 12);
    let (mut e, seen) = (env(), Seen::default());
    e.cfg.scanner.as_mut().unwrap().max_ban_list_age_hours = 720;
    let age = effective_ban_age_hours(720, 24, 24);
    let mut s = vec![fake("a", 2, Trend::Up, &seen)];
    let uni = [entry("AAA", rising(N, 1_000_000))];
    let list = |h| {
        FakeBan(BanListStatus::Available {
            symbols: vec![],
            as_of: Some(day(N - 1) - chrono::Duration::hours(h)),
        })
    };
    let m = rising(N, 1);
    let o = scan_at(&e, &list(25), age, day(N - 1), &mut s, &uni, &m).unwrap();
    assert!(matches!(
        dropped(&o, "AAA"),
        DropReason::BanListStale { .. }
    ));
    let o = scan_at(&e, &list(24), age, day(N - 1), &mut s, &uni, &m).unwrap();
    assert_eq!(syms(&o), ["AAA"]);
}

#[test]
fn liquidity_floor_is_inclusive_at_the_median() {
    // Last 20 closes run 250.0..=259.5, so the median close is 254.75; volume 4 gives 1019.
    let (mut e, seen) = (env(), Seen::default());
    let mut s = vec![fake("a", 2, Trend::Up, &seen)];
    let uni = [entry("AAA", rising(N, 4))];
    let m = rising(N, 1);
    e.cfg.scanner.as_mut().unwrap().min_median_traded_value = Decimal::from(1019);
    assert_eq!(
        syms(&scan(&e, &fresh_ban(&[]), &mut s, &uni, &m).unwrap()),
        ["AAA"]
    );
    e.cfg.scanner.as_mut().unwrap().min_median_traded_value = Decimal::from(1020);
    let o = scan(&e, &fresh_ban(&[]), &mut s, &uni, &m).unwrap();
    assert!(matches!(dropped(&o, "AAA"), DropReason::Illiquid { .. }));
}

#[test]
fn a_decision_bar_after_the_clock_is_not_closed() {
    let (env_, seen) = (env(), Seen::default());
    let mut strats = vec![fake("a", 2, Trend::Up, &seen)];
    let universe = [entry("AAA", rising(N, 1_000_000))];
    let mkt = rising(N, 1);
    let before = day(N - 1) - chrono::Duration::hours(1);
    let ban = FakeBan(BanListStatus::Available {
        symbols: vec![],
        as_of: Some(before),
    });
    assert!(matches!(
        scan_at(&env_, &ban, 24, before, &mut strats, &universe, &mkt),
        Err(ScanError::DecisionBarOpen)
    ));
    // Market bar closed, stock bar stamped after the clock.
    let mut late = rising(N, 1_000_000);
    late.pop();
    let px = Decimal::from(200);
    let cndl = Candle::new(
        day(N - 1) + chrono::Duration::hours(8),
        Interval::Day,
        [px, px, px, px],
        1,
        true,
    );
    late.push(cndl.unwrap());
    let o = scan(
        &env_,
        &fresh_ban(&[]),
        &mut strats,
        &[entry("LATE", late)],
        &mkt,
    )
    .unwrap();
    assert_eq!(dropped(&o, "LATE"), DropReason::DecisionBarNotClosed);
}

#[test]
fn every_registered_strategy_has_an_explicit_family() {
    for f in bazaar_strategies::registry::factories() {
        assert_ne!(family_of(f.id()), f.id(), "{} needs a family arm", f.id());
    }
}

fn scan_on(
    e: &Env,
    now: DateTime<Tz>,
    decision: NaiveDate,
    strategies: &mut [Box<dyn Strategy>],
    uni: &[UniverseEntry],
    market: &[Candle],
) -> Result<ScanOutput, ScanError> {
    let (pf, clock) = (PortfolioView::default(), FixedClock(now));
    let ban = FakeBan(BanListStatus::Available {
        symbols: vec![],
        as_of: Some(now),
    });
    let cx = ScanContext {
        cfg: e.cfg.scanner.as_ref().unwrap(),
        regime: &e.regime,
        calendar: &e.cal,
        portfolio: &pf,
        ban_list: &ban,
        ban_list_max_age_hours: 24,
        clock: &clock,
    };
    run(&cx, strategies, uni, market, decision)
}

fn ist(d: u32, h: u32, m: u32) -> DateTime<Tz> {
    Kolkata.with_ymd_and_hms(2026, 11, d, h, m, 0).unwrap()
}

#[test]
fn a_friday_bar_scanned_on_monday_is_current_but_scanned_on_tuesday_is_stale() {
    let (e, seen) = (env(), Seen::default());
    let (uni, m) = ([entry("AAA", rising(N, 1_000_000))], rising(N, 1));
    let friday = day(N - 1).date_naive();
    assert_eq!(friday.to_string(), "2026-11-13");
    let mut s = vec![fake("a", 2, Trend::Up, &seen)];
    // Weekend and Monday before the close: the latest completed session is still Friday.
    for now in [ist(14, 12, 0), ist(15, 12, 0), ist(16, 9, 0)] {
        assert!(scan_on(&e, now, friday, &mut s, &uni, &m).is_ok(), "{now}");
    }
    // Monday after the close: Monday is the latest completed session, so Friday is stale.
    assert!(matches!(
        scan_on(&e, ist(16, 15, 30), friday, &mut s, &uni, &m),
        Err(ScanError::StaleDecisionDate { decision, latest })
            if decision == friday && latest.to_string() == "2026-11-16"
    ));
}

#[test]
fn a_holiday_gap_is_not_stale() {
    // 2026-11-10 (Tuesday) is a holiday in config/calendar.toml: on Wednesday morning the
    // latest completed session is Monday the 9th, so a Monday decision date is current.
    let e = env();
    let latest = latest_completed_session(&e.cal, ist(11, 9, 0)).unwrap();
    assert_eq!(latest.to_string(), "2026-11-09");
}

#[test]
fn the_bar_still_open_at_1529_is_the_existing_decision_bar_open_error_not_stale() {
    let (e, seen) = (env(), Seen::default());
    let (uni, m) = ([entry("AAA", rising(N, 1_000_000))], rising(N, 1));
    let mut s = vec![fake("a", 2, Trend::Up, &seen)];
    let r = scan_on(
        &e,
        ist(13, 15, 29),
        day(N - 1).date_naive(),
        &mut s,
        &uni,
        &m,
    );
    assert!(matches!(r, Err(ScanError::DecisionBarOpen)));
    let r = scan_on(
        &e,
        ist(13, 15, 30),
        day(N - 1).date_naive(),
        &mut s,
        &uni,
        &m,
    );
    assert!(r.is_ok());
}

#[test]
fn a_signal_past_its_valid_until_is_dropped_with_a_reason() {
    let (e, seen) = (env(), Seen::default());
    let (uni, m) = ([entry("AAA", rising(N, 1_000_000))], rising(N, 1));
    let mut s = vec![Box::new(Fake {
        valid_hours: 0,
        ..fake_struct("a", &seen)
    }) as Box<dyn Strategy>];
    let o = scan_on(
        &e,
        ist(13, 15, 30),
        day(N - 1).date_naive(),
        &mut s,
        &uni,
        &m,
    )
    .unwrap();
    assert_eq!(o.candidates.len(), 0);
    assert_eq!(
        dropped(&o, "AAA"),
        DropReason::SignalExpired {
            strategy_id: "a".into()
        }
    );
}

#[test]
fn families_total_counts_enabled_families_not_hits() {
    let (e, seen) = (env(), Seen::default());
    let (uni, m) = ([entry("AAA", rising(N, 1_000_000))], rising(N, 1));
    // Two strategies of one family plus one of another; only the first of them fires.
    let mut s = vec![
        fake("donchian_breakout", 2, Trend::Up, &seen),
        fake("trend_pullback", 2, Trend::Down, &seen),
        fake("bollinger_reversion", 2, Trend::Down, &seen),
    ];
    let o = scan_on(
        &e,
        ist(13, 15, 30),
        day(N - 1).date_naive(),
        &mut s,
        &uni,
        &m,
    )
    .unwrap();
    assert_eq!(o.candidates[0].hits.len(), 1);
    assert_eq!(o.families_total, 3);
    let mut one = vec![fake("donchian_breakout", 2, Trend::Up, &seen)];
    let o = scan_on(
        &e,
        ist(13, 15, 30),
        day(N - 1).date_naive(),
        &mut one,
        &uni,
        &m,
    )
    .unwrap();
    assert_eq!(o.families_total, 1);
    let mut none: Vec<Box<dyn Strategy>> = vec![];
    assert!(matches!(
        scan_on(
            &e,
            ist(13, 15, 30),
            day(N - 1).date_naive(),
            &mut none,
            &uni,
            &m
        ),
        Err(ScanError::NoStrategiesEnabled)
    ));
}

#[test]
fn a_short_shaped_signal_is_rejected_by_the_signal_contract() {
    // CR-114-3 / D8: long-only by type, so no Buy-versus-Sell conflict can reach the runner.
    let seen = Seen::default();
    let mut f = fake_struct("a", &seen);
    let view_bars = rising(40, 1);
    let view = DataView::ending_at(&view_bars, 39).unwrap();
    let e = env();
    let inst = entry("AAA", vec![]).instrument;
    let (pf, mreg) = (
        PortfolioView::default(),
        RegimeState::unknown(day(39), bazaar_regime::Fact::new("a", "b", "c")),
    );
    let ctx = StrategyContext {
        instrument: &inst,
        market_regime: &mreg,
        stock_regime: &mreg,
        portfolio: &pf,
        calendar: &e.cal,
    };
    let mut sig = f.on_bar(&view, &ctx).unwrap();
    // Short shape: stop above the entry zone.
    sig.stop = Decimal::from(110);
    sig.risk_per_share = sig.entry_high - sig.stop;
    assert_eq!(
        sig.validate(),
        Err(bazaar_strategies::SignalError::StopNotBelowEntry)
    );
#[test]
fn same_inputs_give_identical_output_whatever_the_universe_order() {
    let cfg_env = env();
    let market = rising(N, 1);
    let ban = fresh_ban(&["BAN"]);
    // Distinct tokens; ZZZ and AAA score the same, so symbol (then token) decides the order.
    let mk = || {
        let mut uni = vec![
            entry("ZZZ", rising(N, 1_000_000)),
            entry("AAA", rising(N, 1_000_000)),
            entry("BAN", rising(N, 1_000_000)),
            entry("THIN", rising(N, 1)),
        ];
        for (token, e) in (101_u64..).zip(uni.iter_mut()) {
            e.instrument.instrument_token = token;
        }
        uni
    };
    let fwd = mk();
    let mut rev = mk();
    rev.reverse();
    let (seen1, seen2, seen3) = (Arc::default(), Arc::default(), Arc::default());
    let mut strat1 = vec![fake("a", 2, Trend::Up, &seen1)];
    let mut strat2 = vec![fake("a", 2, Trend::Up, &seen2)];
    let mut strat3 = vec![fake("a", 2, Trend::Up, &seen3)];
    let o1 = scan(&cfg_env, &ban, &mut strat1, &fwd, &market).unwrap();
    let o2 = scan(&cfg_env, &ban, &mut strat2, &rev, &market).unwrap();
    let o3 = scan(&cfg_env, &ban, &mut strat3, &fwd, &market).unwrap();
    assert_eq!(o1, o2, "input order must not change the output");
    assert_eq!(o1, o3, "a repeat run is identical");
    assert_eq!(syms(&o1), ["AAA", "ZZZ"]);
    let tokens: Vec<_> = o1
        .candidates
        .iter()
        .map(|c| c.instrument.instrument_token)
        .collect();
    assert_eq!(
        tokens,
        [102, 101],
        "tokens follow their symbols, not input position"
    );
    let why: BTreeMap<_, _> = o1.dropped.iter().cloned().collect();
    assert_eq!(why["BAN"], DropReason::Banned);
    assert!(matches!(why["THIN"], DropReason::Illiquid { .. }));
}
