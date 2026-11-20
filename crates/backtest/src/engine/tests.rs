use std::cell::RefCell;
use std::path::Path;
use std::str::FromStr;

use bazaar_core::candle::Interval;
use bazaar_core::config::{self, CalendarConfig, ChargeExchange, ChargesConfig, parse_str};
use bazaar_core::instrument::{Exchange, InstrumentType};
use bazaar_core::segment::InstrumentKind;
use bazaar_regime::{Fact, RegimeState};
use bazaar_risk::config::{Baseline, GateConfig};
use bazaar_strategies::{ExitPlan, StrategyMeta};
use chrono::NaiveDate;

use super::*;
use crate::types::{FillKind, HistoryPolicy};

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
    let mut day = NaiveDate::from_ymd_opt(2026, 3, 9).unwrap();
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
            stop: d("96"),
            targets: vec![d("130")],
            risk_per_share: d("5"),
            reference_rr: d("5.8"),
            exit_plan: ExitPlan {
                time_stop_bars: 50,
                rule_text: "t".into(),
            },
            facts: vec![],
            valid_until: at + chrono::Duration::days(1),
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
    let bars = bars(&e.cal, rows);
    let spec = RunSpec {
        bars: &bars,
        instrument: &e.inst,
        calendar: &e.cal,
        config: &e.cfg,
        cost: CostModel::new(&e.charges, HistoryPolicy::Strict, ChargeExchange::Nse),
        limits: &e.limits,
        product,
        capital: d("100000"),
        slippage_multiplier: d(mult),
    };
    run(&spec, strat, &regimes(&bars), &mut Fixed(verdict))
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
