use std::path::{Path, PathBuf};
use std::str::FromStr;

use bazaar_core::calendar::MarketCalendar;
use bazaar_core::candle::{Candle, Interval};
use bazaar_core::config::{self, CalendarConfig, ChargeExchange, ChargesConfig};
use bazaar_core::dataview::DataView;
use bazaar_core::instrument::{Exchange, Instrument, InstrumentType};
use bazaar_core::segment::{InstrumentKind, Segment};
use bazaar_regime::{Fact, RegimeState};
use bazaar_risk::config::EffectiveLimits;
use bazaar_strategies::{
    ExitAction, ExitPlan, OpenPosition, PortfolioView, Strategy, StrategyContext, StrategyMeta,
};
use chrono::{Duration, TimeZone};

use super::*;
use crate::config::BacktestConfig;
use crate::cost::CostModel;
use crate::engine::{RegimeProvider, RunOutput, run};
use crate::types::{HistoryPolicy, Product};

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config")
}

fn at(day: &str) -> NaiveDate {
    NaiveDate::from_str(day).unwrap()
}

/// Everything a run borrows, owned in one place.
struct Rig {
    cal: MarketCalendar,
    inst: Instrument,
    cfg: BacktestConfig,
    charges: ChargesConfig,
    gate: GateConfig,
    limits: EffectiveLimits,
    bars: Vec<Candle>,
}

/// `[open, high, low, close]` rows from the first Monday after 2026-11-20.
fn rig(rows: &[[&str; 4]]) -> Rig {
    let cal = MarketCalendar::from_config(&config::load::<CalendarConfig>(&dir()).unwrap());
    let mut day = at("2026-11-23");
    let mut bars = Vec::new();
    for r in rows {
        let close = cal
            .close_time(day, Segment::Equity, InstrumentKind::CashNonFno)
            .unwrap()
            .with_timezone(&chrono_tz::Asia::Kolkata);
        bars.push(Candle::new(close, Interval::Day, r.map(d), 1000, true).unwrap());
        day = cal.next_trading_day(day, Segment::Equity).unwrap();
    }
    let gate =
        GateConfig::from_toml_str(&std::fs::read_to_string(dir().join("risk.toml")).unwrap())
            .unwrap();
    let limits = gate.effective(&Baseline::none());
    Rig {
        cal,
        inst: Instrument {
            instrument_token: 7,
            exchange_token: 7,
            tradingsymbol: "ABC".into(),
            name: "ABC".into(),
            exchange: Exchange::parse("NSE"),
            instrument_type: InstrumentType::parse("EQ"),
            segment: "NSE".into(),
            expiry: None,
            strike: None,
            tick_size: d("0.05"),
            lot_size: 1,
        },
        cfg: BacktestConfig::load(&dir()).unwrap(),
        charges: config::load::<ChargesConfig>(&dir()).unwrap(),
        gate,
        limits,
        bars,
    }
}

impl Rig {
    fn spec(&self) -> RunSpec<'_> {
        RunSpec {
            bars: &self.bars,
            instrument: &self.inst,
            calendar: &self.cal,
            config: &self.cfg,
            cost: CostModel::new(&self.charges, HistoryPolicy::Strict, ChargeExchange::Nse),
            limits: &self.limits,
            product: Product::Delivery,
            capital: d("100000"),
            slippage_multiplier: d("1"),
            warmup: 0,
        }
    }
}

/// A quiet day inside the entry zone: close 100.50.
const FLAT: [&str; 4] = ["100", "101", "99.5", "100.5"];
/// A day that trades down through the 96 stop.
const DIP: [&str; 4] = ["100", "101", "95", "100.5"];

fn good_evidence() -> EvidenceInputs {
    EvidenceInputs {
        oos_trades: Tri::Known(200),
        profit_factor: Tri::Known(d("3")),
        expectancy_r: Tri::Known(d("1")),
        walk_forward_pass: Tri::Known(true),
    }
}

fn quiet(_: NaiveDate) -> EventFlags {
    EventFlags {
        results_window: Tri::Known(false),
        policy_day: Tri::Known(false),
        option_on_own_expiry: Tri::Known(false),
    }
}

fn results_day(_: NaiveDate) -> EventFlags {
    EventFlags {
        results_window: Tri::Known(true),
        ..quiet(at("2026-11-23"))
    }
}

fn env(evidence: EvidenceInputs, events: EventsFor<'_>) -> GateEnv<'_> {
    GateEnv {
        evidence,
        sector: "IT".into(),
        events,
    }
}

/// Fires at the listed decision indexes with a 100-101 zone and a 96 stop.
struct Fire(Vec<usize>, StrategyMeta);

fn fire(at: &[usize]) -> Fire {
    Fire(
        at.to_vec(),
        StrategyMeta {
            id: "fire",
            name: "Fire",
            version: 1,
            min_history_bars: 1,
            regime_fit: bazaar_regime::RegimeFit {
                market_trends: vec![],
                stock_cells: vec![],
                weak_cells: vec![],
            },
            penalty_weight: 0.5,
        },
    )
}

fn signal_at(time: DateTime<Tz>) -> Signal {
    Signal {
        strategy_id: "fire",
        instrument_token: 7,
        strategy_version: 1,
        params_hash: "h".into(),
        signal_bar_time: time,
        entry_low: d("100"),
        entry_high: d("101"),
        stop: d("96"),
        targets: vec![d("130")],
        risk_per_share: d("5"),
        reference_rr: d("5.8"),
        exit_plan: ExitPlan {
            time_stop_bars: 3,
            rule_text: "t".into(),
        },
        facts: vec![],
        valid_until: time + Duration::days(4),
    }
}

impl Strategy for Fire {
    fn meta(&self) -> &StrategyMeta {
        &self.1
    }
    fn on_bar(&mut self, view: &DataView<'_>, _: &StrategyContext<'_>) -> Option<Signal> {
        let i = view.len() - 1;
        self.0
            .contains(&i)
            .then(|| signal_at(view.last().unwrap().close_time()))
    }
    fn manage(
        &mut self,
        _: &DataView<'_>,
        _: &StrategyContext<'_>,
        _: &OpenPosition,
    ) -> Result<ExitAction, bazaar_strategies::ExitError> {
        Ok(ExitAction::Hold)
    }
}

struct Regimes(Vec<RegimeState>);

impl RegimeProvider for Regimes {
    fn context<'a>(
        &'a self,
        view: &DataView<'_>,
        instrument: &'a Instrument,
        portfolio: &'a PortfolioView,
        calendar: &'a MarketCalendar,
    ) -> Option<StrategyContext<'a>> {
        let r = self.0.get(view.len() - 1)?;
        Some(StrategyContext {
            instrument,
            market_regime: r,
            stock_regime: r,
            portfolio,
            calendar,
        })
    }
}

/// Run through the real gate; returns the output and the policy for its record.
fn go<'a>(
    rig: &'a Rig,
    spec: &'a RunSpec<'a>,
    at: &[usize],
    env: GateEnv<'a>,
) -> (RunOutput, GatePolicy<'a>) {
    let regimes = Regimes(
        rig.bars
            .iter()
            .map(|b| RegimeState::unknown(b.close_time(), Fact::new("t", "n/a", "fixture")))
            .collect(),
    );
    let mut policy = GatePolicy::new(spec, &rig.gate, env).unwrap();
    let out = run(spec, &mut fire(at), &regimes, &mut policy).unwrap();
    (out, policy)
}

#[test]
fn a_clean_entry_passes_the_real_gate_and_trades() {
    let r = rig(&[FLAT, FLAT, FLAT, FLAT, FLAT, FLAT]);
    let spec = r.spec();
    let (out, p) = go(&r, &spec, &[1], env(good_evidence(), &quiet));
    assert_eq!(p.approved(), 1);
    assert_eq!(p.rejections(), &[]);
    assert_eq!(out.trades.len(), 1);
    assert_eq!(out.skipped, 0);
}

#[test]
fn unknown_evidence_is_rejected_and_recorded_not_hidden() {
    let r = rig(&[FLAT, FLAT, FLAT, FLAT]);
    let spec = r.spec();
    let mut ev = good_evidence();
    ev.oos_trades = Tri::Unknown;
    let (out, p) = go(&r, &spec, &[1], env(ev, &quiet));
    assert_eq!(out.trades.len(), 0);
    assert_eq!(out.skipped, 1);
    assert_eq!(p.approved(), 0);
    let rej = p.rejections();
    assert_eq!(rej.len(), 1);
    assert_eq!(rej[0].rules, vec![RuleId::G01]);
    assert_eq!(rej[0].at, r.bars[1].close_time());
    assert!(rej[0].reasons[0].starts_with("G01:"));
}

#[test]
fn weak_evidence_fails_the_evidence_bar() {
    let r = rig(&[FLAT, FLAT, FLAT, FLAT]);
    let spec = r.spec();
    let mut ev = good_evidence();
    ev.oos_trades = Tri::Known(5);
    let (_, p) = go(&r, &spec, &[1], env(ev, &quiet));
    assert_eq!(p.rejections()[0].rules, vec![RuleId::G01]);
}

#[test]
fn the_selection_only_sentinel_evidence_passes_g01_and_every_other_rule_still_applies() {
    use crate::walkforward::{Window, window_env};
    let r = rig(&[FLAT, FLAT, FLAT, FLAT, FLAT, FLAT]);
    let spec = r.spec();
    let sel = |events| window_env(Window::InSample, None, "IT".into(), events).unwrap();
    // The sentinel maxima clear G01 in the real gate: the trade goes through.
    let (out, p) = go(&r, &spec, &[1], sel(&quiet));
    assert_eq!((p.approved(), out.trades.len()), (1, 1));
    assert_eq!(p.rejections(), &[]);
    // Another rule still bites under the same evidence (G07 on a results day).
    let (out, p) = go(&r, &spec, &[1], sel(&results_day));
    assert_eq!(out.trades.len(), 0);
    assert_eq!(p.rejections()[0].rules, vec![RuleId::G07]);
}

#[test]
fn an_event_day_blocks_the_entry() {
    let r = rig(&[FLAT, FLAT, FLAT, FLAT]);
    let spec = r.spec();
    let (out, p) = go(&r, &spec, &[1], env(good_evidence(), &results_day));
    assert_eq!(out.trades.len(), 0);
    assert_eq!(p.rejections()[0].rules, vec![RuleId::G07]);
}

#[test]
fn a_close_above_the_entry_zone_is_refused() {
    // Decision close 103 is above the 101 zone top (tolerance 0 bps): no chasing.
    let r = rig(&[FLAT, ["100", "103", "99.5", "103"], FLAT, FLAT]);
    let spec = r.spec();
    let (out, p) = go(&r, &spec, &[1], env(good_evidence(), &quiet));
    assert_eq!(out.trades.len(), 0);
    assert_eq!(p.approved(), 0);
    assert_eq!(p.rejections().len(), 1);
    // G10 (entry zone: no chasing) is the rule that refused the chase.
    assert!(
        p.rejections()[0].rules.contains(&RuleId::G10),
        "{:?}",
        p.rejections()[0].rules
    );
}

#[test]
fn a_losing_exit_locks_the_symbol_for_the_next_signal() {
    // Entry at bar 2, stopped out on bar 3; a second signal at the close of bar 3 meets the
    // snapshot's losing exit (and the loss streak) built from the closed trade.
    let r = rig(&[FLAT, FLAT, FLAT, DIP, FLAT, FLAT]);
    let spec = r.spec();
    let (out, p) = go(&r, &spec, &[1, 3], env(good_evidence(), &quiet));
    assert_eq!(out.trades.len(), 1);
    assert!(out.trades[0].net_pnl < Decimal::ZERO);
    assert_eq!(out.skipped, 1);
    let rej = p.rejections();
    assert_eq!(rej.len(), 1);
    assert_eq!(rej[0].at, r.bars[3].close_time());
    assert!(rej[0].rules.contains(&RuleId::G17), "{:?}", rej[0].rules);
}

#[test]
fn the_run_limits_must_be_the_gate_configs() {
    let r = rig(&[FLAT, FLAT]);
    let mut spec = r.spec();
    let mut other = r.limits.clone();
    other.min_charges_slippage_buffer += Decimal::ONE;
    spec.limits = &other;
    let err = GatePolicy::new(&spec, &r.gate, env(good_evidence(), &quiet)).unwrap_err();
    assert!(matches!(err, BacktestError::Invalid(_)));
}

fn sized() -> Sized {
    Sized {
        qty: 100,
        lots: 100,
        risk_at_stop: d("500"),
        budget: d("1000"),
        notional: d("10100"),
        capped_by_position_cap: false,
        capped_by_order_cap: false,
        reduced_by_override: false,
    }
}

#[test]
fn a_decision_bar_outside_the_run_is_a_recorded_rejection() {
    let r = rig(&[FLAT, FLAT]);
    let spec = r.spec();
    let mut p = GatePolicy::new(&spec, &r.gate, env(good_evidence(), &quiet)).unwrap();
    let stray = r.bars[0].close_time() + Duration::hours(1);
    let v = p.decide(&signal_at(stray), &sized(), d("100000"));
    assert!(matches!(v, EntryVerdict::Reject(_)));
    assert_eq!(p.approved(), 0);
    assert_eq!(p.rejections()[0].rules.len(), 0);
    assert_eq!(p.rejections()[0].at, stray);
}

fn closed(symbol: &str, entry: &str, exit: &str, net: &str) -> Closed {
    let t = |day: &str| {
        chrono_tz::Asia::Kolkata
            .with_ymd_and_hms(at(day).year(), at(day).month(), at(day).day(), 15, 30, 0)
            .unwrap()
    };
    Closed {
        symbol: symbol.into(),
        entry_day: at(entry),
        exit_day: at(exit),
        exit_at: t(exit),
        net: d(net),
    }
}

#[test]
fn the_snapshot_is_built_from_closed_trades() {
    let r = rig(&[FLAT, FLAT]);
    let spec = r.spec();
    let mut p = GatePolicy::new(&spec, &r.gate, env(good_evidence(), &quiet)).unwrap();
    // Wed 2026-11-25 is "today"; the week starts Mon 2026-11-23.
    p.closed = vec![
        closed("ABC", "2026-11-20", "2026-11-20", "-50"),
        closed("ABC", "2026-11-23", "2026-11-23", "200"),
        closed("ABC", "2026-11-25", "2026-11-25", "-30"),
        closed("XYZ", "2026-11-25", "2026-11-25", "-20"),
    ];
    let now = chrono_tz::Asia::Kolkata
        .with_ymd_and_hms(2026, 11, 25, 15, 30, 0)
        .unwrap();
    let s = p.snapshot(now, d("100100"));
    assert_eq!(s.week_start, Tri::Known(at("2026-11-23")));
    assert_eq!(s.entries_today, Tri::Known(2));
    assert_eq!(s.orders_today, Tri::Known(4));
    assert_eq!(s.consecutive_losses, Tri::Known(2));
    assert_eq!(s.day_pnl, Tri::Known(d("-50")));
    assert_eq!(s.week_pnl, Tri::Known(d("150")));
    assert_eq!(s.realised_pnl, Tri::Known(d("100")));
    assert_eq!(s.equity_mtm, Tri::Known(d("100100")));
    assert_eq!(s.starting_cash, Tri::Known(d("100000")));
    assert_eq!(s.kill_switch, Tri::Known(false));
    assert_eq!(s.positions, Tri::Known(vec![]));
    let last = s.last_losing_exits.known().unwrap();
    assert_eq!(last.len(), 2);
    assert_eq!(last[0].symbol, "ABC");
    assert_eq!(last[0].at, p.closed[2].exit_at);
}

#[test]
fn a_win_resets_the_loss_streak_and_arithmetic_overflow_is_unknown() {
    let r = rig(&[FLAT, FLAT]);
    let spec = r.spec();
    let mut p = GatePolicy::new(&spec, &r.gate, env(good_evidence(), &quiet)).unwrap();
    p.closed = vec![
        closed("ABC", "2026-11-25", "2026-11-25", "-10"),
        closed("ABC", "2026-11-25", "2026-11-25", "5"),
    ];
    let now = chrono_tz::Asia::Kolkata
        .with_ymd_and_hms(2026, 11, 25, 15, 30, 0)
        .unwrap();
    assert_eq!(p.snapshot(now, d("1")).consecutive_losses, Tri::Known(0));
    let max = Decimal::MAX.to_string();
    p.closed = vec![
        closed("ABC", "2026-11-25", "2026-11-25", &max),
        closed("ABC", "2026-11-25", "2026-11-25", &max),
    ];
    let s = p.snapshot(now, d("1"));
    assert_eq!(s.day_pnl, Tri::Unknown);
    assert_eq!(s.realised_pnl, Tri::Unknown);
}

#[test]
fn the_gate_cost_is_the_engines_round_trip_estimate_for_buys_only() {
    let r = rig(&[FLAT, FLAT]);
    let spec = r.spec();
    let stop = snap(d("0.05"), d("96")).unwrap();
    let cost = GateCost {
        spec: &spec,
        fp: fill_params(&spec),
        stop,
        token: 7,
        date: at("2026-11-24"),
    };
    let fp = fill_params(&spec);
    let entry = snap(d("0.05"), d("101")).unwrap();
    let want = round_trip_cost(&spec, &fp, (entry, stop), 100, (7, at("2026-11-24"))).unwrap();
    assert_eq!(cost.cost(Side::Buy, 100, d("101")), Tri::Known(want));
    assert!(want >= r.limits.min_charges_slippage_buffer);
    assert_eq!(cost.cost(Side::Sell, 100, d("101")), Tri::Unknown);
}

#[test]
fn every_refusal_path_rejects_and_is_recorded() {
    let r = rig(&[FLAT, FLAT]);
    let spec = r.spec();
    let now = r.bars[0].close_time();
    let weak: [fn(&mut EvidenceInputs); 4] = [
        |e| e.oos_trades = Tri::Known(5),
        |e| e.profit_factor = Tri::Known(d("0.5")),
        |e| e.expectancy_r = Tri::Known(d("-1")),
        |e| e.walk_forward_pass = Tri::Known(false),
    ];
    for tweak in weak {
        let mut ev = good_evidence();
        tweak(&mut ev);
        let mut p = GatePolicy::new(&spec, &r.gate, env(ev, &quiet)).unwrap();
        let v = p.decide(&signal_at(now), &sized(), d("100000"));
        assert!(matches!(v, EntryVerdict::Reject(_)));
        assert_eq!(p.approved(), 0);
        assert_eq!(p.rejections()[0].rules, vec![RuleId::G01]);
    }
}

#[test]
fn a_margin_shortfall_is_refused() {
    // Notional 10100 against equity 5000: the order cannot be funded.
    let r = rig(&[FLAT, FLAT]);
    let spec = r.spec();
    let mut p = GatePolicy::new(&spec, &r.gate, env(good_evidence(), &quiet)).unwrap();
    let v = p.decide(&signal_at(r.bars[0].close_time()), &sized(), d("5000"));
    assert!(matches!(v, EntryVerdict::Reject(_)));
    assert_eq!(p.approved(), 0);
    // X03 (cash or margin) is the rule that names the shortfall.
    assert!(
        p.rejections()[0].rules.contains(&RuleId::X03),
        "{:?}",
        p.rejections()[0].rules
    );
}

#[test]
fn the_loss_streak_counts_only_the_session_day_g08() {
    // Two losses yesterday and one today: the streak resets at the session end, so it is 1.
    let r = rig(&[FLAT, FLAT]);
    let spec = r.spec();
    let mut p = GatePolicy::new(&spec, &r.gate, env(good_evidence(), &quiet)).unwrap();
    p.closed = vec![
        closed("ABC", "2026-11-24", "2026-11-24", "-10"),
        closed("ABC", "2026-11-24", "2026-11-24", "-10"),
        closed("ABC", "2026-11-25", "2026-11-25", "-10"),
    ];
    let now = chrono_tz::Asia::Kolkata
        .with_ymd_and_hms(2026, 11, 25, 15, 30, 0)
        .unwrap();
    assert_eq!(p.snapshot(now, d("1")).consecutive_losses, Tri::Known(1));
}

#[test]
fn a_non_cash_equity_instrument_is_refused() {
    let mut r = rig(&[FLAT, FLAT]);
    r.inst.instrument_type = InstrumentType::parse("FUT");
    let spec = r.spec();
    let err = GatePolicy::new(&spec, &r.gate, env(good_evidence(), &quiet)).unwrap_err();
    assert!(matches!(err, BacktestError::Invalid(_)));
}

#[test]
fn a_reused_policy_starts_over_when_the_ledger_is_shorter() {
    let r = rig(&[FLAT, FLAT, FLAT, DIP, FLAT, FLAT]);
    let spec = r.spec();
    let (out, mut p) = go(&r, &spec, &[1], env(good_evidence(), &quiet));
    p.observe(&out.trades);
    assert_eq!(p.closed.len(), 1);
    p.observe(&[]);
    assert_eq!(p.closed.len(), 0);
    assert_eq!(p.approved(), 0);
    assert_eq!(p.rejections(), &[]);
}
