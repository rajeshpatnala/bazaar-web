use std::path::Path;
use std::str::FromStr;

use super::*;
use crate::ledger::Drawdown;
use crate::metrics::Metrics;

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

/// The thresholds the tests rely on are set here, not read from the config file.
const THIN: u32 = 5;

fn cfg() -> BacktestConfig {
    let mut c =
        BacktestConfig::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config")).unwrap();
    c.evidence.min_profitable_fold_pct = d("60");
    c.evidence.min_oos_is_ratio = d("0.5");
    c.walk_forward.min_folds = 3;
    c.walk_forward.thin_fold_trades = THIN;
    c
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
        thin: n < THIN,
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
fn thin_folds_cannot_supply_the_fold_minimum() {
    let c = cfg();
    // 2 profitable voting folds (100%) plus 6 thin losing ones: below min_folds (3) voting folds.
    let mut folds = vec![ran(10, "100", "0.3", "0.3"), ran(10, "50", "0.3", "0.3")];
    folds.extend((0..6).map(|_| ran(2, "-20", "0.3", "-0.5")));
    let v = vote(&c, &folds).unwrap();
    assert_eq!((v.voting_folds, v.profitable_pct), (2, Some(d("100"))));
    assert!(!v.vote_ok);
    folds.push(ran(10, "40", "0.3", "0.3"));
    assert!(vote(&c, &folds).unwrap().vote_ok);
}

#[test]
fn a_thin_fold_with_an_extreme_r_does_not_move_the_ratio() {
    let c = cfg();
    let base = [ran(10, "10", "0.4", "0.2")];
    let mut with_thin = base.to_vec();
    with_thin.push(ran(2, "10", "0.4", "50"));
    assert_eq!(
        vote(&c, &base).unwrap().oos_is_ratio,
        vote(&c, &with_thin).unwrap().oos_is_ratio
    );
}

#[test]
fn an_overflowing_r_mean_is_an_error_not_a_pass() {
    let max = Decimal::MAX.to_string();
    let r = vote(
        &cfg(),
        &[ran(10, "10", &max, "0.2"), ran(10, "10", &max, "0.2")],
    );
    assert!(matches!(r, Err(BacktestError::Overflow(_))));
}

#[test]
fn weighted_mean_reports_overflow_in_the_product_and_in_the_sum() {
    let big = Decimal::MAX;
    assert!(matches!(
        weighted_mean(&[(big, 2)]),
        Err(BacktestError::Overflow(_))
    ));
    assert!(matches!(
        weighted_mean(&[(big, 1), (big, 1)]),
        Err(BacktestError::Overflow(_))
    ));
    assert_eq!(weighted_mean(&[]).unwrap(), None);
    assert_eq!(
        weighted_mean(&[(d("1"), 1), (d("3"), 3)]).unwrap(),
        Some(d("2.5"))
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
