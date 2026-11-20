use std::path::Path;
use std::str::FromStr;

use super::*;
use crate::ledger::Drawdown;
use crate::metrics::Metrics;

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn cfg() -> BacktestConfig {
    BacktestConfig::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config")).unwrap()
}

fn m(r_trades: u32, net: &str, exp: &str) -> Metrics {
    Metrics {
        trades: r_trades,
        wins: 0,
        losses: 0,
        win_rate: Decimal::ZERO,
        net_pnl: d(net),
        gross_profit: Decimal::ZERO,
        gross_loss: Decimal::ZERO,
        profit_factor: None,
        expectancy_r: Some(d(exp)),
        r_trades,
        no_r_trades: 0,
        drawdown: Drawdown {
            max_abs: Decimal::ZERO,
            max_pct: Decimal::ZERO,
        },
    }
}

/// A fold with an OOS run of `n` R trades, net `net`, mean R `oos`; IS mean R `is`.
fn ran(n: u32, net: &str, is: &str, oos: &str) -> FoldResult {
    FoldResult {
        chosen: 0,
        is: m(20, "10", is),
        oos: Some(m(n, net, oos)),
        skipped_in_is: false,
        thin: n < 5,
    }
}

fn skipped() -> FoldResult {
    FoldResult {
        chosen: 0,
        is: m(20, "10", "0.01"),
        oos: None,
        skipped_in_is: true,
        thin: false,
    }
}

#[test]
fn skipped_folds_count_as_failures_in_the_vote() {
    let c = cfg();
    // 3 profitable folds of 5: 60% passes (floor 60); the same 3 plus 2 skipped is 3 of 5.
    let good = || ran(10, "100", "0.3", "0.3");
    let folds = [good(), good(), good(), skipped(), skipped()];
    let v = vote(&c, &folds).unwrap();
    assert_eq!(
        (v.voting_folds, v.profitable_folds, v.skipped_folds),
        (5, 3, 2)
    );
    assert_eq!(v.profitable_pct, Some(d("60")));
    assert!(v.vote_ok);
    // Dropping the skipped folds from the denominator would give 100%: one more skip fails it.
    let folds = [good(), good(), good(), skipped(), skipped(), skipped()];
    let v = vote(&c, &folds).unwrap();
    assert_eq!(v.profitable_pct, Some(d("50")));
    assert!(!v.vote_ok);
}

#[test]
fn thin_folds_are_left_out_of_the_vote_and_the_single_fold_check() {
    let c = cfg();
    let folds = [ran(10, "100", "0.3", "0.3"), ran(2, "100", "0.3", "0.3")];
    let v = vote(&c, &folds).unwrap();
    assert_eq!((v.voting_folds, v.profitable_folds), (1, 1));
    // One profitable voting fold supplies all the profit.
    assert!(!v.single_fold_ok);
    let two = [ran(10, "100", "0.3", "0.3"), ran(10, "50", "0.3", "0.3")];
    assert!(vote(&c, &two).unwrap().single_fold_ok);
}

#[test]
fn oos_is_ratio_is_weighted_and_fails_closed() {
    let c = cfg();
    let v = vote(
        &c,
        &[ran(10, "10", "0.4", "0.2"), ran(10, "10", "0.4", "0.2")],
    )
    .unwrap();
    assert_eq!(v.oos_is_ratio, Some(d("0.5")));
    assert!(v.ratio_ok);
    let v = vote(&c, &[ran(10, "10", "0.4", "0.19")]).unwrap();
    assert_eq!(v.oos_is_ratio, Some(d("0.475")));
    assert!(!v.ratio_ok);
    // IS mean not above zero: undefined, not a pass. No OOS fold at all: undefined.
    let v = vote(&c, &[ran(10, "10", "0", "0.2")]).unwrap();
    assert_eq!((v.oos_is_ratio, v.ratio_ok), (None, false));
    let v = vote(&c, &[skipped()]).unwrap();
    assert_eq!(
        (v.oos_is_ratio, v.ratio_ok, v.vote_ok),
        (None, false, false)
    );
}

#[test]
fn no_folds_pass_nothing() {
    let v = vote(&cfg(), &[]).unwrap();
    assert_eq!((v.voting_folds, v.profitable_pct), (0, None));
    assert!(!v.all_ok());
}

#[test]
fn all_ok_needs_every_check() {
    let c = cfg();
    let folds = [
        ran(10, "100", "0.3", "0.3"),
        ran(10, "80", "0.3", "0.3"),
        ran(10, "60", "0.3", "0.3"),
    ];
    assert!(vote(&c, &folds).unwrap().all_ok());
    let losing = [ran(10, "-5", "0.3", "0.3"), ran(10, "-5", "0.3", "0.3")];
    assert!(!vote(&c, &losing).unwrap().all_ok());
}
