//! G03 and G19 boundaries, week reset, unknown inputs and exits.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use bazaar_core::money::Qty;
use bazaar_risk::classify::{Classification, classify};
use bazaar_risk::config::{Baseline, GateConfig, RuleCtx};
use bazaar_risk::rules::loss_limits::{g03, g19};
use bazaar_risk::snapshot::{PortfolioSnapshot, Position};
use bazaar_risk::types::{OrderIntent, Side, Tri};
use chrono::{NaiveDate, TimeZone};
use chrono_tz::Asia::Kolkata;
use rust_decimal::Decimal;

const TEXT: &str = include_str!("../../../fixtures/config/risk.toml");

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

/// Limits: daily 3%, weekly 6% of Rs 1,00,000.
fn ctx() -> RuleCtx {
    let cfg = GateConfig::from_toml_str(TEXT).unwrap();
    let mut limits = cfg.effective(&Baseline::none());
    limits.max_daily_loss_pct = d("3");
    limits.max_weekly_loss_pct = d("6");
    RuleCtx {
        limits,
        mode: cfg.mode(),
        config_hash: cfg.hash().clone(),
        now: Kolkata.with_ymd_and_hms(2026, 11, 19, 11, 0, 0).unwrap(),
    }
}

/// Thursday 2026-11-19; its Monday is 2026-11-16.
fn snap(day: &str, week: &str) -> PortfolioSnapshot {
    let mut s = PortfolioSnapshot::unknown();
    s.as_of = Tri::Known(Kolkata.with_ymd_and_hms(2026, 11, 19, 11, 0, 0).unwrap());
    s.positions = Tri::Known(vec![]);
    s.day_pnl = Tri::Known(d(day));
    s.week_pnl = Tri::Known(d(week));
    s.week_start = Tri::Known(NaiveDate::from_ymd_opt(2026, 11, 16).unwrap());
    s.starting_cash = Tri::Known(d("100000"));
    s.realised_pnl = Tri::Known(Decimal::ZERO);
    s.equity_mtm = Tri::Known(d("100000"));
    s
}

fn intent(side: Side, qty: u32) -> OrderIntent {
    OrderIntent {
        symbol: "ABC".into(),
        side,
        qty: Qty(qty),
        price: d("500"),
        stop: Some(d("490")),
        new_stop: None,
        lot_size: 1,
        tick_size: d("0.05"),
    }
}

fn entry(s: &PortfolioSnapshot) -> Classification {
    classify(&intent(Side::Buy, 10), s).unwrap()
}

#[test]
fn g03_boundaries_at_minus_2_99_3_00_3_01_percent() {
    let c = ctx();
    for (pnl, ok) in [
        ("-2990", true),
        ("-3000", false),
        ("-3010", false),
        ("500", true),
    ] {
        let s = snap(pnl, "0");
        let r = g03(&c, &entry(&s), &s);
        assert_eq!(r.passed(), ok, "{pnl}");
        assert!(ok || (r.message().contains("Rs 3,000") && r.message().ends_with("(G03)")));
        assert!(!r.is_input_missing());
    }
}

#[test]
fn g19_boundaries_at_minus_5_99_6_00_6_01_percent() {
    let c = ctx();
    for (pnl, ok) in [("-5990", true), ("-6000", false), ("-6010", false)] {
        let s = snap("0", pnl);
        let r = g19(&c, &entry(&s), &s);
        assert_eq!(r.passed(), ok, "{pnl}");
        assert!(ok || r.message().ends_with("(G19)"));
    }
}

#[test]
fn equity_base_is_capped_at_starting_cash() {
    let c = ctx();
    // Realised +50,000 does not raise the base above starting cash: 3% stays Rs 3,000.
    let mut s = snap("-3000", "0");
    s.realised_pnl = Tri::Known(d("50000"));
    s.equity_mtm = Tri::Known(d("150000"));
    assert!(!g03(&c, &entry(&s), &s).passed());
}

#[test]
fn week_resets_on_monday_only_with_the_right_week_start() {
    let c = ctx();
    // Monday morning: week_start must be that same Monday, with a fresh (small) loss.
    let mut s = snap("0", "-100");
    s.as_of = Tri::Known(Kolkata.with_ymd_and_hms(2026, 11, 16, 9, 20, 0).unwrap());
    assert!(g19(&c, &entry(&s), &s).passed());
    // Last week's start on Monday: stale figure, rejected as missing input.
    s.week_start = Tri::Known(NaiveDate::from_ymd_opt(2026, 11, 9).unwrap());
    let r = g19(&c, &entry(&s), &s);
    assert!(!r.passed() && r.is_input_missing());
    // Sunday still belongs to the week that began the previous Monday.
    let mut sun = snap("0", "-100");
    sun.as_of = Tri::Known(Kolkata.with_ymd_and_hms(2026, 11, 22, 10, 0, 0).unwrap());
    assert!(g19(&c, &entry(&sun), &sun).passed());
}

#[test]
fn unknown_inputs_reject_entries_as_missing() {
    let c = ctx();
    let mut s = snap("-1", "-1");
    s.day_pnl = Tri::Unknown;
    let r = g03(&c, &entry(&s), &s);
    assert!(!r.passed() && r.is_input_missing());
    let breakers: [fn(&mut PortfolioSnapshot); 4] = [
        |s| s.week_pnl = Tri::Unknown,
        |s| s.week_start = Tri::Unknown,
        |s| s.as_of = Tri::Unknown,
        |s| s.equity_mtm = Tri::Unknown,
    ];
    for f in breakers {
        let mut s = snap("-1", "-1");
        f(&mut s);
        let r = g19(&c, &entry(&s), &s);
        assert!(!r.passed() && r.is_input_missing());
    }
    let unknown = PortfolioSnapshot::unknown();
    assert!(!g03(&c, &entry(&unknown), &unknown).passed());
}

#[test]
fn exits_and_stop_moves_pass_even_past_the_limit() {
    let c = ctx();
    let mut s = snap("-9000", "-9000");
    s.positions = Tri::Known(vec![Position {
        symbol: "ABC".into(),
        side: Side::Buy,
        qty: 10,
        stop: Tri::Known(d("490")),
        risk_price: Tri::Known(d("500")),
        avg_cost: Tri::Known(d("500")),
    }]);
    let exit = classify(&intent(Side::Sell, 10), &s).unwrap();
    assert!(g03(&c, &exit, &s).passed() && g19(&c, &exit, &s).passed());
    let mut mv = intent(Side::Buy, 0);
    mv.new_stop = Some(d("495"));
    let stop_move = classify(&mv, &s).unwrap();
    assert!(g03(&c, &stop_move, &s).passed() && g19(&c, &stop_move, &s).passed());
    // The same entry is blocked.
    assert!(!g03(&c, &entry(&s), &s).passed() && !g19(&c, &entry(&s), &s).passed());
}

#[test]
fn overflowing_figures_reject_without_a_zero_cap() {
    let c = ctx();
    let mut s = snap("0", "0");
    s.day_pnl = Tri::Known(Decimal::MIN);
    let r = g03(&c, &entry(&s), &s);
    assert!(!r.passed() && !r.message().contains("Rs 0 "));
}
