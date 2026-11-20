use std::path::Path;
use std::str::FromStr;

use bazaar_core::money::{Price, Qty};
use bazaar_core::segment::Segment;
use chrono::{NaiveDate, TimeZone};
use chrono_tz::Asia::Kolkata;

use super::*;
use crate::types::{ChargeBreakdown, Fill, FillKind, Leg, LegInstrument, LegSide, Product};

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn fill(side: LegSide) -> Fill {
    let price = Price::snap_to_tick(d("100"), d("0.05")).unwrap();
    Fill {
        leg: Leg {
            instrument_token: 1,
            symbol: "S1".into(),
            segment: Segment::Equity,
            instrument: LegInstrument::Cash,
            product: Product::Intraday,
            side,
            qty: Qty(1),
            price,
            trade_date: NaiveDate::from_ymd_opt(2026, 6, 5).unwrap(),
        },
        kind: FillKind::Entry,
        bar_time: Kolkata.with_ymd_and_hms(2026, 6, 5, 9, 15, 0).unwrap(),
        reference_price: price,
        slippage_per_unit: Decimal::ZERO,
        charges: ChargeBreakdown::default(),
    }
}

fn trade(net: &str, r: Option<&str>) -> Trade {
    Trade {
        strategy_id: "s".into(),
        entry: fill(LegSide::Buy),
        exit: fill(LegSide::Sell),
        gross_pnl: d(net),
        charges: ChargeBreakdown::default(),
        net_pnl: d(net),
        risk_inr: d("100"),
        r_multiple: r.map(d),
    }
}

fn config() -> BacktestConfig {
    BacktestConfig::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config")).unwrap()
}

#[test]
fn metrics_by_hand() {
    // Nets +300 (1.5R), +100 (0.5R), -100 (-0.5R), -50 (no R).
    // Wins 2 of 4 = 0.5; net 250; profit 400, loss 150, PF 400/150 = 2.6666 (4 dp toward zero);
    // expectancy over the 3 R trades = (1.5 + 0.5 - 0.5) / 3 = 0.5; one trade has no R.
    let trades = [
        trade("300", Some("1.5")),
        trade("100", Some("0.5")),
        trade("-100", Some("-0.5")),
        trade("-50", None),
    ];
    let m = Metrics::from_trades(&trades, d("1000"), &[]).unwrap();
    assert_eq!((m.trades, m.wins, m.losses), (4, 2, 2));
    assert_eq!(m.win_rate, d("0.5"));
    assert_eq!(m.net_pnl, d("250"));
    assert_eq!((m.gross_profit, m.gross_loss), (d("400"), d("150")));
    assert_eq!(m.profit_factor, Some(d("2.6666")));
    assert_eq!(m.expectancy_r, Some(d("0.5")));
    assert_eq!((m.r_trades, m.no_r_trades), (3, 1));
}

#[test]
fn flat_trade_is_neither_win_nor_loss() {
    let m = Metrics::from_trades(&[trade("0", Some("0"))], d("1000"), &[]).unwrap();
    assert_eq!((m.wins, m.losses), (0, 0));
    assert_eq!(m.win_rate, d("0"));
    assert_eq!(m.profit_factor, None);
}

#[test]
fn no_losses_gives_undefined_profit_factor_and_no_trades_gives_zeros() {
    let m = Metrics::from_trades(&[trade("10", Some("0.1"))], d("1000"), &[]).unwrap();
    assert_eq!(m.profit_factor, None);
    let e = Metrics::from_trades(&[], d("1000"), &[]).unwrap();
    assert_eq!(e.trades, 0);
    assert_eq!(e.win_rate, d("0"));
    assert_eq!(e.profit_factor, None);
    assert_eq!(e.expectancy_r, None);
}

#[test]
fn only_no_r_trades_give_no_expectancy() {
    let m = Metrics::from_trades(&[trade("10", None)], d("1000"), &[]).unwrap();
    assert_eq!(m.expectancy_r, None);
    assert_eq!((m.r_trades, m.no_r_trades), (0, 1));
}

#[test]
fn drawdown_uses_starting_equity() {
    let curve = [EquityPoint {
        time: Kolkata.with_ymd_and_hms(2026, 6, 5, 15, 30, 0).unwrap(),
        equity: d("900"),
    }];
    let m = Metrics::from_trades(&[], d("1000"), &curve).unwrap();
    assert_eq!(m.drawdown.max_abs, d("100"));
    assert_eq!(m.drawdown.max_pct, d("0.1"));
}

#[test]
fn sum_overflow_is_an_error() {
    let trades = [trade("7.9E+28", None), trade("7.9E+28", None)];
    assert!(matches!(
        Metrics::from_trades(&trades, d("0"), &[]),
        Err(BacktestError::Overflow(_))
    ));
}

fn m(trades: u32, pf: &str, e: &str) -> Metrics {
    Metrics {
        trades,
        wins: 0,
        losses: 0,
        win_rate: d("0"),
        net_pnl: d("0"),
        gross_profit: d("0"),
        gross_loss: d("0"),
        profit_factor: Some(d(pf)),
        expectancy_r: Some(d(e)),
        r_trades: trades,
        no_r_trades: 0,
        drawdown: Drawdown {
            max_abs: d("0"),
            max_pct: d("0"),
        },
    }
}

fn facts(folds: u32, regimes: u32, trials: u32) -> RunFacts {
    RunFacts {
        folds,
        regimes,
        costs_approximate: false,
        trials_in_run: trials,
    }
}

#[test]
fn labels_follow_trade_counts_and_regimes() {
    let c = config();
    let a = |n, f: RunFacts| assess(&c, &m(n, "1.3", "0.1"), &f).unwrap();
    assert_eq!(a(29, facts(4, 2, 1)).label, EvidenceLabel::LowEvidence);
    assert!(!a(29, facts(4, 2, 1)).meets_pooled_bar);
    assert_eq!(a(30, facts(4, 2, 1)).label, EvidenceLabel::Provisional);
    assert!(a(30, facts(4, 2, 1)).meets_pooled_bar);
    assert_eq!(a(99, facts(4, 2, 1)).label, EvidenceLabel::Provisional);
    assert_eq!(a(100, facts(4, 2, 1)).label, EvidenceLabel::Trusted);
    assert_eq!(a(100, facts(4, 1, 1)).label, EvidenceLabel::Provisional);
    assert_eq!(a(100, facts(3, 2, 1)).label, EvidenceLabel::LowEvidence);
    assert!(!a(100, facts(3, 2, 1)).meets_pooled_bar);
}

#[test]
fn trial_penalty_raises_the_bar_and_caps_the_label() {
    let c = config();
    let a = |trials, pf| assess(&c, &m(100, pf, "0.1"), &facts(4, 2, trials)).unwrap();
    // 1 trial: bar 1.3. 2-10: 1.4. 11-50: 1.5 and Provisional cap. >50: 1.6 and cap.
    assert_eq!(a(1, "1.3").required_profit_factor, d("1.3"));
    assert_eq!(a(2, "1.3").required_profit_factor, d("1.4"));
    assert!(!a(2, "1.3").meets_pooled_bar);
    assert!(a(2, "1.4").meets_pooled_bar);
    assert_eq!(a(10, "1.4").label, EvidenceLabel::Trusted);
    assert_eq!(a(11, "1.5").required_profit_factor, d("1.5"));
    assert_eq!(a(11, "1.5").label, EvidenceLabel::Provisional);
    assert!(!a(11, "1.4").meets_pooled_bar);
    assert_eq!(a(50, "1.5").required_profit_factor, d("1.5"));
    assert_eq!(a(51, "1.6").required_profit_factor, d("1.6"));
    assert_eq!(a(51, "1.6").label, EvidenceLabel::Provisional);
}

#[test]
fn prior_trials_count_toward_the_penalty() {
    let mut c = config();
    c.trials.prior_trials = 1;
    let r = assess(&c, &m(100, "1.4", "0.1"), &facts(4, 2, 1)).unwrap();
    assert_eq!(r.required_profit_factor, d("1.4"));
}

#[test]
fn prior_trials_crossing_the_cap_lower_the_label_in_assess() {
    let mut c = config();
    let trusted = |c: &BacktestConfig| assess(c, &m(100, "1.5", "0.1"), &facts(4, 2, 1)).unwrap();
    assert_eq!(trusted(&c).label, EvidenceLabel::Trusted);
    // 10 prior + 1 in this run = 11 total: above the tier-1 cap, so Provisional and bar 1.5.
    c.trials.prior_trials = 10;
    let r = trusted(&c);
    assert_eq!(r.label, EvidenceLabel::Provisional);
    assert_eq!(r.required_profit_factor, d("1.5"));
    // The cap lowers the label only: PF 1.5 equals the bar, so the pooled verdict stays true.
    assert!(r.meets_pooled_bar);
    // 9 prior + 1 = 10 is at the cap, not above it: still Trusted, bar 1.4.
    c.trials.prior_trials = 9;
    let r = assess(&c, &m(100, "1.4", "0.1"), &facts(4, 2, 1)).unwrap();
    assert_eq!(r.label, EvidenceLabel::Trusted);
    assert_eq!(r.required_profit_factor, d("1.4"));
}

#[test]
fn approximate_costs_are_exploratory_and_never_meet_the_bar() {
    let c = config();
    let mut f = facts(4, 2, 1);
    f.costs_approximate = true;
    let r = assess(&c, &m(500, "9", "2"), &f).unwrap();
    assert_eq!(r.label, EvidenceLabel::Exploratory);
    assert!(!r.meets_pooled_bar);
}

#[test]
fn undefined_or_low_figures_fail_the_bar() {
    let c = config();
    let f = facts(4, 2, 1);
    let mut undefined = m(100, "5", "1");
    undefined.profit_factor = None;
    assert!(!assess(&c, &undefined, &f).unwrap().meets_pooled_bar);
    let mut no_e = m(100, "5", "1");
    no_e.expectancy_r = None;
    assert!(!assess(&c, &no_e, &f).unwrap().meets_pooled_bar);
    assert!(
        !assess(&c, &m(100, "5", "0.09"), &f)
            .unwrap()
            .meets_pooled_bar
    );
    assert!(
        !assess(&c, &m(100, "1.29", "1"), &f)
            .unwrap()
            .meets_pooled_bar
    );
}

#[test]
fn rounding_never_lifts_a_figure_across_the_bar() {
    // PF 1299.96 / 1000 = 1.29996 stays 1.2999 (not 1.3000); expectancy 0.09996 stays 0.0999.
    let t = [
        trade("1299.96", Some("0.09996")),
        trade("-1000", Some("0.09996")),
    ];
    let m = Metrics::from_trades(&t, d("1000"), &[]).unwrap();
    assert_eq!(m.profit_factor, Some(d("1.2999")));
    assert_eq!(m.expectancy_r, Some(d("0.0999")));
    let c = config();
    let mut thirty = m;
    thirty.trades = 30;
    thirty.r_trades = 30;
    assert!(
        !assess(&c, &thirty, &facts(4, 2, 1))
            .unwrap()
            .meets_pooled_bar
    );
    // Exactly 1.3000 and 0.1 pass.
    let exact = [trade("1300", Some("0.1")), trade("-1000", Some("0.1"))];
    let mut e = Metrics::from_trades(&exact, d("1000"), &[]).unwrap();
    e.trades = 30;
    e.r_trades = 30;
    assert!(assess(&c, &e, &facts(4, 2, 1)).unwrap().meets_pooled_bar);
}

#[test]
fn thirty_trades_with_three_r_values_do_not_meet_the_bar() {
    let c = config();
    let mut x = m(30, "5", "1");
    x.r_trades = 3;
    x.no_r_trades = 27;
    let r = assess(&c, &x, &facts(4, 2, 1)).unwrap();
    assert!(!r.meets_pooled_bar);
    assert_eq!(r.label, EvidenceLabel::LowEvidence);
    assert_eq!(r.no_r_trades, 27);
}

#[test]
fn drawdown_tracks_peak_recovery_and_deeper_fall() {
    let at = |h, v: &str| EquityPoint {
        time: Kolkata.with_ymd_and_hms(2026, 6, 5, h, 0, 0).unwrap(),
        equity: d(v),
    };
    let curve = [at(10, "1200"), at(11, "900"), at(12, "1100")];
    let m = Metrics::from_trades(&[], d("1000"), &curve).unwrap();
    assert_eq!(m.drawdown.max_abs, d("300"));
    assert_eq!(m.drawdown.max_pct, d("0.25"));
}
