use std::cell::RefCell;
use std::path::Path;
use std::str::FromStr;

use bazaar_core::candle::Interval;
use bazaar_core::config::{self, ChargeExchange, ChargesConfig};
use bazaar_core::money::{Price, Qty};
use bazaar_core::segment::Segment;
use chrono::{DateTime, Datelike, TimeZone, Weekday};
use chrono_tz::Asia::Kolkata;

use super::*;
use crate::types::{
    ChargeBreakdown, EvidenceLabel, Fill, FillKind, HistoryPolicy, Leg, LegInstrument, LegSide,
    Product,
};

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn day(y: i32, m: u32, dd: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, dd).unwrap()
}

/// 4 months in-sample, 2 out, step 2, no warm-up: 5 whole folds over 14 months of weekdays.
fn cfg() -> BacktestConfig {
    let mut c =
        BacktestConfig::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config")).unwrap();
    c.walk_forward.in_sample_months = 4;
    c.walk_forward.out_of_sample_months = 2;
    c.walk_forward.step_months = 2;
    c.walk_forward.warmup_bars = 0;
    c
}

fn bars(close: &str) -> Vec<Candle> {
    let mut out = Vec::new();
    let mut date = day(2024, 1, 1);
    while date <= day(2025, 2, 28) {
        if !matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
            let t = Kolkata
                .with_ymd_and_hms(date.year(), date.month(), date.day(), 15, 30, 0)
                .unwrap();
            let px = [d("100"), d("101"), d("99"), d(close)];
            out.push(Candle::new(t, Interval::Day, px, 1000, true).unwrap());
        }
        date = date.succ_opt().unwrap();
    }
    out
}

fn fill(side: LegSide, at: DateTime<chrono_tz::Tz>) -> Fill {
    let price = Price::snap_to_tick(d("100"), d("0.05")).unwrap();
    Fill {
        leg: Leg {
            instrument_token: 1,
            symbol: "S1".into(),
            segment: Segment::Equity,
            instrument: LegInstrument::Cash,
            product: Product::Delivery,
            side,
            qty: Qty(1),
            price,
            trade_date: at.date_naive(),
        },
        kind: FillKind::Entry,
        bar_time: at,
        reference_price: price,
        slippage_per_unit: Decimal::ZERO,
        charges: ChargeBreakdown::default(),
    }
}

fn trade(at: DateTime<chrono_tz::Tz>, net: &str, r: &str) -> Trade {
    Trade {
        strategy_id: "s".into(),
        entry: fill(LegSide::Buy, at),
        exit: fill(LegSide::Sell, at),
        gross_pnl: d(net),
        charges: ChargeBreakdown::default(),
        net_pnl: d(net),
        risk_inr: d("100"),
        r_multiple: Some(d(r)),
    }
}

fn out(trades: Vec<Trade>) -> RunOutput {
    RunOutput {
        trades,
        equity: Vec::new(),
        skipped: 0,
    }
}

fn charges() -> ChargesConfig {
    config::load::<ChargesConfig>(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config"))
        .unwrap()
}

fn sets(n: usize) -> Vec<ParamSet> {
    (0..n)
        .map(|i| ParamSet {
            id: "s".into(),
            params: format!("p={i}"),
        })
        .collect()
}

fn params<'a>(ch: &'a ChargesConfig, sets: &'a [ParamSet]) -> WfParams<'a> {
    WfParams {
        capital: d("100000"),
        sets,
        regimes: 2,
        cost: CostModel::new(ch, HistoryPolicy::Strict, ChargeExchange::Nse),
    }
}

/// A winner and a loser (net, R) stamped at the slice's last bar.
fn wl(b: &[Candle], win: (&str, &str), loss: (&str, &str)) -> RunOutput {
    let at = b.last().unwrap().close_time();
    out(vec![trade(at, win.0, win.1), trade(at, loss.0, loss.1)])
}

/// A winner worth `win` then a 5 loss, both stamped at the slice's last bar.
fn winner_loser(b: &[Candle], win: &str) -> RunOutput {
    let at = b.last().unwrap().close_time();
    let r = (d(win) / d("100")).to_string();
    out(vec![trade(at, win, &r), trade(at, "-5", "-0.05")])
}

#[test]
fn folds_roll_without_overlap_and_partial_windows_are_dropped() {
    let b = bars("100");
    let folds = split(&b, &cfg()).unwrap();
    assert_eq!(folds.len(), 5);
    for f in &folds {
        assert_eq!(f.is.end, f.oos.start);
        assert!(f.is.start < f.is.end && f.oos.start < f.oos.end);
    }
    for w in folds.windows(2) {
        assert!(w[0].oos.end <= w[1].oos.start, "OOS windows overlap");
        assert!(w[0].is.start < w[1].is.start);
    }
    // Fold 0: IS Jan-Apr, OOS May-Jun 2024.
    let first = &folds[0];
    assert_eq!(trade_date(b[first.is.start].close_time()), day(2024, 1, 1));
    assert_eq!(trade_date(b[first.oos.start].close_time()), day(2024, 5, 1));
    assert_eq!(trade_date(b[first.oos.end].close_time()), day(2024, 7, 1));
}

#[test]
fn warmup_bars_are_skipped_and_short_data_has_no_folds() {
    let b = bars("100");
    let mut c = cfg();
    c.walk_forward.warmup_bars = 20;
    let folds = split(&b, &c).unwrap();
    assert!(folds.len() < 5);
    assert!(folds[0].is.start >= 20);
    c.walk_forward.warmup_bars = 100_000;
    assert_eq!(split(&b, &c).unwrap(), vec![]);
}

#[test]
fn overlapping_oos_steps_are_refused() {
    let mut c = cfg();
    c.walk_forward.step_months = 1;
    assert!(matches!(
        split(&bars("100"), &c),
        Err(BacktestError::Invalid(_))
    ));
}

type Call = (Window, usize, NaiveDate, Option<EvidenceInputs>);

#[test]
fn is_runs_see_no_oos_bars_get_no_evidence_and_oos_gets_it() {
    let b = bars("100");
    let c = cfg();
    let folds = split(&b, &c).unwrap();
    let log: RefCell<Vec<Call>> = RefCell::new(Vec::new());
    let mut run = |w: Window, cand: usize, s: &[Candle], ev: Option<EvidenceInputs>| {
        let last = trade_date(s.last().unwrap().close_time());
        log.borrow_mut().push((w, cand, last, ev));
        Ok(winner_loser(s, "50"))
    };
    let (ch, ps) = (charges(), sets(1));
    let r = walk_forward(&c, &b, &params(&ch, &ps), &mut run).unwrap();
    let log = log.into_inner();
    assert_eq!(log.len(), 10);
    for (i, f) in folds.iter().enumerate() {
        let (is, oos) = (&log[2 * i], &log[2 * i + 1]);
        assert_eq!((is.0, oos.0), (Window::InSample, Window::OutOfSample));
        assert!(is.3.is_none(), "IS must not receive gate evidence");
        assert_eq!(oos.3, Some(measurement_evidence(&r.folds[i].is)));
        // The IS slice ends before the OOS slice begins.
        assert!(is.2 < trade_date(b[f.oos.start].close_time()));
    }
}

#[test]
fn oos_prices_cannot_change_is_selection_or_evidence() {
    let c = cfg();
    let run_with = |close: &str| {
        let b = bars("100");
        let folds = split(&b, &c).unwrap();
        let mut b2 = b.clone();
        // Rewrite every bar from fold 0's OOS start on.
        for x in b2.iter_mut().skip(folds[0].oos.start) {
            let px = [d("100"), d("200"), d("99"), d(close)];
            *x = Candle::new(x.close_time(), Interval::Day, px, 1000, true).unwrap();
        }
        // The result is a function of the slice's closes, so an OOS bar in IS would change it.
        let mut run = |_: Window, _: usize, s: &[Candle], _: Option<EvidenceInputs>| {
            let sum: Decimal = s.iter().map(Candle::close).sum();
            Ok(winner_loser(s, &(sum / d("1000")).round().to_string()))
        };
        let (ch, ps) = (charges(), sets(1));
        walk_forward(&c, &b2, &params(&ch, &ps), &mut run)
            .unwrap()
            .folds[0]
            .is
            .clone()
    };
    assert_eq!(run_with("100"), run_with("140"));
}

#[test]
fn the_best_in_sample_candidate_alone_runs_oos_and_trials_are_counted() {
    let b = bars("100");
    let c = cfg();
    let log: RefCell<Vec<(Window, usize)>> = RefCell::new(Vec::new());
    let mut run = |w: Window, cand: usize, s: &[Candle], _: Option<EvidenceInputs>| {
        log.borrow_mut().push((w, cand));
        // Candidate 1 earns most in IS; ties would go to the lower index.
        Ok(winner_loser(s, ["40", "60", "60"][cand]))
    };
    let (ch, ps) = (charges(), sets(3));
    let r = walk_forward(&c, &b, &params(&ch, &ps), &mut run).unwrap();
    assert_eq!(r.param_sets, ps);
    let log = log.into_inner();
    assert!(r.folds.iter().all(|f| f.chosen == 1));
    let oos: Vec<_> = log.iter().filter(|x| x.0 == Window::OutOfSample).collect();
    assert_eq!(oos.len(), 5);
    assert!(oos.iter().all(|x| x.1 == 1));
    assert_eq!(r.trials, 3);
    assert_eq!(r.gate_evidence, GATE_EVIDENCE);
}

#[test]
fn a_candidate_missing_the_is_bar_gets_no_oos_run() {
    let b = bars("100");
    let c = cfg();
    let oos_runs = RefCell::new(0_u32);
    let mut run = |w: Window, _: usize, s: &[Candle], _: Option<EvidenceInputs>| {
        if w == Window::OutOfSample {
            *oos_runs.borrow_mut() += 1;
        }
        let at = s.last().unwrap().close_time();
        Ok(out(vec![trade(at, "-10", "-0.1"), trade(at, "5", "0.05")]))
    };
    let (ch, ps) = (charges(), sets(1));
    let r = walk_forward(&c, &b, &params(&ch, &ps), &mut run).unwrap();
    assert_eq!(oos_runs.into_inner(), 0);
    // A skipped fold is flagged skipped, not thin (thin only means something with an OOS run).
    assert!(
        r.folds
            .iter()
            .all(|f| f.oos.is_none() && f.skipped_in_is && !f.thin)
    );
    assert_eq!(r.pooled.trades, 0);
    assert_eq!(r.oos_trades, vec![]);
    assert_eq!(r.assessment.label, EvidenceLabel::LowEvidence);
}

#[test]
fn pooled_oos_trades_feed_the_assessment_and_thin_folds_are_flagged() {
    let b = bars("100");
    let c = cfg();
    let mut run =
        |_: Window, _: usize, s: &[Candle], _: Option<EvidenceInputs>| Ok(winner_loser(s, "50"));
    let (ch, ps) = (charges(), sets(1));
    let r = walk_forward(&c, &b, &params(&ch, &ps), &mut run).unwrap();
    // 2 trades per OOS fold: below thin_fold_trades (5); pooled 10 trades over 5 folds.
    assert!(r.folds.iter().all(|f| f.thin && !f.skipped_in_is));
    assert_eq!(r.pooled.trades, 10);
    assert_eq!(r.oos_trades.len(), 10);
    // 10 R trades are below the 30-trade floor.
    assert_eq!(r.assessment.label, crate::types::EvidenceLabel::LowEvidence);
    assert!(!r.assessment.meets_pooled_bar);
    assert_eq!(r.trials, 1);
}

#[test]
fn measurement_evidence_is_the_is_metrics_with_the_named_flag() {
    let at = Kolkata.with_ymd_and_hms(2024, 1, 2, 15, 30, 0).unwrap();
    let m = Metrics::from_trades(
        &[trade(at, "50", "0.5"), trade(at, "-5", "-0.05")],
        d("1000"),
        &[],
    )
    .unwrap();
    let e = measurement_evidence(&m);
    assert_eq!(e.oos_trades, Tri::Known(2));
    assert_eq!(e.profit_factor, Tri::Known(d("10")));
    assert_eq!(e.expectancy_r, Tri::Known(d("0.225")));
    assert_eq!(e.walk_forward_pass, Tri::Known(true));
    // No losing trade: profit factor is undefined, so the gate would fail closed.
    let none = Metrics::from_trades(&[trade(at, "50", "0.5")], d("1000"), &[]).unwrap();
    assert_eq!(measurement_evidence(&none).profit_factor, Tri::Unknown);
}

#[test]
fn zero_candidates_and_run_errors_are_errors() {
    let b = bars("100");
    let c = cfg();
    let mut ok =
        |_: Window, _: usize, s: &[Candle], _: Option<EvidenceInputs>| Ok(winner_loser(s, "50"));
    let (ch, none, one) = (charges(), sets(0), sets(1));
    assert!(matches!(
        walk_forward(&c, &b, &params(&ch, &none), &mut ok),
        Err(BacktestError::Invalid(_))
    ));
    let mut bad = |_: Window, _: usize, _: &[Candle], _: Option<EvidenceInputs>| {
        Err(BacktestError::Overflow("test"))
    };
    assert!(matches!(
        walk_forward(&c, &b, &params(&ch, &one), &mut bad),
        Err(BacktestError::Overflow(_))
    ));
}

#[test]
fn step_zero_and_unsorted_bars_are_refused() {
    let mut c = cfg();
    c.walk_forward.step_months = 0;
    c.walk_forward.out_of_sample_months = 0;
    assert!(matches!(
        split(&bars("100"), &c),
        Err(BacktestError::Invalid(_))
    ));
    let mut b = bars("100");
    b.swap(3, 4);
    assert!(matches!(split(&b, &cfg()), Err(BacktestError::Invalid(_))));
    let dup = [b[0].clone(), b[0].clone()];
    assert!(matches!(
        split(&dup, &cfg()),
        Err(BacktestError::Invalid(_))
    ));
}

/// Runs a fixed result per window kind and reports which OOS windows ran.
fn oos_runs(c: &BacktestConfig, ps: &[ParamSet], is: (&str, &str), loss: (&str, &str)) -> u32 {
    let (b, ch) = (bars("100"), charges());
    let n = RefCell::new(0_u32);
    let mut run = |w: Window, _: usize, s: &[Candle], _: Option<EvidenceInputs>| {
        if w == Window::OutOfSample {
            *n.borrow_mut() += 1;
        }
        Ok(wl(s, is, loss))
    };
    walk_forward(c, &b, &params(&ch, ps), &mut run).unwrap();
    n.into_inner()
}

#[test]
fn the_trial_penalty_alone_blocks_a_candidate_with_many_sets() {
    // PF 1.35: above the 1.3 floor, below 1.3 + 0.1 (2..=10 trials). Expectancy 0.175 is fine.
    let (win, loss) = (("135", "1.35"), ("-100", "-1"));
    let c = cfg();
    assert_eq!(oos_runs(&c, &sets(1), win, loss), 5);
    assert_eq!(oos_runs(&c, &sets(3), win, loss), 0);
}

#[test]
fn prior_trials_count_and_raise_the_bar() {
    let (b, ch, ps) = (bars("100"), charges(), sets(1));
    let mut c = cfg();
    c.trials.prior_trials = 10;
    let n = RefCell::new(0_u32);
    let mut run = |w: Window, _: usize, s: &[Candle], _: Option<EvidenceInputs>| {
        if w == Window::OutOfSample {
            *n.borrow_mut() += 1;
        }
        Ok(wl(s, ("135", "1.35"), ("-100", "-1")))
    };
    let r = walk_forward(&c, &b, &params(&ch, &ps), &mut run).unwrap();
    // 10 prior + 1 = 11 trials: the second tier adds 0.2, so the bar is 1.5 and PF 1.35 fails.
    assert_eq!(r.trials, 11);
    assert_eq!(r.assessment.required_profit_factor, d("1.5"));
    assert_eq!(n.into_inner(), 0);
}

#[test]
fn low_expectancy_with_a_good_profit_factor_gets_no_oos_run() {
    // PF 1.3 passes the floor; mean R (0.05 - 0.01) / 2 = 0.02 is below 0.1.
    let n = oos_runs(&cfg(), &sets(1), ("130", "0.05"), ("-100", "-0.01"));
    assert_eq!(n, 0);
}

#[test]
fn ties_pick_the_lowest_index_and_selection_is_among_sets_that_clear_the_bar() {
    let (b, ch) = (bars("100"), charges());
    let c = cfg();
    let ps = sets(3);
    let mut tie =
        |_: Window, _: usize, s: &[Candle], _: Option<EvidenceInputs>| Ok(winner_loser(s, "50"));
    let r = walk_forward(&c, &b, &params(&ch, &ps), &mut tie).unwrap();
    assert!(r.folds.iter().all(|f| f.chosen == 0));
    // Set 0 earns more but misses the bar (PF 1.33 < 1.4 with 2 sets); set 1 clears it.
    let two = sets(2);
    let mut pick = |_: Window, i: usize, s: &[Candle], _: Option<EvidenceInputs>| {
        Ok(if i == 0 {
            wl(s, ("400", "4"), ("-300", "-3"))
        } else {
            wl(s, ("150", "1.5"), ("-100", "-1"))
        })
    };
    let r = walk_forward(&c, &b, &params(&ch, &two), &mut pick).unwrap();
    assert!(r.folds.iter().all(|f| f.chosen == 1 && f.oos.is_some()));
}

#[test]
fn duplicate_parameter_sets_count_once_and_distinct_sets_count_each() {
    let (b, ch) = (bars("100"), charges());
    let c = cfg();
    let mut ps = sets(2);
    ps.push(ps[0].clone());
    ps.push(ps[1].clone());
    let mut run =
        |_: Window, _: usize, s: &[Candle], _: Option<EvidenceInputs>| Ok(winner_loser(s, "50"));
    let r = walk_forward(&c, &b, &params(&ch, &ps), &mut run).unwrap();
    assert_eq!(r.trials, 2);
    assert_eq!(r.param_sets, sets(2));
    let mut other = sets(2);
    other[1].id = "t".into();
    other.push(other[0].clone());
    assert_eq!(distinct_sets(&other).len(), 2);
    assert_eq!(distinct_sets(&sets(3)).len(), 3);
}

#[test]
fn skipped_folds_do_not_count_as_folds_for_the_label() {
    let (b, ch, ps) = (bars("100"), charges(), sets(1));
    let c = cfg();
    let is_calls = RefCell::new(0_u32);
    let mut run = |w: Window, _: usize, s: &[Candle], _: Option<EvidenceInputs>| {
        let at = s.last().unwrap().close_time();
        if w == Window::InSample {
            let first = *is_calls.borrow() == 0;
            *is_calls.borrow_mut() += 1;
            return Ok(if first {
                winner_loser(s, "50")
            } else {
                out(vec![trade(at, "-10", "-0.1"), trade(at, "5", "0.05")])
            });
        }
        // 29 winners and a loser: PF and expectancy are far above the bar.
        let mut t: Vec<Trade> = (0..29).map(|_| trade(at, "50", "0.5")).collect();
        t.push(trade(at, "-5", "-0.05"));
        Ok(out(t))
    };
    let r = walk_forward(&c, &b, &params(&ch, &ps), &mut run).unwrap();
    assert_eq!(r.folds.len(), 5);
    assert_eq!(r.folds.iter().filter(|f| f.skipped_in_is).count(), 4);
    assert_eq!(r.pooled.r_trades, 30);
    // One fold of evidence is below min_folds (4): never a pass, however good the 30 trades.
    assert_eq!(r.assessment.label, EvidenceLabel::LowEvidence);
    assert!(!r.assessment.meets_pooled_bar);
}

#[test]
fn costs_approximate_comes_from_the_cost_model() {
    let (b, ch, ps) = (bars("100"), charges(), sets(1));
    let c = cfg();
    let mut run = |_: Window, _: usize, s: &[Candle], _: Option<EvidenceInputs>| {
        let at = s.last().unwrap().close_time();
        let mut t: Vec<Trade> = (0..29).map(|_| trade(at, "50", "0.5")).collect();
        t.push(trade(at, "-5", "-0.05"));
        Ok(out(t))
    };
    let strict = walk_forward(&c, &b, &params(&ch, &ps), &mut run).unwrap();
    assert!(strict.assessment.meets_pooled_bar);
    let mut p = params(&ch, &ps);
    p.cost = CostModel::new(&ch, HistoryPolicy::LatestForAll, ChargeExchange::Nse);
    let approx = walk_forward(&c, &b, &p, &mut run).unwrap();
    assert!(!approx.assessment.meets_pooled_bar);
}

fn quiet(_: NaiveDate) -> bazaar_risk::rules::inputs::EventFlags {
    bazaar_risk::rules::inputs::EventFlags {
        results_window: Tri::Known(false),
        policy_day: Tri::Known(false),
        option_on_own_expiry: Tri::Known(false),
    }
}

#[test]
fn window_env_gives_oos_exactly_the_engine_evidence_and_is_the_selection_policy() {
    let at = Kolkata.with_ymd_and_hms(2024, 1, 2, 15, 30, 0).unwrap();
    let m = Metrics::from_trades(
        &[trade(at, "50", "0.5"), trade(at, "-5", "-0.05")],
        d("1000"),
        &[],
    )
    .unwrap();
    let ev = measurement_evidence(&m);
    let oos = window_env(Window::OutOfSample, Some(ev), "IT".into(), &quiet).unwrap();
    assert_eq!(oos.evidence, ev);
    // OOS without evidence is refused: no silent permissive default.
    assert!(matches!(
        window_env(Window::OutOfSample, None, "IT".into(), &quiet),
        Err(BacktestError::Invalid(_))
    ));
    // IS ignores a passed-in record and always gets the same selection-only one.
    let a = window_env(Window::InSample, Some(ev), "IT".into(), &quiet).unwrap();
    let b = window_env(Window::InSample, None, "IT".into(), &quiet).unwrap();
    assert_eq!(a.evidence, b.evidence);
    assert_ne!(a.evidence, ev);
}

#[test]
fn the_result_vote_counts_skipped_folds_as_failures() {
    let (b, ch, ps) = (bars("100"), charges(), sets(1));
    let c = cfg();
    let mut bad = |_: Window, _: usize, s: &[Candle], _: Option<EvidenceInputs>| {
        Ok(wl(s, ("-10", "-0.1"), ("5", "0.05")))
    };
    let r = walk_forward(&c, &b, &params(&ch, &ps), &mut bad).unwrap();
    assert_eq!((r.vote.voting_folds, r.vote.skipped_folds), (5, 5));
    assert!(!r.vote.all_ok());
}

#[test]
fn the_gated_runner_gives_in_sample_runs_selection_only_evidence() {
    let (bs, ch, ps) = (bars("100"), charges(), sets(2));
    // 2024 bars predate the shipped rate sets, so the cost model uses latest-for-all.
    let mut params = params(&ch, &ps);
    params.cost = CostModel::new(&ch, HistoryPolicy::LatestForAll, ChargeExchange::Nse);
    let mut seen = Vec::new();
    let mut run = |win: Window, _: usize, slice: &[Candle], env: GateEnv<'_>| {
        seen.push((win, env.evidence.oos_trades));
        Ok(match win {
            Window::InSample => wl(slice, ("10", "0.1"), ("-5", "-0.05")),
            Window::OutOfSample => out(vec![]),
        })
    };
    let (res, _) = walk_forward_gated(&cfg(), &bs, &params, "IT", &quiet, &mut run).unwrap();
    assert_eq!(res.folds.len(), 5);
    // IS runs got the selection-only evidence, never the OOS record.
    assert!(
        seen.iter()
            .filter(|(win, _)| *win == Window::InSample)
            .all(|(_, n)| *n == Tri::Known(u32::MAX))
    );
}

/// Delivery charges with a different `verified_on` for every table.
fn dated_charges() -> ChargesConfig {
    let row = |side: &str, v: &str| {
        format!(
            "[[rate_set]]\nexchange = \"NSE\"\nsegment = \"equity_delivery\"\nside = \"{side}\"\n\
             effective_from = \"2026-01-01\"\nverified_on = \"{v}\"\nsource = \"test\"\n\
             turnover_basis = \"notional\"\nbrokerage = {{ kind = \"zero\" }}\n\
             stt_ctt_pct = \"0\"\nexchange_txn_pct = \"0\"\nsebi_per_crore = \"0\"\n\
             ipft_pct = \"0\"\nstamp_pct = \"0\"\n\n"
        )
    };
    let mut t = String::from(
        "schema_version = 1\nverified_on = \"2026-03-10\"\nsource = \"test\"\n\n\
         [meta]\nbroker = \"t\"\ncurrency = \"INR\"\nmax_age_days = 90\n\
         history_policy = \"effective_dated\"\n\n\
         [gst]\nrate_pct = \"18\"\nverified_on = \"2026-03-01\"\nsource = \"test\"\n\n\
         [dp]\nper_scrip_total = \"15.34\"\nincludes_gst = true\nverified_on = \"2026-03-15\"\n\
         source = \"test\"\n\n",
    );
    t += &row("buy", "2026-02-01");
    t += &row("sell", "2026-04-01");
    config::parse_str(&t).unwrap()
}

/// A one-trade run whose entry and exit legs both have `side`.
fn one_sided(b: &[Candle], side: LegSide) -> RunOutput {
    let at = b.last().unwrap().close_time();
    let mut t = trade(at, "10", "0.1");
    t.entry.leg.side = side;
    t.exit.leg.side = side;
    out(vec![t])
}

/// Candidate 0 trades only buy legs (rate set 02-01, GST 03-01), candidate 1 only sell legs
/// (rate set 04-01, GST 03-01, DP 03-15); both in sample only, OOS trades nothing.
fn gated_range(with_sell_candidate: bool) -> ChargesUsed {
    let (bs, ch, ps) = (bars("100"), dated_charges(), sets(2));
    let mut params = params(&ch, &ps);
    params.cost = CostModel::new(&ch, HistoryPolicy::LatestForAll, ChargeExchange::Nse);
    let mut run = |win: Window, i: usize, slice: &[Candle], _: GateEnv<'_>| {
        Ok(match (win, i) {
            (Window::InSample, 0) => one_sided(slice, LegSide::Buy),
            (Window::InSample, 1) if with_sell_candidate => one_sided(slice, LegSide::Sell),
            _ => out(vec![]),
        })
    };
    walk_forward_gated(&cfg(), &bs, &params, "IT", &quiet, &mut run)
        .unwrap()
        .1
}

#[test]
fn the_gated_runner_spans_rate_sets_used_only_in_sample() {
    assert_eq!(
        gated_range(true),
        ChargesUsed::Range(day(2026, 2, 1), day(2026, 4, 1))
    );
    // Without the sell-only candidate the sell set and DP are not in the range.
    assert_eq!(
        gated_range(false),
        ChargesUsed::Range(day(2026, 2, 1), day(2026, 3, 1))
    );
}

#[test]
fn a_gated_run_with_no_trades_levies_no_charges() {
    let (bs, ch, ps) = (bars("100"), charges(), sets(1));
    let mut run = |_: Window, _: usize, _: &[Candle], _: GateEnv<'_>| Ok(out(vec![]));
    let (_, used) =
        walk_forward_gated(&cfg(), &bs, &params(&ch, &ps), "IT", &quiet, &mut run).unwrap();
    assert_eq!(used, ChargesUsed::NoneLevied);
}
