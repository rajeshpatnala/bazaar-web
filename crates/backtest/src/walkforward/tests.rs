use std::cell::RefCell;
use std::path::Path;
use std::str::FromStr;

use bazaar_core::candle::Interval;
use bazaar_core::money::{Price, Qty};
use bazaar_core::segment::Segment;
use chrono::{DateTime, Datelike, TimeZone, Weekday};
use chrono_tz::Asia::Kolkata;

use super::*;
use crate::types::{ChargeBreakdown, Fill, FillKind, Leg, LegInstrument, LegSide, Product};

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

fn params(candidates: usize) -> WfParams {
    WfParams {
        capital: d("100000"),
        candidates,
        regimes: 2,
        costs_approximate: false,
    }
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
    walk_forward(&c, &b, &params(1), &mut run).unwrap();
    let log = log.into_inner();
    assert_eq!(log.len(), 10);
    for (i, f) in folds.iter().enumerate() {
        let (is, oos) = (&log[2 * i], &log[2 * i + 1]);
        assert_eq!((is.0, oos.0), (Window::InSample, Window::OutOfSample));
        assert!(is.3.is_none(), "IS must not receive gate evidence");
        assert!(oos.3.is_some());
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
        walk_forward(&c, &b2, &params(1), &mut run).unwrap().folds[0]
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
    let r = walk_forward(&c, &b, &params(3), &mut run).unwrap();
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
    let r = walk_forward(&c, &b, &params(1), &mut run).unwrap();
    assert_eq!(oos_runs.into_inner(), 0);
    assert!(r.folds.iter().all(|f| f.oos.is_none() && f.thin));
    assert_eq!(r.pooled.trades, 0);
    assert_eq!(r.oos_trades, vec![]);
}

#[test]
fn pooled_oos_trades_feed_the_assessment_and_thin_folds_are_flagged() {
    let b = bars("100");
    let c = cfg();
    let mut run =
        |_: Window, _: usize, s: &[Candle], _: Option<EvidenceInputs>| Ok(winner_loser(s, "50"));
    let r = walk_forward(&c, &b, &params(1), &mut run).unwrap();
    // 2 trades per OOS fold: below thin_fold_trades (5); pooled 10 trades over 5 folds.
    assert!(r.folds.iter().all(|f| f.thin));
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
    assert!(matches!(
        walk_forward(&c, &b, &params(0), &mut ok),
        Err(BacktestError::Invalid(_))
    ));
    let mut bad = |_: Window, _: usize, _: &[Candle], _: Option<EvidenceInputs>| {
        Err(BacktestError::Overflow("test"))
    };
    assert!(matches!(
        walk_forward(&c, &b, &params(1), &mut bad),
        Err(BacktestError::Overflow(_))
    ));
}
