//! No look-ahead (F9 AC 4, QUAN-88-9): results up to bar `k` do not depend on bars after `k`; a
//! strategy that peeks at the next close still gets only the next bar's open plus adverse
//! slippage (the engine gives it no earlier or better fill); views end at the closed bar; a peeking
//! regime provider is rejected. Also TS-88-1: stop and target in one bar, the stop fills. A fixed
//! test keeps the flat-series-with-one-high case that once failed the proptest.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use bazaar_backtest::engine::{EntryPolicy, EntryVerdict, RegimeProvider, RunOutput, RunSpec, run};
use bazaar_backtest::{BacktestConfig, BacktestError, CostModel, FillKind, HistoryPolicy, Product};
use bazaar_core::calendar::MarketCalendar;
use bazaar_core::candle::{Candle, Interval};
use bazaar_core::config::{self, CalendarConfig, ChargeExchange, ChargesConfig};
use bazaar_core::dataview::DataView;
use bazaar_core::instrument::{Exchange, Instrument, InstrumentType};
use bazaar_core::segment::{InstrumentKind, Segment};
use bazaar_regime::{Fact, RegimeFit, RegimeState};
use bazaar_risk::config::{Baseline, EffectiveLimits, GateConfig};
use bazaar_risk::sizing::Sized;
use bazaar_strategies::{
    ExitAction, ExitPlan, OpenPosition, PortfolioView, Signal, Strategy, StrategyContext,
    StrategyMeta,
};
use chrono::{DateTime, NaiveDate};
use chrono_tz::Tz;
use proptest::prelude::{ProptestConfig, any, prop, prop_assert, prop_assert_eq, proptest};
use proptest::strategy::Strategy as PropStrategy;
use rust_decimal::Decimal;

type Row = [Decimal; 4];

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config")
}

/// Rows from integer tick counts (tick 0.05): `open`, `down` below it, `up` above it, and the
/// close at `f / 10` of the low-to-high range.
fn row(open: i64, down: i64, up: i64, f: i64) -> Row {
    let (lo, hi) = (open - down, open + up);
    let close = lo + (hi - lo) * f / 10;
    [open, lo, hi, close].map(|t| Decimal::new(t * 5, 2))
}

fn candles(cal: &MarketCalendar, rows: &[Row]) -> Vec<Candle> {
    let mut day = NaiveDate::from_ymd_opt(2026, 11, 23).unwrap();
    let mut out = Vec::new();
    for r in rows {
        let close = cal
            .close_time(day, Segment::Equity, InstrumentKind::CashNonFno)
            .unwrap()
            .with_timezone(&chrono_tz::Asia::Kolkata);
        // Row order is (open, low, high, close); Candle wants (open, high, low, close).
        out.push(Candle::new(close, Interval::Day, [r[0], r[2], r[1], r[3]], 1000, true).unwrap());
        day = cal.next_trading_day(day, Segment::Equity).unwrap();
    }
    out
}

#[derive(Clone, Copy)]
enum Mode {
    /// Fires when the last close is above the one before (past data only).
    PastUp,
    /// The double: fires when the NEXT bar closes higher, read from the full series.
    Peek,
    /// Fires once at this decision index.
    At(usize),
}

/// One observation per `on_bar` call: `(view length, last close time)`.
type Seen = Vec<(usize, DateTime<Tz>)>;

struct Probe<'a> {
    meta: StrategyMeta,
    full: &'a [Candle],
    mode: Mode,
    seen: RefCell<Seen>,
    signals: RefCell<Vec<DateTime<Tz>>>,
}

fn probe(full: &[Candle], mode: Mode) -> Probe<'_> {
    Probe {
        meta: StrategyMeta {
            id: "probe",
            name: "Probe",
            version: 1,
            min_history_bars: 1,
            regime_fit: RegimeFit {
                market_trends: vec![],
                stock_cells: vec![],
                weak_cells: vec![],
            },
            penalty_weight: 0.5,
        },
        full,
        mode,
        seen: RefCell::new(Vec::new()),
        signals: RefCell::new(Vec::new()),
    }
}

impl Strategy for Probe<'_> {
    fn meta(&self) -> &StrategyMeta {
        &self.meta
    }

    fn on_bar(&mut self, view: &DataView<'_>, ctx: &StrategyContext<'_>) -> Option<Signal> {
        let i = view.len() - 1;
        let last = view.last()?;
        // The view is the full series cut at the closed bar: nothing after it is reachable.
        assert_eq!(view.bars().len(), i + 1);
        assert_eq!(last.close_time(), self.full[i].close_time());
        assert_eq!(view.lookback(usize::MAX).len(), i + 1);
        ctx.check_closed_bar(view)
            .expect("the engine already vets the context");
        self.seen.borrow_mut().push((view.len(), last.close_time()));
        let fire = match self.mode {
            Mode::PastUp => i > 0 && last.close() > self.full[i - 1].close(),
            Mode::Peek => self
                .full
                .get(i + 1)
                .is_some_and(|n| n.close() > last.close()),
            Mode::At(at) => i == at,
        };
        if !fire {
            return None;
        }
        let at = last.close_time();
        self.signals.borrow_mut().push(at);
        // Wide levels so the generated bars (about 99-105) are always inside the zone and never
        // reach the stop or target; `At` keeps tight levels for the stop/target tie tests.
        let (low, high, stop, target, risk, rr) = match self.mode {
            Mode::At(_) => ("100", "101", "96", "130", "5", "5.8"),
            _ => ("98", "110", "90", "200", "20", "4.5"),
        };
        Some(Signal {
            strategy_id: "probe",
            instrument_token: 7,
            strategy_version: 1,
            params_hash: "h".into(),
            signal_bar_time: at,
            entry_low: d(low),
            entry_high: d(high),
            stop: d(stop),
            targets: vec![d(target)],
            risk_per_share: d(risk),
            reference_rr: d(rr),
            exit_plan: ExitPlan {
                time_stop_bars: 3,
                rule_text: "t".into(),
            },
            facts: vec![],
            valid_until: at + chrono::Duration::days(4),
        })
    }

    fn manage(
        &mut self,
        _view: &DataView<'_>,
        _ctx: &StrategyContext<'_>,
        _position: &OpenPosition,
    ) -> Result<ExitAction, bazaar_strategies::ExitError> {
        Ok(ExitAction::Hold)
    }
}

/// One `unknown` regime per bar; `future` hands out the next bar's regime (a peeking provider).
struct Regimes {
    states: Vec<RegimeState>,
    future: bool,
}

impl RegimeProvider for Regimes {
    fn context<'a>(
        &'a self,
        view: &DataView<'_>,
        instrument: &'a Instrument,
        portfolio: &'a PortfolioView,
        calendar: &'a MarketCalendar,
    ) -> Option<StrategyContext<'a>> {
        let r = self.states.get(view.len() - 1 + usize::from(self.future))?;
        Some(StrategyContext {
            instrument,
            market_regime: r,
            stock_regime: r,
            portfolio,
            calendar,
        })
    }
}

struct ApproveAll;

impl EntryPolicy for ApproveAll {
    fn decide(&mut self, _: &Signal, _: &Sized, _: Decimal) -> EntryVerdict {
        EntryVerdict::Approve
    }
}

struct Env {
    cal: MarketCalendar,
    inst: Instrument,
    cfg: BacktestConfig,
    charges: ChargesConfig,
    limits: EffectiveLimits,
}

fn env() -> Env {
    let risk = std::fs::read_to_string(dir().join("risk.toml")).unwrap();
    Env {
        cal: MarketCalendar::from_config(&config::load::<CalendarConfig>(&dir()).unwrap()),
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
        limits: GateConfig::from_toml_str(&risk)
            .unwrap()
            .effective(&Baseline::none()),
    }
}

fn exec(
    e: &Env,
    bars: &[Candle],
    mode: Mode,
    future_regime: bool,
) -> (Result<RunOutput, BacktestError>, Seen, Vec<DateTime<Tz>>) {
    let spec = RunSpec {
        bars,
        instrument: &e.inst,
        calendar: &e.cal,
        config: &e.cfg,
        cost: CostModel::new(&e.charges, HistoryPolicy::Strict, ChargeExchange::Nse),
        limits: &e.limits,
        product: Product::Delivery,
        capital: d("100000"),
        slippage_multiplier: d("1"),
    };
    let regimes = Regimes {
        states: bars
            .iter()
            .map(|b| RegimeState::unknown(b.close_time(), Fact::new("t", "n/a", "fixture")))
            .collect(),
        future: future_regime,
    };
    let mut s = probe(bars, mode);
    let out = run(&spec, &mut s, &regimes, &mut ApproveAll);
    let seen = s.seen.borrow().clone();
    let signals = s.signals.borrow().clone();
    (out, seen, signals)
}

fn series() -> impl PropStrategy<Value = Vec<Row>> {
    prop::collection::vec(
        (1980i64..2040, 0i64..60, 0i64..60, 0i64..=10).prop_map(|(o, dn, up, f)| row(o, dn, up, f)),
        8..20,
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Replacing every bar after `k` changes nothing observable up to `k`.
    #[test]
    fn lookahead_future_bars_do_not_change_the_past(
        head in series(),
        tail in series(),
        cut in 2usize..6,
    ) {
        let e = env();
        let k = cut.min(head.len() - 3);
        let mut other = head[..=k].to_vec();
        other.extend(tail.iter().take(head.len() - k - 1).copied());
        other.resize(head.len(), head[head.len() - 1]);
        let (a, b) = (candles(&e.cal, &head), candles(&e.cal, &other));
        let (ra, sa, _) = exec(&e, &a, Mode::PastUp, false);
        let (rb, sb, _) = exec(&e, &b, Mode::PastUp, false);
        let (ra, rb) = (ra.unwrap(), rb.unwrap());
        let tk = a[k].close_time();
        let upto = |s: &Seen| s.iter().filter(|(n, _)| *n <= k + 1).copied().collect::<Seen>();
        prop_assert_eq!(upto(&sa), upto(&sb));
        let done = |o: &RunOutput| {
            o.trades.iter().filter(|t| t.exit.bar_time <= tk).cloned().collect::<Vec<_>>()
        };
        prop_assert_eq!(done(&ra), done(&rb));
        let curve = |o: &RunOutput| {
            o.equity.iter().filter(|p| p.time <= tk).cloned().collect::<Vec<_>>()
        };
        prop_assert_eq!(curve(&ra), curve(&rb));
    }

    /// Even a strategy that peeks at the next close fills at the next open plus adverse
    /// slippage, never at the close that produced the signal, and sees only closed-bar views.
    #[test]
    fn lookahead_entries_fill_at_the_next_open_plus_slippage(rows in series(), peek in any::<bool>()) {
        let e = env();
        let bars = candles(&e.cal, &rows);
        let mode = if peek { Mode::Peek } else { Mode::PastUp };
        let (out, seen, signals) = exec(&e, &bars, mode, false);
        let out = out.unwrap();
        prop_assert!(seen.iter().all(|(n, t)| *t == bars[*n - 1].close_time()));
        prop_assert!(seen.windows(2).all(|w| w[0].0 < w[1].0));
        for t in &out.trades {
            let idx = bars.iter().position(|b| b.close_time() == t.entry.bar_time).unwrap();
            prop_assert!(idx >= 1);
            prop_assert!(signals.contains(&bars[idx - 1].close_time()));
            prop_assert_eq!(t.entry.kind, FillKind::Entry);
            prop_assert_eq!(t.entry.reference_price.value(), bars[idx].open());
            // Adverse, non-zero and tick-aligned: the fill is exactly open plus the slippage.
            let slip = t.entry.slippage_per_unit;
            prop_assert!(slip > Decimal::ZERO);
            prop_assert_eq!(slip % e.inst.tick_size, Decimal::ZERO);
            prop_assert_eq!(t.entry.leg.price.value(), bars[idx].open() + slip);
        }
    }
}

#[test]
fn a_regime_provider_that_peeks_is_rejected() {
    let e = env();
    let rows = [row(2000, 5, 5, 5); 6];
    let bars = candles(&e.cal, &rows);
    let (out, _, _) = exec(&e, &bars, Mode::PastUp, true);
    assert!(matches!(out, Err(BacktestError::Invalid(_))));
}

#[test]
fn stop_and_target_in_one_bar_the_stop_fills() {
    // TS-88-1: the entry bar trades through both the stop (96) and the target (130).
    let e = env();
    let flat = row(2000, 20, 20, 5);
    let wide = [2000, 1900, 2700, 2000].map(|t| Decimal::new(t * 5, 2));
    let bars = candles(&e.cal, &[flat, flat, wide, flat, flat]);
    let (out, _, _) = exec(&e, &bars, Mode::At(1), false);
    let out = out.unwrap();
    assert_eq!(out.trades.len(), 1);
    let t = out.trades.first().unwrap();
    assert_eq!(t.exit.kind, FillKind::Stop);
    assert_eq!(t.exit.reference_price.value(), d("96"));
}

#[test]
fn stop_and_target_in_a_later_bar_the_stop_fills() {
    // QUAN-89-2: entry on bar 2 (stays inside 99-101), then bar 3 trades through both levels.
    let e = env();
    let flat = row(2000, 20, 20, 5);
    let wide = [2000, 1900, 2700, 2000].map(|t| Decimal::new(t * 5, 2));
    let bars = candles(&e.cal, &[flat, flat, flat, wide, flat]);
    let (out, _, _) = exec(&e, &bars, Mode::At(1), false);
    let out = out.unwrap();
    assert_eq!(out.trades.len(), 1);
    let t = out.trades.first().unwrap();
    assert_eq!(t.entry.reference_price.value(), d("100"));
    assert_eq!(t.exit.kind, FillKind::Stop);
    assert_eq!(t.exit.reference_price.value(), d("96"));
}

#[test]
fn a_rising_series_enters_and_the_fill_is_open_plus_slippage() {
    // CR-89-1: the proptest loops over trades, so prove here that entries do happen.
    let e = env();
    let rows = [
        row(2000, 5, 5, 3),
        row(2000, 5, 5, 8),
        row(2000, 5, 5, 5),
        row(2000, 5, 5, 5),
        row(2000, 5, 5, 5),
        row(2000, 5, 5, 5),
    ];
    let bars = candles(&e.cal, &rows);
    let (out, _, signals) = exec(&e, &bars, Mode::PastUp, false);
    let out = out.unwrap();
    assert_ne!(signals.len(), 0);
    let t = out.trades.first().unwrap();
    assert_eq!(t.entry.reference_price.value(), d("100"));
    assert!(t.entry.slippage_per_unit > Decimal::ZERO);
    assert_eq!(
        t.entry.leg.price.value(),
        d("100") + t.entry.slippage_per_unit
    );
}

#[test]
fn regression_flat_series_with_one_high_runs_clean() {
    // Shrunk proptest failure (8 flat rows at 99.00, row 2 with a 99.10 high): it exposed a
    // strategy that assumed one view per bar while a held position skips `on_bar`.
    let e = env();
    let mut rows = [row(1980, 0, 0, 0); 8];
    rows[2] = row(1980, 0, 2, 0);
    let bars = candles(&e.cal, &rows);
    for mode in [Mode::PastUp, Mode::Peek] {
        let (out, _, _) = exec(&e, &bars, mode, false);
        assert!(out.is_ok());
    }
}
