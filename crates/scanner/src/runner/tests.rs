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
            valid_until: last,
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

fn fake(
    id: &'static str,
    rr: i64,
    stock_trend: Trend,
    seen: &Arc<Mutex<Vec<(usize, NaiveDate)>>>,
) -> Box<dyn Strategy> {
    let cells: Vec<_> = [VolState::Low, VolState::Normal, VolState::High]
        .into_iter()
        .map(|v| (stock_trend, v))
        .collect();
    Box::new(Fake {
        meta: StrategyMeta {
            id,
            name: id,
            version: 1,
            min_history_bars: 1,
            regime_fit: RegimeFit {
                market_trends: vec![Trend::Up, Trend::Range],
                stock_cells: cells,
                weak_cells: vec![],
            },
            penalty_weight: 0.5,
        },
        rr,
        seen: Arc::clone(seen),
    })
}

fn day(i: usize) -> DateTime<Tz> {
    let base = Kolkata.with_ymd_and_hms(2025, 1, 1, 15, 30, 0).unwrap();
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
        circuit_locked: false,
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
    let pf = PortfolioView::default();
    let clock = FixedClock(day(N - 1));
    let cx = ScanContext {
        cfg: e.cfg.scanner.as_ref().unwrap(),
        regime: &e.regime,
        calendar: &e.cal,
        portfolio: &pf,
        ban_list: ban,
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
    locked.circuit_locked = true;
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
