//! Golden tests (F9 AC 1, 6, 7) on a deterministic synthetic series (a rising sawtooth with some
//! falling cycles; the pinned numbers are snapshots of the engine, not derived by hand): the engine's charges follow the
//! calculator's rounding rules (AC 1); the same run twice and on four threads is identical and
//! net of charges, with provenance (AC 6); walk-forward folds never overlap, OOS runs only the set
//! chosen in IS, and the trial count and thin-fold flag appear (AC 7).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use bazaar_backtest::engine::{EntryPolicy, EntryVerdict, RegimeProvider, RunOutput, RunSpec, run};
use bazaar_backtest::metrics::Metrics;
use bazaar_backtest::provenance::{ChargesEvidence, ProvenanceInputs, RunKind, build};
use bazaar_backtest::walkforward::{ParamSet, WfParams, Window, split, walk_forward_gated};
use bazaar_backtest::{BacktestConfig, CostModel, EvidenceLabel, HistoryPolicy, Product};
use bazaar_core::calendar::MarketCalendar;
use bazaar_core::candle::{Candle, Interval};
use bazaar_core::config::{self, CalendarConfig, ChargeExchange, ChargesConfig};
use bazaar_core::dataview::DataView;
use bazaar_core::instrument::{Exchange, Instrument, InstrumentType};
use bazaar_regime::{Fact, RegimeFit, RegimeState};
use bazaar_risk::config::{Baseline, EffectiveLimits, GateConfig};
use bazaar_risk::rules::inputs::EventFlags;
use bazaar_risk::sizing::Sized;
use bazaar_risk::types::Tri;
use bazaar_strategies::{
    ExitAction, ExitPlan, OpenPosition, PortfolioView, Signal, Strategy, StrategyContext,
    StrategyMeta,
};
use chrono::NaiveDate;
use rust_decimal::Decimal;

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config")
}

/// Close in ticks (0.05): a sawtooth of period 5 (+2.00 a bar) on a slow rise; some cycles fall.
fn close_ticks(i: usize) -> i64 {
    let (phase, cycle) = (i64::try_from(i % 5).unwrap(), i64::try_from(i / 5).unwrap());
    // Every seventh cycle falls instead of rises, so there are losing trades (a profit factor).
    let up = if cycle % 7 == 3 { 4 - phase } else { phase };
    2000 + 40 * up + cycle
}

/// Weekday daily bars closing 15:30 IST from 2026-11-23 (the shipped holiday calendar ends with
/// 2026, so the series is built from weekdays; the engine only passes the calendar through).
fn candles(n: usize) -> Vec<Candle> {
    use chrono::{Datelike, TimeZone, Weekday};
    let mut day = NaiveDate::from_ymd_opt(2026, 11, 23).unwrap();
    let mut out = Vec::new();
    for i in 0..n {
        let close = chrono_tz::Asia::Kolkata
            .from_local_datetime(&day.and_hms_opt(15, 30, 0).unwrap())
            .unwrap();
        let (now, prev) = (close_ticks(i), close_ticks(i.saturating_sub(1)));
        let px = |ticks: i64| Decimal::new(ticks * 5, 2);
        let (high, low) = (prev.max(now) + 2, prev.min(now) - 2);
        let k = [px(prev), px(high), px(low), px(now)];
        out.push(Candle::new(close, Interval::Day, k, 1000, true).unwrap());
        day = day.succ_opt().unwrap();
        while matches!(day.weekday(), Weekday::Sat | Weekday::Sun) {
            day = day.succ_opt().unwrap();
        }
    }
    out
}

/// Fires at every `every`-th bar, with levels relative to the signal close.
struct Every {
    meta: StrategyMeta,
    every: usize,
}

impl Strategy for Every {
    fn meta(&self) -> &StrategyMeta {
        &self.meta
    }

    fn on_bar(&mut self, view: &DataView<'_>, _: &StrategyContext<'_>) -> Option<Signal> {
        let i = view.len() - 1;
        if !i.is_multiple_of(self.every) {
            return None;
        }
        let at = view.last()?.close_time();
        let c = view.last()?.close();
        let tick = |x: Decimal| (x / d("0.05")).round() * d("0.05");
        let target = tick(c * d("2"));
        let (high, stop) = (tick(c * d("1.1")), tick(c * d("0.8")));
        Some(Signal {
            strategy_id: "every",
            instrument_token: 7,
            strategy_version: 1,
            params_hash: "h".into(),
            signal_bar_time: at,
            entry_low: tick(c * d("0.9")),
            entry_high: high,
            stop,
            targets: vec![target],
            risk_per_share: high - stop,
            reference_rr: (target - high) / (high - stop),
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
        _: &DataView<'_>,
        _: &StrategyContext<'_>,
        _: &OpenPosition,
    ) -> Result<ExitAction, bazaar_strategies::ExitError> {
        Ok(ExitAction::Hold)
    }
}

fn every(n: usize) -> Every {
    Every {
        meta: StrategyMeta {
            id: "every",
            name: "Every",
            version: 1,
            min_history_bars: 1,
            regime_fit: RegimeFit {
                market_trends: vec![],
                stock_cells: vec![],
                weak_cells: vec![],
            },
            penalty_weight: 0.5,
        },
        every: n,
    }
}

struct Unknown(Vec<RegimeState>);

impl RegimeProvider for Unknown {
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

fn regimes(bars: &[Candle]) -> Unknown {
    Unknown(
        bars.iter()
            .map(|b| RegimeState::unknown(b.close_time(), Fact::new("t", "n/a", "fixture")))
            .collect(),
    )
}

fn model(e: &Env) -> CostModel<'_> {
    CostModel::new(&e.charges, HistoryPolicy::Strict, ChargeExchange::Nse)
}

fn spec<'a>(e: &'a Env, bars: &'a [Candle], warmup: usize) -> RunSpec<'a> {
    RunSpec {
        bars,
        instrument: &e.inst,
        calendar: &e.cal,
        config: &e.cfg,
        cost: model(e),
        limits: &e.limits,
        product: Product::Delivery,
        capital: d("100000"),
        slippage_multiplier: d("1"),
        warmup,
    }
}

fn exec(e: &Env, bars: &[Candle], every_n: usize) -> RunOutput {
    let s = spec(e, bars, 0);
    run(&s, &mut every(every_n), &regimes(bars), &mut ApproveAll).unwrap()
}

#[test]
fn ac6_the_run_is_identical_twice_and_on_four_threads() {
    let e = env();
    let bars = candles(120);
    let a = exec(&e, &bars, 5);
    assert_eq!(a.trades.len(), 24);
    assert_eq!(
        a.trades.iter().map(|t| t.net_pnl).sum::<Decimal>(),
        d("2720.06")
    );
    let b = exec(&e, &bars, 5);
    assert_eq!(format!("{a:?}"), format!("{b:?}"));
    let parallel: Vec<RunOutput> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..4).map(|_| s.spawn(|| exec(&e, &bars, 5))).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    for p in &parallel {
        assert_eq!(format!("{a:?}"), format!("{p:?}"));
    }
}

#[test]
fn ac6_and_ac1_pnl_equity_and_r_are_net_of_every_charge() {
    let e = env();
    let bars = candles(120);
    let out = exec(&e, &bars, 5);
    let cost = model(&e);
    let whole = |x: Decimal| x == x.round();
    let mut net = Decimal::ZERO;
    let mut exit_dp = Decimal::ZERO;
    let mut exit_days = std::collections::BTreeSet::new();
    for t in &out.trades {
        let fills = [&t.entry, &t.exit];
        let charges: Decimal = fills.iter().map(|f| f.charges.total().unwrap()).sum();
        for f in fills {
            // AC 1 rounding: STT and stamp whole rupees, the other lines 2 dp. Every line but DP
            // equals the cost model on the same leg: a consistency check on the engine path (the
            // independent calculator check is `cost_samples.rs`). DP is checked below.
            assert!(whole(f.charges.stt) && whole(f.charges.stamp));
            for x in [
                f.charges.brokerage,
                f.charges.exchange_txn,
                f.charges.gst,
                f.charges.sebi,
            ] {
                assert_eq!(x, x.round_dp(2));
            }
            let want = cost.charges_excl_dp(&f.leg).unwrap();
            let lines = |c: &bazaar_backtest::ChargeBreakdown| {
                (c.brokerage, c.exchange_txn, c.sebi, c.gst, c.stt, c.stamp)
            };
            assert_eq!(lines(&f.charges), lines(&want));
        }
        // DP: never on the buy, charged on the delivery sell.
        assert_eq!(t.entry.charges.dp, Decimal::ZERO);
        assert!(t.exit.charges.dp > Decimal::ZERO);
        exit_dp += t.exit.charges.dp;
        exit_days.insert(t.exit.bar_time.date_naive());
        let qty = Decimal::from(t.entry.leg.qty.0);
        let gross = (t.exit.leg.price.value() - t.entry.leg.price.value()) * qty;
        assert_eq!(t.net_pnl, gross - charges, "net is gross less charges");
        assert!(charges > Decimal::ZERO);
        net += t.net_pnl;
    }
    // Once per scrip per IST day (one scrip here): the sum is days x the configured value.
    let per_day = e.charges.dp.as_ref().unwrap().per_scrip_total;
    assert_eq!(exit_dp, per_day * Decimal::from(exit_days.len()));
    let m = Metrics::from_trades(&out.trades, d("100000"), &out.equity).unwrap();
    assert_eq!(m.net_pnl, net);
    assert_eq!(out.equity.last().unwrap().equity, d("100000") + net);
}

#[test]
fn ac6_provenance_carries_hashes_charges_range_data_range_and_trials() {
    let e = env();
    let bars = candles(120);
    let out = exec(&e, &bars, 5);
    let legs = out.trades.iter().flat_map(|t| [&t.entry.leg, &t.exit.leg]);
    let used = ChargesEvidence::from_legs(&model(&e), legs).unwrap();
    let files = std::fs::read(dir().join("backtest.toml")).unwrap();
    let sets = [("every".to_owned(), "n=5".to_owned())];
    let from = bars[0].close_time().date_naive();
    let to = bars.last().unwrap().close_time().date_naive();
    let p = build(&ProvenanceInputs {
        config_files: &[("backtest.toml", &files)],
        charges: &e.charges,
        data_from: from,
        data_to: to,
        universe: &["ABC".to_owned()],
        parameter_sets: &sets,
        trials: &e.cfg.trials,
        kind: RunKind::Plain,
        charges_used: used,
    })
    .unwrap();
    assert_eq!(p.config_hashes["backtest.toml"].len(), 64);
    assert!(!p.no_charges_levied && !p.costs_approximate);
    assert!(p.charges_verified_from.unwrap() <= p.charges_verified_to.unwrap());
    assert_eq!(p.data_from, Some(from));
    assert_eq!(p.data_to, Some(to));
    assert_eq!(p.trials, e.cfg.trials.total_trials(1));
    assert_eq!(p.gate_evidence, None);
}

fn quiet(_: NaiveDate) -> EventFlags {
    EventFlags {
        results_window: Tri::Known(false),
        policy_day: Tri::Known(false),
        option_on_own_expiry: Tri::Known(false),
    }
}

/// `(window, candidate, first bar, end bar, net P&L of the run)`.
type Call = (Window, usize, usize, usize, Decimal);

#[test]
fn ac7_folds_do_not_overlap_oos_uses_the_is_choice_and_trials_are_counted() {
    let e = env();
    let bars = candles(1500);
    let sets = vec![
        ParamSet {
            id: "every".into(),
            params: "n=5".into(),
        },
        ParamSet {
            id: "every".into(),
            params: "n=3".into(),
        },
    ];
    let p = WfParams {
        capital: d("100000"),
        sets: &sets,
        cost: model(&e),
    };
    let folds = split(&bars, &e.cfg).unwrap();
    assert!(folds.len() >= 2, "got {} folds", folds.len());
    for f in &folds {
        assert_eq!(f.is.end, f.oos.start, "IS ends where OOS starts");
    }
    for w in folds.windows(2) {
        assert!(w[0].oos.end <= w[1].oos.start, "OOS windows never overlap");
    }
    let calls: RefCell<Vec<Call>> = RefCell::new(Vec::new());
    let ns = [5usize, 3];
    let (res, charges) = walk_forward_gated(&e.cfg, &bars, &p, "IT", &quiet, &mut |w, c, b, _| {
        let at = b[0].close_time();
        let first = bars.iter().position(|x| x.close_time() == at).unwrap();
        let s = spec(&e, b.with_history(), b.warmup());
        let out = run(
            &s,
            &mut every(ns[c]),
            &regimes(b.with_history()),
            &mut ApproveAll,
        )?;
        let net = out.trades.iter().map(|t| t.net_pnl).sum();
        calls.borrow_mut().push((w, c, first, first + b.len(), net));
        Ok(out)
    })
    .unwrap();
    let calls = calls.into_inner();
    assert_eq!(res.trials, e.cfg.trials.total_trials(2));
    assert_eq!(res.param_sets, sets);
    assert_eq!(res.folds.len(), folds.len());
    let mut oos_seen = 0;
    let mut thin = Vec::new();
    for (f, fold) in res.folds.iter().zip(&folds) {
        let ran_in = |w: Window, start: usize| -> Vec<&Call> {
            calls.iter().filter(|c| c.0 == w && c.2 == start).collect()
        };
        let is = ran_in(Window::InSample, fold.is.start);
        assert_eq!(is.len(), 2, "every candidate runs in IS");
        assert!(
            is.iter().all(|c| c.3 <= fold.oos.start),
            "IS stops before OOS"
        );
        let oos = ran_in(Window::OutOfSample, fold.oos.start);
        assert_eq!(oos.len(), usize::from(f.oos.is_some()));
        assert_eq!(f.skipped_in_is, f.oos.is_none());
        for c in oos {
            assert_eq!(c.1, f.chosen, "OOS runs only the set chosen in IS");
            oos_seen += 1;
        }
        // The chosen set is the IS winner: its IS net P&L is the fold's and no candidate beat it
        // (ties go to the lowest index; in this fixture both sets clear the IS bar when one does).
        let chosen_net = is.iter().find(|c| c.1 == f.chosen).unwrap().4;
        assert_eq!(f.is.net_pnl, chosen_net);
        if f.oos.is_some() {
            for c in &is {
                assert!(c.4 < chosen_net || (c.4 == chosen_net && c.1 >= f.chosen));
            }
        }
        thin.push(f.oos.as_ref().map(|_| f.thin));
    }
    // Every OOS fold here has at least `thin_fold_trades` trades, so none is thin (the thin case
    // is the slice 6 unit test `pooled_oos_trades_feed_the_assessment_and_thin_folds_are_flagged`);
    // folds skipped in IS have no flag. Too few folds with an OOS run: low evidence, bar not met.
    assert_eq!(thin, [Some(false), Some(false), None, Some(false), None]);
    assert_eq!(res.assessment.label, EvidenceLabel::LowEvidence);
    assert!(!res.assessment.meets_pooled_bar);
    assert!(oos_seen >= 1, "at least one fold ran OOS");
    assert_eq!(charges.policy(), HistoryPolicy::Strict);
    // Golden values of this fixture (a change here means the engine's numbers changed).
    assert_eq!((res.folds.len(), oos_seen, res.trials), (5, 3, 2));
    assert_eq!(res.pooled.trades, 80);
    assert_eq!(res.pooled.net_pnl, d("-7725.57"));
}

fn golden_pins() -> serde_json::Value {
    let p =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/backtest/golden_baseline.json");
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn pin(v: &serde_json::Value, k: &str) -> Decimal {
    d(v[k].as_str().unwrap())
}

#[test]
fn ac6_baseline_stats_match_the_golden_fixture() {
    let e = env();
    let bars = candles(120);
    let out = exec(&e, &bars, 5);
    let m = Metrics::from_trades(&out.trades, d("100000"), &out.equity).unwrap();
    let g = golden_pins();
    let b = &g["baseline"];
    assert_eq!(u64::from(m.trades), b["trades"].as_u64().unwrap());
    assert_eq!(u64::from(m.wins), b["wins"].as_u64().unwrap());
    assert_eq!(u64::from(m.losses), b["losses"].as_u64().unwrap());
    assert_eq!(m.win_rate, pin(b, "win_rate"));
    assert_eq!(m.net_pnl, pin(b, "net_pnl"));
    assert_eq!(m.gross_profit, pin(b, "gross_profit"));
    assert_eq!(m.gross_loss, pin(b, "gross_loss"));
    assert_eq!(m.profit_factor, Some(pin(b, "profit_factor")));
    assert_eq!(m.expectancy_r, Some(pin(b, "expectancy_r")));
    assert_eq!(m.drawdown.max_abs, pin(b, "max_drawdown_abs"));
}

#[test]
fn ac6_cost_sensitivity_doubling_slippage_lowers_net_and_keeps_the_trades() {
    let e = env();
    let bars = candles(120);
    let base = exec(&e, &bars, 5);
    let mut s = spec(&e, &bars, 0);
    s.slippage_multiplier = d("2");
    let hi = run(&s, &mut every(5), &regimes(&bars), &mut ApproveAll).unwrap();
    let net = |o: &RunOutput| o.trades.iter().map(|t| t.net_pnl).sum::<Decimal>();
    let g = golden_pins();
    let want = &g["slippage_x2"];
    assert_eq!(
        hi.trades.len(),
        usize::try_from(want["trades"].as_u64().unwrap()).unwrap()
    );
    assert_eq!(net(&hi), pin(want, "net_pnl"));
    assert_eq!(hi.trades.len(), base.trades.len());
    assert!(net(&hi) < net(&base), "worse fills must cost P&L");
    // Every trade is no better under heavier slippage.
    for (a, b) in base.trades.iter().zip(&hi.trades) {
        assert!(b.net_pnl <= a.net_pnl);
    }
    // Charges are not slippage: lines other than the price-driven ones stay positive on both.
    assert!(
        hi.trades
            .iter()
            .all(|t| t.entry.charges.total().unwrap() > Decimal::ZERO)
    );
}
