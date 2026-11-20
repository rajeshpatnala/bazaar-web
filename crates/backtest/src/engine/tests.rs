use std::cell::RefCell;
use std::path::Path;
use std::str::FromStr;

use crate::config::BacktestConfig;
use crate::cost::CostModel;
use crate::types::{BacktestError, Product};
use bazaar_core::calendar::MarketCalendar;
use bazaar_core::candle::Candle;
use bazaar_core::candle::Interval;
use bazaar_core::config::{self, CalendarConfig, ChargeExchange, ChargesConfig, parse_str};
use bazaar_core::dataview::DataView;
use bazaar_core::instrument::Instrument;
use bazaar_core::instrument::{Exchange, InstrumentType};
use bazaar_core::segment::InstrumentKind;
use bazaar_core::segment::Segment;
use bazaar_regime::{Fact, RegimeState};
use bazaar_risk::config::{Baseline, GateConfig};
use bazaar_risk::sizing::Sized;
use bazaar_strategies::PortfolioView;
use bazaar_strategies::{
    ExitAction, ExitPlan, OpenPosition, Signal, Strategy, StrategyContext, StrategyMeta,
};
use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::*;
use crate::provenance::ChargesEvidence;
use crate::types::{FillKind, HistoryPolicy, Leg};

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn config_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config")
}

fn row(seg: &str, side: &str, brokerage: &str, stt: &str) -> String {
    format!(
        "[[rate_set]]\nexchange = \"NSE\"\nsegment = \"{seg}\"\nside = \"{side}\"\n\
         effective_from = \"2026-01-01\"\nverified_on = \"2026-01-01\"\nsource = \"test\"\n\
         turnover_basis = \"notional\"\nbrokerage = {brokerage}\nstt_ctt_pct = \"{stt}\"\n\
         exchange_txn_pct = \"0\"\nsebi_per_crore = \"0\"\nipft_pct = \"0\"\nstamp_pct = \"0\"\n\n"
    )
}

/// Test-owned rates: delivery STT 0.1%, intraday flat 20; DP 15.34 incl. GST.
fn charges() -> ChargesConfig {
    let zero = "{ kind = \"zero\" }";
    let flat = "{ kind = \"flat\", flat = \"20\" }";
    let mut t = String::from(
        "schema_version = 1\nverified_on = \"2026-06-01\"\nsource = \"test\"\n\n\
         [meta]\nbroker = \"t\"\ncurrency = \"INR\"\nmax_age_days = 90\n\
         history_policy = \"effective_dated\"\n\n\
         [gst]\nrate_pct = \"18\"\nverified_on = \"2026-06-01\"\nsource = \"test\"\n\n\
         [dp]\nper_scrip_total = \"15.34\"\nincludes_gst = true\nverified_on = \"2026-06-01\"\n\
         source = \"test\"\n\n",
    );
    t += &row("equity_delivery", "buy", zero, "0.1");
    t += &row("equity_delivery", "sell", zero, "0.1");
    t += &row("equity_intraday", "buy", flat, "0");
    t += &row("equity_intraday", "sell", flat, "0.025");
    parse_str(&t).unwrap()
}

/// Daily bars on consecutive weekdays from 2026-03-09 (no holiday in range), closing at the
/// calendar's equity close. Rows are `(open, high, low, close)`.
fn bars(cal: &MarketCalendar, rows: &[[&str; 4]]) -> Vec<Candle> {
    bars_from(cal, NaiveDate::from_ymd_opt(2026, 3, 9).unwrap(), rows)
}

fn bars_from(cal: &MarketCalendar, start: NaiveDate, rows: &[[&str; 4]]) -> Vec<Candle> {
    let mut day = start;
    let mut out = Vec::new();
    for r in rows {
        let close = cal
            .close_time(day, Segment::Equity, InstrumentKind::CashNonFno)
            .unwrap()
            .with_timezone(&chrono_tz::Asia::Kolkata);
        out.push(Candle::new(close, Interval::Day, r.map(d), 1000, true).unwrap());
        day = cal.next_trading_day(day, Segment::Equity).unwrap();
    }
    out
}

fn instrument() -> Instrument {
    Instrument {
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
    }
}

/// Fires one signal when the decision bar is index `at`; records every view length it sees.
struct Fake {
    meta: StrategyMeta,
    at: usize,
    seen: RefCell<Vec<usize>>,
    exit_on: Option<usize>,
    stop: &'static str,
    target: &'static str,
    time_stop: u32,
    valid_days: i64,
    /// `(decision bar index, new stop)`
    tighten: Option<(usize, &'static str)>,
}

impl Strategy for Fake {
    fn meta(&self) -> &StrategyMeta {
        &self.meta
    }

    fn on_bar(&mut self, view: &DataView<'_>, ctx: &StrategyContext<'_>) -> Option<Signal> {
        self.seen.borrow_mut().push(view.len());
        ctx.check_closed_bar(view).ok()?;
        if view.len() != self.at + 1 {
            return None;
        }
        let at = view.last()?.close_time();
        Some(Signal {
            strategy_id: "fake",
            instrument_token: 7,
            strategy_version: 1,
            params_hash: "h".into(),
            signal_bar_time: at,
            entry_low: d("100"),
            entry_high: d("101"),
            stop: d(self.stop),
            targets: vec![d(self.target)],
            risk_per_share: d("5"),
            reference_rr: d("5.8"),
            exit_plan: ExitPlan {
                time_stop_bars: self.time_stop,
                rule_text: "t".into(),
            },
            facts: vec![],
            valid_until: at + chrono::Duration::days(self.valid_days),
        })
    }

    fn manage(
        &mut self,
        view: &DataView<'_>,
        _ctx: &StrategyContext<'_>,
        _position: &OpenPosition,
    ) -> Result<ExitAction, bazaar_strategies::ExitError> {
        self.seen.borrow_mut().push(view.len());
        if self.exit_on == Some(view.len() - 1) {
            return Ok(ExitAction::Exit("rule".into()));
        }
        if let Some((_, s)) = self.tighten.filter(|(i, _)| *i == view.len() - 1) {
            return Ok(ExitAction::TightenStop(d(s)));
        }
        Ok(ExitAction::Hold)
    }
}

fn fake(at: usize, exit_on: Option<usize>) -> Fake {
    Fake {
        meta: StrategyMeta {
            id: "fake",
            name: "Fake",
            version: 1,
            min_history_bars: 1,
            regime_fit: bazaar_regime::RegimeFit {
                market_trends: vec![],
                stock_cells: vec![],
                weak_cells: vec![],
            },
            penalty_weight: 0.5,
        },
        at,
        seen: RefCell::new(Vec::new()),
        exit_on,
        stop: "96",
        target: "130",
        time_stop: 50,
        valid_days: 1,
        tighten: None,
    }
}

/// One regime per bar, as of that bar's close.
struct PerBar(Vec<RegimeState>);

impl RegimeProvider for PerBar {
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

/// A provider with faults: `None` from bar `none_from` on, or a regime from the bar after.
struct Odd {
    inner: PerBar,
    none_from: usize,
    future: bool,
}

impl RegimeProvider for Odd {
    fn context<'a>(
        &'a self,
        view: &DataView<'_>,
        instrument: &'a Instrument,
        portfolio: &'a PortfolioView,
        calendar: &'a MarketCalendar,
    ) -> Option<StrategyContext<'a>> {
        let i = view.len() - 1;
        if i >= self.none_from {
            return None;
        }
        let at = if self.future { i + 1 } else { i };
        let r = self.inner.0.get(at)?;
        Some(StrategyContext {
            instrument,
            market_regime: r,
            stock_regime: r,
            portfolio,
            calendar,
        })
    }
}

/// Test-only policy (the crate ships none).
struct Fixed(EntryVerdict);

impl EntryPolicy for Fixed {
    fn decide(&mut self, _: &Signal, _: &Sized, _: Decimal) -> EntryVerdict {
        self.0.clone()
    }
}

fn regimes(bars: &[Candle]) -> PerBar {
    PerBar(
        bars.iter()
            .map(|b| RegimeState::unknown(b.close_time(), Fact::new("t", "n/a", "fixture")))
            .collect(),
    )
}

struct Env {
    cal: MarketCalendar,
    inst: Instrument,
    cfg: BacktestConfig,
    charges: ChargesConfig,
    limits: EffectiveLimits,
    capital: Decimal,
    start: NaiveDate,
    interval: Interval,
    odd: (usize, bool),
}

fn env() -> Env {
    let dir = config_dir();
    let risk = std::fs::read_to_string(dir.join("risk.toml")).unwrap();
    Env {
        cal: MarketCalendar::from_config(&config::load::<CalendarConfig>(&dir).unwrap()),
        inst: instrument(),
        cfg: BacktestConfig::load(&dir).unwrap(),
        charges: charges(),
        limits: GateConfig::from_toml_str(&risk)
            .unwrap()
            .effective(&Baseline::none()),
        capital: d("100000"),
        start: NaiveDate::from_ymd_opt(2026, 3, 9).unwrap(),
        interval: Interval::Day,
        odd: (usize::MAX, false),
    }
}

fn go(
    e: &Env,
    rows: &[[&str; 4]],
    product: Product,
    mult: &str,
    verdict: EntryVerdict,
    strat: &mut Fake,
) -> Result<RunOutput, BacktestError> {
    let bars: Vec<Candle> = bars_from(&e.cal, e.start, rows)
        .into_iter()
        .map(|b| {
            let r = [b.open(), b.high(), b.low(), b.close()];
            Candle::new(b.close_time(), e.interval, r, 1000, true).unwrap()
        })
        .collect();
    let spec = RunSpec {
        bars: &bars,
        instrument: &e.inst,
        calendar: &e.cal,
        config: &e.cfg,
        cost: CostModel::new(&e.charges, HistoryPolicy::Strict, ChargeExchange::Nse),
        limits: &e.limits,
        product,
        capital: e.capital,
        slippage_multiplier: d(mult),
    };
    let reg = Odd {
        inner: regimes(&bars),
        none_from: e.odd.0,
        future: e.odd.1,
    };
    run(&spec, strat, &reg, &mut Fixed(verdict))
}

const FLAT: [&str; 4] = ["100", "101", "99", "100"];

#[test]
fn delivery_round_trip_pays_dp_once_with_real_charges_and_adverse_slippage() {
    let e = env();
    let rows = [
        FLAT,
        FLAT,
        ["100", "103", "99.5", "102"],
        ["102", "106", "101", "105"],
    ];
    let mut s = fake(1, None);
    let out = go(
        &e,
        &rows,
        Product::Delivery,
        "1",
        EntryVerdict::Approve,
        &mut s,
    )
    .unwrap();
    let t = out.trades.first().unwrap();
    assert_eq!(out.trades.len(), 1);
    assert_eq!(t.charges.dp, d("15.34"));
    // QUAN-84-6: both legs were priced by the cost model (never a default-charges Fill).
    assert!(t.entry.charges.total().unwrap() > Decimal::ZERO);
    assert!(t.exit.charges.total().unwrap() > Decimal::ZERO);
    assert_eq!(t.exit.kind, FillKind::TimeExit);
    // Entry fills at the next open (100) plus adverse slippage, never the signal-bar close.
    assert!(t.entry.leg.price.value() > d("100"));
    assert_eq!(t.net_pnl, t.gross_pnl - t.charges.total().unwrap());
    assert_eq!(out.skipped, 0);
    // The strategy only ever saw views ending at a closed bar, one more bar each call.
    let seen = s.seen.borrow();
    assert!(seen.windows(2).all(|w| w[0] <= w[1]));
    assert_eq!(seen.last(), Some(&3));
    assert_eq!(out.equity.len(), 4);
}

#[test]
fn slippage_multiplier_below_one_is_refused_and_two_costs_more() {
    let e = env();
    let rows = [
        FLAT,
        FLAT,
        ["100", "103", "99.5", "102"],
        ["102", "106", "101", "105"],
    ];
    let bad = go(
        &e,
        &rows,
        Product::Delivery,
        "0.5",
        EntryVerdict::Approve,
        &mut fake(1, None),
    );
    assert!(matches!(bad, Err(BacktestError::Invalid(_))));
    let one = go(
        &e,
        &rows,
        Product::Delivery,
        "1",
        EntryVerdict::Approve,
        &mut fake(1, None),
    )
    .unwrap();
    let two = go(
        &e,
        &rows,
        Product::Delivery,
        "2",
        EntryVerdict::Approve,
        &mut fake(1, None),
    )
    .unwrap();
    let net = |o: &RunOutput| o.trades.first().unwrap().net_pnl;
    assert!(net(&two) < net(&one));
}

#[test]
fn a_rejected_signal_never_trades() {
    let e = env();
    let rows = [FLAT, FLAT, FLAT, FLAT];
    let out = go(
        &e,
        &rows,
        Product::Delivery,
        "1",
        EntryVerdict::Reject("no".into()),
        &mut fake(1, None),
    )
    .unwrap();
    assert_eq!(out.trades.len(), 0);
    assert_eq!(out.skipped, 1);
}

#[test]
fn an_open_gapped_past_the_zone_is_not_chased() {
    let e = env();
    let rows = [FLAT, FLAT, ["105", "106", "104", "105"], FLAT];
    let out = go(
        &e,
        &rows,
        Product::Delivery,
        "1",
        EntryVerdict::Approve,
        &mut fake(1, None),
    )
    .unwrap();
    assert_eq!(out.trades.len(), 0);
    assert_eq!(out.skipped, 1);
}

#[test]
fn a_gap_through_the_stop_fills_at_the_open_not_the_stop() {
    let e = env();
    let rows = [
        FLAT,
        FLAT,
        ["100", "101", "99.5", "100"],
        ["90", "91", "89", "90"],
        FLAT,
    ];
    let out = go(
        &e,
        &rows,
        Product::Delivery,
        "1",
        EntryVerdict::Approve,
        &mut fake(1, None),
    )
    .unwrap();
    let t = out.trades.first().unwrap();
    assert_eq!(t.exit.kind, FillKind::Stop);
    assert_eq!(t.exit.reference_price.value(), d("90"));
    assert!(t.exit.leg.price.value() <= d("90"));
}

#[test]
fn a_strategy_exit_fills_at_the_next_open() {
    let e = env();
    let rows = [FLAT, FLAT, FLAT, ["102", "103", "101", "102"], FLAT];
    let out = go(
        &e,
        &rows,
        Product::Delivery,
        "1",
        EntryVerdict::Approve,
        &mut fake(1, Some(2)),
    )
    .unwrap();
    let t = out.trades.first().unwrap();
    assert_eq!(t.exit.kind, FillKind::TimeExit);
    assert_eq!(t.exit.reference_price.value(), d("102"));
}

#[test]
fn intraday_squares_off_at_the_close_of_the_entry_bar() {
    let e = env();
    let rows = [FLAT, FLAT, ["100", "103", "99.5", "102"], FLAT];
    let out = go(
        &e,
        &rows,
        Product::Intraday,
        "1",
        EntryVerdict::Approve,
        &mut fake(1, None),
    )
    .unwrap();
    let t = out.trades.first().unwrap();
    assert_eq!(t.entry.leg.trade_date, t.exit.leg.trade_date);
    assert_eq!(t.exit.reference_price.value(), d("102"));
    assert_eq!(t.charges.dp, Decimal::ZERO);
}

#[test]
fn unsorted_bars_are_refused() {
    let e = env();
    let mut all = bars(&e.cal, &[FLAT, FLAT]);
    all.reverse();
    let spec = RunSpec {
        bars: &all,
        instrument: &e.inst,
        calendar: &e.cal,
        config: &e.cfg,
        cost: CostModel::new(&e.charges, HistoryPolicy::Strict, ChargeExchange::Nse),
        limits: &e.limits,
        product: Product::Delivery,
        capital: d("100000"),
        slippage_multiplier: d("1"),
    };
    let r = run(
        &spec,
        &mut fake(1, None),
        &regimes(&all),
        &mut Fixed(EntryVerdict::Approve),
    );
    assert!(matches!(r, Err(BacktestError::Invalid(_))));
}

fn approve(e: &Env, rows: &[[&str; 4]], s: &mut Fake) -> Result<RunOutput, BacktestError> {
    go(e, rows, Product::Delivery, "1", EntryVerdict::Approve, s)
}

/// Records how many closed trades the engine showed it at each decision bar.
struct Recorder(Vec<usize>, Option<chrono::DateTime<chrono_tz::Tz>>);

impl EntryPolicy for Recorder {
    fn observe(&mut self, closed: &[Trade]) {
        self.0.push(closed.len());
        self.1 = closed.iter().map(|t| t.exit.bar_time).max();
    }
    fn decide(&mut self, s: &Signal, _: &Sized, _: Decimal) -> EntryVerdict {
        // No look-ahead: every trade shown at this decision closed at or before its bar close.
        assert!(self.1.is_none_or(|t| t <= s.signal_bar_time));
        EntryVerdict::Approve
    }
}

#[test]
fn the_policy_observes_closed_trades_at_every_decision_bar() {
    let e = env();
    let rows = [FLAT, FLAT, FLAT, FLAT, FLAT, FLAT];
    let bars = bars_from(&e.cal, e.start, &rows);
    let spec = RunSpec {
        bars: &bars,
        instrument: &e.inst,
        calendar: &e.cal,
        config: &e.cfg,
        cost: CostModel::new(&e.charges, HistoryPolicy::Strict, ChargeExchange::Nse),
        limits: &e.limits,
        product: Product::Delivery,
        capital: e.capital,
        slippage_multiplier: d("1"),
    };
    let reg = regimes(&bars);
    let mut rec = Recorder(Vec::new(), None);
    let out = run(&spec, &mut fake(1, None), &reg, &mut rec).unwrap();
    assert_ne!(rec.0.len(), 0);
    assert!(rec.0.windows(2).all(|w| w[0] <= w[1]));
    assert_eq!(rec.0.first(), Some(&0));
    assert_eq!(out.trades.len(), 1);
    assert!(rec.0.iter().all(|n| *n <= 1));
}

#[test]
fn the_end_of_data_exit_is_in_the_final_equity_point() {
    let e = env();
    let rows = [FLAT, FLAT, FLAT, ["100", "104", "99", "103"]];
    let out = approve(&e, &rows, &mut fake(1, None)).unwrap();
    let t = out.trades.first().unwrap();
    assert_eq!(t.exit.kind, FillKind::TimeExit);
    assert_eq!(t.exit.reference_price.value(), d("103"));
    // QUAN-88-4: flat book at the end, so the last point is capital plus realised net P&L.
    let last = out.equity.last().unwrap();
    assert_eq!(last.equity, e.capital + t.net_pnl);
}

#[test]
fn a_time_stop_exits_at_the_next_open() {
    let e = env();
    let rows = [FLAT, FLAT, FLAT, FLAT, ["103", "104", "102", "103"], FLAT];
    let mut s = fake(1, None);
    s.time_stop = 2;
    let out = approve(&e, &rows, &mut s).unwrap();
    let t = out.trades.first().unwrap();
    assert_eq!(t.exit.kind, FillKind::TimeExit);
    assert_eq!(t.exit.reference_price.value(), d("103"));
}

#[test]
fn a_time_stop_still_fires_when_the_regime_is_missing() {
    let mut e = env();
    e.odd = (3, false);
    let rows = [FLAT, FLAT, FLAT, FLAT, ["103", "104", "102", "103"], FLAT];
    let mut s = fake(1, None);
    s.time_stop = 2;
    let out = approve(&e, &rows, &mut s).unwrap();
    // QUAN-88-8: no regime from bar 3 on, yet the position still leaves at bar 4's open.
    let t = out.trades.first().unwrap();
    assert_eq!(t.exit.reference_price.value(), d("103"));
}

#[test]
fn no_regime_means_no_signal_and_no_error() {
    let mut e = env();
    e.odd = (0, false);
    let out = approve(&e, &[FLAT, FLAT, FLAT, FLAT], &mut fake(1, None)).unwrap();
    assert_eq!(out.trades.len(), 0);
    assert_eq!(out.skipped, 0);
    assert_eq!(out.equity.len(), 4);
}

#[test]
fn a_regime_from_the_future_is_a_hard_error() {
    let mut e = env();
    e.odd = (usize::MAX, true);
    let r = approve(&e, &[FLAT, FLAT, FLAT, FLAT], &mut fake(1, None));
    assert!(matches!(r, Err(BacktestError::Invalid(_))));
}

#[test]
fn a_tightened_stop_is_the_one_that_fills() {
    let e = env();
    let rows = [FLAT, FLAT, FLAT, ["100", "100.5", "98.5", "99.5"], FLAT];
    let mut s = fake(1, None);
    s.tighten = Some((2, "99"));
    let out = approve(&e, &rows, &mut s).unwrap();
    let t = out.trades.first().unwrap();
    assert_eq!(t.exit.kind, FillKind::Stop);
    assert_eq!(t.exit.reference_price.value(), d("99"));
}

#[test]
fn a_target_fills_at_the_target() {
    let e = env();
    let rows = [FLAT, FLAT, FLAT, ["105", "131", "104", "130"], FLAT];
    let out = approve(&e, &rows, &mut fake(1, None)).unwrap();
    let t = out.trades.first().unwrap();
    assert_eq!(t.exit.kind, FillKind::Target);
    assert_eq!(t.exit.reference_price.value(), d("130"));
}

#[test]
fn a_stop_on_the_entry_bar_fills_that_bar() {
    let e = env();
    let rows = [FLAT, FLAT, ["100", "101", "95", "97"], FLAT];
    let out = approve(&e, &rows, &mut fake(1, None)).unwrap();
    let t = out.trades.first().unwrap();
    assert_eq!(t.exit.kind, FillKind::Stop);
    assert_eq!(t.entry.leg.trade_date, t.exit.leg.trade_date);
    assert_eq!(t.exit.reference_price.value(), d("96"));
}

#[test]
fn a_sizing_refusal_is_counted_as_skipped() {
    let mut e = env();
    e.capital = d("100");
    let out = approve(&e, &[FLAT, FLAT, FLAT, FLAT], &mut fake(1, None)).unwrap();
    assert_eq!(out.trades.len(), 0);
    assert_eq!(out.skipped, 1);
}

#[test]
fn a_stale_signal_is_skipped_not_filled() {
    let e = env();
    // The signal bar is a Friday; the next bar closes Monday, past the one-day validity.
    let rows = [FLAT; 7];
    let out = approve(&e, &rows, &mut fake(4, None)).unwrap();
    assert_eq!(out.trades.len(), 0);
    assert_eq!(out.skipped, 1);
}

#[test]
fn a_missing_rate_set_is_an_error_not_a_skipped_signal() {
    let mut e = env();
    // Real rates take effect 2026-11-20; the bars are in March, strict history.
    e.charges = config::load::<ChargesConfig>(&config_dir()).unwrap();
    let r = approve(&e, &[FLAT, FLAT, FLAT, FLAT], &mut fake(1, None));
    assert!(matches!(r, Err(BacktestError::NoRateSet(_))));
}

#[test]
fn a_run_against_the_real_charges_file_prices_both_legs() {
    let mut e = env();
    e.charges = config::load::<ChargesConfig>(&config_dir()).unwrap();
    e.start = NaiveDate::from_ymd_opt(2026, 11, 23).unwrap();
    let rows = [FLAT, FLAT, FLAT, ["100", "104", "99", "103"]];
    let out = approve(&e, &rows, &mut fake(1, None)).unwrap();
    let t = out.trades.first().unwrap();
    assert!(t.charges.total().unwrap() > Decimal::ZERO);
    assert_eq!(t.net_pnl, t.gross_pnl - t.charges.total().unwrap());
}

#[test]
fn intraday_on_sub_daily_bars_is_refused() {
    let mut e = env();
    e.interval = Interval::Min15;
    let r = go(
        &e,
        &[FLAT, FLAT, FLAT, FLAT],
        Product::Intraday,
        "1",
        EntryVerdict::Approve,
        &mut fake(1, None),
    );
    assert!(matches!(r, Err(BacktestError::Invalid(_))));
}

#[test]
fn a_non_cash_equity_instrument_is_refused() {
    let mut e = env();
    e.inst.exchange = Exchange::parse("NFO");
    e.inst.instrument_type = InstrumentType::parse("FUT");
    let r = approve(&e, &[FLAT, FLAT, FLAT, FLAT], &mut fake(1, None));
    assert!(matches!(r, Err(BacktestError::Invalid(_))));
}

fn stressed(e: &Env, rows: &[[&str; 4]], at: usize) -> Result<StressedRun, BacktestError> {
    let bars = bars_from(&e.cal, e.start, rows);
    let spec = RunSpec {
        bars: &bars,
        instrument: &e.inst,
        calendar: &e.cal,
        config: &e.cfg,
        cost: CostModel::new(&e.charges, HistoryPolicy::LatestForAll, ChargeExchange::Nse),
        limits: &e.limits,
        product: Product::Delivery,
        capital: e.capital,
        // Ignored: the stress runner owns both multipliers.
        slippage_multiplier: d("5"),
    };
    let mut make = || RunParts {
        strategy: Box::new(fake(at, None)),
        regimes: Box::new(regimes(&bars)),
        policy: Box::new(Fixed(EntryVerdict::Approve)),
    };
    run_with_stress(&spec, &mut make)
}

#[test]
fn the_stress_run_costs_more_and_the_evidence_covers_every_leg_of_both_runs() {
    let e = env();
    let rows = [
        FLAT,
        FLAT,
        ["100", "103", "99.5", "102"],
        ["102", "106", "101", "105"],
    ];
    let r = stressed(&e, &rows, 1).unwrap();
    assert_eq!((r.base.trades.len(), r.stress.trades.len()), (1, 1));
    // Adverse slippage at 2x entry price is higher than at 1x (the multiplier of 5 was ignored).
    assert!(r.stress.trades[0].entry.leg.price.value() > r.base.trades[0].entry.leg.price.value());
    assert!(r.stress_pnl_change().unwrap() < Decimal::ZERO);
    // QUAN-94-8: the evidence is the one `from_legs` gives for ALL legs, under the model's policy.
    let cost = CostModel::new(&e.charges, HistoryPolicy::LatestForAll, ChargeExchange::Nse);
    let legs: Vec<&Leg> = r
        .base
        .trades
        .iter()
        .chain(&r.stress.trades)
        .flat_map(|t| [&t.entry.leg, &t.exit.leg])
        .collect();
    assert_eq!(legs.len(), 4);
    let expect = ChargesEvidence::from_legs(&cost, legs).unwrap();
    assert_eq!(r.charges, expect);
    assert_ne!(r.charges, ChargesEvidence::from_legs(&cost, []).unwrap());
    assert_eq!(r.charges.policy(), HistoryPolicy::LatestForAll);
}

#[test]
fn a_stress_run_with_no_trades_levies_no_charges() {
    let e = env();
    // The strategy fires on a bar index that never exists, so nothing trades.
    let r = stressed(&e, &[FLAT, FLAT, FLAT], 99).unwrap();
    assert!(r.base.trades.is_empty() && r.stress.trades.is_empty());
    assert_eq!(
        r.charges,
        ChargesEvidence::from_legs(
            &CostModel::new(&e.charges, HistoryPolicy::LatestForAll, ChargeExchange::Nse),
            []
        )
        .unwrap()
    );
    assert_eq!(r.stress_pnl_change().unwrap(), Decimal::ZERO);
}
