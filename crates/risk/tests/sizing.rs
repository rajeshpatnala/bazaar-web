//! Sizing tests: spec example, caps, lots, floors, equity basis, rejects and properties.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use bazaar_risk::config::{Baseline, EffectiveLimits, GateConfig};
use bazaar_risk::sizing::{
    Sized, SizingError, SizingInput, size, size_with_cost, size_with_reference, sizing_equity,
};
use bazaar_risk::snapshot::PortfolioSnapshot;
use bazaar_risk::types::{Side, Tri};
use proptest::prelude::*;
use rust_decimal::Decimal;

const TEXT: &str = include_str!("../../../fixtures/config/risk.toml");

fn d(n: i64) -> Decimal {
    Decimal::from(n)
}

/// Fixture limits with the spec's values: 1% risk, 25% cap, Rs 60 buffer floor, factor 0.5.
fn limits() -> EffectiveLimits {
    let mut l = GateConfig::from_toml_str(TEXT)
        .unwrap()
        .effective(&Baseline::none());
    l.max_risk_per_trade_pct = d(1);
    l.max_position_value_pct = d(25);
    l.min_charges_slippage_buffer = d(60);
    l.override_size_factor = Decimal::new(5, 1);
    // G16 is not under test unless a test sets it: large enough that it never binds here.
    l.max_notional_per_order_inr = d(1_000_000);
    l.cap_equity_at_starting_cash = true;
    l
}

fn input() -> SizingInput {
    SizingInput {
        equity: d(100_000),
        side: Side::Buy,
        entry: d(500),
        stop: d(490),
        lot_size: 1,
        tick_size: Decimal::new(5, 2),
        buffer: Tri::Known(d(60)),
        overridden: false,
    }
}

fn snap(start: i64, realised: i64, mtm: i64) -> PortfolioSnapshot {
    let mut s = PortfolioSnapshot::unknown();
    s.starting_cash = Tri::Known(d(start));
    s.realised_pnl = Tri::Known(d(realised));
    s.equity_mtm = Tri::Known(d(mtm));
    s
}

#[test]
fn spec_example_is_94_before_the_cap_and_50_after() {
    // Without the G14 cap: qty 94, risk at stop Rs 940 + Rs 60 buffer = the Rs 1,000 budget.
    let mut l = limits();
    l.max_position_value_pct = d(100);
    let s = size(&l, &input()).unwrap();
    assert_eq!((s.qty, s.risk_at_stop, s.budget), (94, d(940), d(1000)));
    assert!(!s.capped_by_position_cap);

    // With the 25% cap: Rs 25,000 / Rs 500 = 50.
    let s = size(&limits(), &input()).unwrap();
    assert_eq!((s.qty, s.risk_at_stop, s.notional), (50, d(500), d(25_000)));
    assert!(s.capped_by_position_cap);
}

#[test]
fn lots_are_floored_never_rounded_up() {
    let mut l = limits();
    l.max_position_value_pct = d(100);
    let mut i = input();
    i.lot_size = 25; // 94 shares -> 3 lots = 75
    let s = size(&l, &i).unwrap();
    assert_eq!((s.lots, s.qty), (3, 75));
    assert!(s.risk_at_stop + d(60) <= s.budget);
}

#[test]
fn one_lot_over_budget_is_no_trade() {
    let mut i = input();
    i.lot_size = 100; // one lot risks Rs 1,000 + Rs 60 buffer, budget Rs 1,000
    assert_eq!(size(&limits(), &i), Err(SizingError::BudgetBelowOneLot));
    // Buffer eats the whole budget.
    let mut i = input();
    i.buffer = Tri::Known(d(1000));
    assert_eq!(size(&limits(), &i), Err(SizingError::BudgetBelowOneLot));
    assert!(
        SizingError::BudgetBelowOneLot
            .to_string()
            .contains("no trade")
    );
}

#[test]
fn position_cap_below_one_lot_is_no_trade() {
    let mut i = input();
    i.lot_size = 100; // 100 x Rs 500 = Rs 50,000 > Rs 25,000 cap
    let mut l = limits();
    l.max_risk_per_trade_pct = d(10); // budget is not the problem
    assert_eq!(size(&l, &i), Err(SizingError::CapBelowOneLot));
}

#[test]
fn override_applies_the_factor_to_the_capped_qty_and_floors_to_lots() {
    let mut i = input();
    i.overridden = true;
    let s = size(&limits(), &i).unwrap();
    assert_eq!(s.qty, 25); // 50 x 0.5
    assert!(s.reduced_by_override && s.capped_by_position_cap);

    i.lot_size = 10; // capped 50 -> 5 lots; x 0.5 = 2.5 -> 2 lots = 20
    let s = size(&limits(), &i).unwrap();
    assert_eq!((s.lots, s.qty), (2, 20));

    i.lot_size = 50; // 1 lot; x 0.5 -> 0 lots
    assert_eq!(size(&limits(), &i), Err(SizingError::OverrideBelowOneLot));
}

#[test]
fn override_factor_outside_zero_one_is_rejected() {
    let mut i = input();
    i.overridden = true;
    for bad in [Decimal::ZERO, d(2), d(-1)] {
        let mut l = limits();
        l.override_size_factor = bad;
        assert_eq!(size(&l, &i), Err(SizingError::BadSizeFactor));
    }
    // Ignored when the trade is not overridden.
    let mut l = limits();
    l.override_size_factor = d(2);
    i.overridden = false;
    assert!(size(&l, &i).is_ok());
}

#[test]
fn invalid_inputs_are_rejected() {
    let l = limits();
    let case = |f: &dyn Fn(&mut SizingInput)| {
        let mut i = input();
        f(&mut i);
        size(&l, &i)
    };
    assert_eq!(case(&|i| i.lot_size = 0), Err(SizingError::ZeroLot));
    assert_eq!(
        case(&|i| i.tick_size = Decimal::ZERO),
        Err(SizingError::ZeroTick)
    );
    assert_eq!(case(&|i| i.tick_size = d(-1)), Err(SizingError::ZeroTick));
    assert_eq!(
        case(&|i| i.stop = i.entry),
        Err(SizingError::EntryEqualsStop)
    );
    assert_eq!(
        case(&|i| i.stop = d(510)),
        Err(SizingError::StopOnWrongSide)
    );
    assert_eq!(
        case(&|i| {
            i.side = Side::Sell;
            i.stop = d(490);
        }),
        Err(SizingError::StopOnWrongSide)
    );
    assert_eq!(
        case(&|i| i.equity = Decimal::ZERO),
        Err(SizingError::ZeroEquity)
    );
    assert_eq!(
        case(&|i| i.entry = Decimal::ZERO),
        Err(SizingError::BadPrice)
    );
    assert_eq!(
        case(&|i| i.stop = Decimal::ZERO),
        Err(SizingError::BadPrice)
    );
    assert_eq!(
        case(&|i| i.buffer = d(-5).into_known()),
        Err(SizingError::NegativeCost)
    );
    assert!(matches!(
        case(&|i| i.buffer = Tri::Unknown),
        Err(SizingError::InputUnknown(_))
    ));
}

trait IntoKnown {
    fn into_known(self) -> Tri<Decimal>;
}
impl IntoKnown for Decimal {
    fn into_known(self) -> Tri<Decimal> {
        Tri::Known(self)
    }
}

#[test]
fn short_trades_size_on_the_distance_above_entry() {
    let mut i = input();
    i.side = Side::Sell;
    i.stop = d(510);
    let s = size(&limits(), &i).unwrap();
    assert_eq!(s.qty, 50);
}

#[test]
fn equity_is_the_lower_of_book_and_mark_to_market() {
    let l = limits();
    // Book 1,00,000 - 4,000 = 96,000; mark-to-market lower.
    assert_eq!(
        sizing_equity(&snap(100_000, -4000, 90_000), &l).unwrap(),
        d(90_000)
    );
    assert_eq!(
        sizing_equity(&snap(100_000, -4000, 99_000), &l).unwrap(),
        d(96_000)
    );
}

#[test]
fn equity_never_rises_above_starting_cash_when_capped() {
    let mut l = limits();
    let s = snap(100_000, 5000, 108_000);
    assert_eq!(sizing_equity(&s, &l).unwrap(), d(100_000));
    l.cap_equity_at_starting_cash = false;
    assert_eq!(sizing_equity(&s, &l).unwrap(), d(105_000));
}

#[test]
fn equity_rejects_unknown_inputs_and_non_positive_equity() {
    let l = limits();
    let mut s = snap(100_000, 0, 100_000);
    s.starting_cash = Tri::Unknown;
    assert!(matches!(
        sizing_equity(&s, &l),
        Err(SizingError::InputUnknown(_))
    ));
    let mut s = snap(100_000, 0, 100_000);
    s.realised_pnl = Tri::Unknown;
    assert!(matches!(
        sizing_equity(&s, &l),
        Err(SizingError::InputUnknown(_))
    ));
    let mut s = snap(100_000, 0, 100_000);
    s.equity_mtm = Tri::Unknown;
    assert!(matches!(
        sizing_equity(&s, &l),
        Err(SizingError::InputUnknown(_))
    ));
    assert_eq!(
        sizing_equity(&snap(100_000, -100_000, 100_000), &l),
        Err(SizingError::ZeroEquity)
    );
    assert_eq!(
        sizing_equity(&snap(100_000, 0, -5), &l),
        Err(SizingError::ZeroEquity)
    );
}

#[test]
fn overflow_is_an_error_not_a_panic() {
    let mut i = input();
    i.equity = Decimal::MAX;
    assert_eq!(size(&limits(), &i), Err(SizingError::Arithmetic));
    let mut s = snap(0, 0, 1);
    s.starting_cash = Tri::Known(Decimal::MAX);
    s.realised_pnl = Tri::Known(Decimal::MAX);
    assert_eq!(sizing_equity(&s, &limits()), Err(SizingError::Arithmetic));
}

#[test]
fn error_messages_are_plain_english() {
    for e in [
        SizingError::ZeroLot,
        SizingError::ZeroTick,
        SizingError::PriceOffTick,
        SizingError::BadPrice,
        SizingError::EntryEqualsStop,
        SizingError::StopOnWrongSide,
        SizingError::ZeroEquity,
        SizingError::InputUnknown("x"),
        SizingError::NegativeCost,
        SizingError::CapBelowOneLot,
        SizingError::OverrideBelowOneLot,
        SizingError::BadSizeFactor,
        SizingError::Arithmetic,
    ] {
        assert!(e.to_string().len() > 10, "{e:?}");
    }
}

fn run(eq: i64, entry: i64, dist: i64, lot: u32, over: bool) -> Option<Sized> {
    let mut i = input();
    i.equity = d(eq);
    i.entry = d(entry);
    i.stop = d(entry - dist);
    i.lot_size = lot;
    i.overridden = over;
    size(&limits(), &i).ok()
}

proptest! {
    #[test]
    fn risk_never_exceeds_budget_and_qty_is_a_lot_multiple(
        eq in 10_000i64..10_000_000,
        entry in 50i64..5000,
        dist_pct in 1i64..50,
        lot in prop::sample::select(vec![1u32, 5, 25, 50, 75, 100]),
        over in any::<bool>(),
    ) {
        let dist = (entry * dist_pct / 100).max(1).min(entry - 1);
        if let Some(s) = run(eq, entry, dist, lot, over) {
            prop_assert!(s.qty > 0);
            prop_assert_eq!(s.qty % lot, 0);
            prop_assert!(s.risk_at_stop + d(60) <= s.budget);
            prop_assert!(s.notional <= d(eq) * Decimal::new(25, 2));
        }
    }

    #[test]
    fn override_never_sizes_up(
        eq in 10_000i64..10_000_000,
        entry in 50i64..5000,
        lot in prop::sample::select(vec![1u32, 5, 25, 50]),
    ) {
        let dist = (entry / 20).max(1);
        let normal = run(eq, entry, dist, lot, false);
        let over = run(eq, entry, dist, lot, true);
        if let (Some(n), Some(o)) = (normal, over) {
            prop_assert!(o.qty <= n.qty);
        }
    }

    #[test]
    fn more_equity_never_gives_a_smaller_size(
        eq in 10_000i64..1_000_000,
        extra in 0i64..1_000_000,
        entry in 50i64..2000,
    ) {
        let dist = (entry / 20).max(1);
        let a = run(eq, entry, dist, 1, false);
        let b = run(eq + extra, entry, dist, 1, false);
        if let (Some(a), Some(b)) = (a, b) {
            prop_assert!(b.qty >= a.qty);
        }
    }

    #[test]
    fn same_inputs_same_size(eq in 10_000i64..1_000_000, entry in 50i64..2000) {
        let dist = (entry / 20).max(1);
        prop_assert_eq!(run(eq, entry, dist, 1, false), run(eq, entry, dist, 1, false));
    }
}

#[test]
fn cost_that_grows_with_qty_steps_the_size_down() {
    let mut l = limits();
    l.max_position_value_pct = d(100);
    // Flat Rs 60 gives 94 shares; Rs 1 a share of cost cannot fit at 94 (940 + 94 > 1000).
    let cost = |q: u32| Tri::Known(d(60) + d(i64::from(q)));
    let s = size_with_cost(&l, &input(), &cost).unwrap();
    assert!(s.qty < 94);
    let total = s.risk_at_stop + d(60) + d(i64::from(s.qty));
    assert!(total <= s.budget);
    // One more share would not have fit.
    let more = d(10) * d(i64::from(s.qty) + 1) + d(60) + d(i64::from(s.qty) + 1);
    assert!(more > s.budget);
}

#[test]
fn unknown_or_negative_cost_at_the_final_qty_rejects() {
    let l = limits();
    let neg = |q: u32| Tri::Known(if q > 1 { d(-10) } else { d(60) });
    assert_eq!(
        size_with_cost(&l, &input(), &neg),
        Err(SizingError::NegativeCost)
    );
    let unk = |q: u32| {
        if q > 1 {
            Tri::Unknown
        } else {
            Tri::Known(d(60))
        }
    };
    assert!(matches!(
        size_with_cost(&l, &input(), &unk),
        Err(SizingError::InputUnknown(_))
    ));
}

#[test]
fn cost_below_floor_sizes_with_the_floor_allowance() {
    // Rs 10 cost and Rs 59 flat buffer both size as Rs 60: 94 shares, risk 940 + 60 = budget.
    let l = limits();
    let low = size_with_cost(&l, &input(), &|_| Tri::Known(d(10))).unwrap();
    let mut i = input();
    i.buffer = Tri::Known(d(59));
    let flat = size(&l, &i).unwrap();
    let at_floor = size(&l, &input()).unwrap();
    for s in [&low, &flat] {
        assert_eq!((s.qty, s.risk_at_stop, s.budget), (50, d(500), d(1000)));
        assert_eq!(s, &at_floor);
    }
    let mut wide = l.clone();
    wide.max_position_value_pct = d(100);
    let s = size_with_cost(&wide, &input(), &|_| Tri::Known(d(10))).unwrap();
    assert_eq!((s.qty, s.risk_at_stop), (94, d(940)));
}

#[test]
fn cost_above_floor_is_the_allowance() {
    let mut l = limits();
    l.max_position_value_pct = d(100);
    // Cost Rs 100: (1000 - 100) / 10 = 90 shares.
    let s = size_with_cost(&l, &input(), &|_| Tri::Known(d(100))).unwrap();
    assert_eq!((s.qty, s.risk_at_stop), (90, d(900)));
    // Cost equal to the floor behaves as the floor.
    let s = size_with_cost(&l, &input(), &|_| Tri::Known(d(60))).unwrap();
    assert_eq!((s.qty, s.risk_at_stop), (94, d(940)));
}

#[test]
fn step_down_uses_the_floor_at_the_final_quantity() {
    // Cost is Rs 1 below 80 shares, Rs 200 above: the first estimate (one lot) is under the floor;
    // the step-down finds the largest qty whose max(cost, 60) + risk fits Rs 1,000.
    let mut l = limits();
    l.max_position_value_pct = d(100);
    let cost = |q: u32| Tri::Known(if q <= 80 { d(1) } else { d(200) });
    let s = size_with_cost(&l, &input(), &cost).unwrap();
    // Estimated qty 94 -> cost 200 -> 1140 > 1000; steps down to 80: 800 + max(1, 60) = 860.
    assert_eq!((s.qty, s.risk_at_stop), (80, d(800)));
    assert!(s.risk_at_stop + d(60) <= s.budget);
}

#[test]
fn production_floor_sizes_a_normal_equity_entry_with_a_small_real_cost() {
    // Real config/risk.toml; the floor and percentages are read from it, never set here.
    let l = GateConfig::from_toml_str(include_str!("../../../config/risk.toml"))
        .unwrap()
        .effective(&Baseline::none());
    let floor = l.min_charges_slippage_buffer;
    let cost = |q: u32| Tri::Known(Decimal::new(i64::from(q) * 5, 1)); // Rs 0.50 a share, under the floor
    let mut i = input();
    i.equity = d(100_000);
    // Rs 500 entry, Rs 10 stop distance: budget Rs 1,000; the cap (25% -> 50 shares) binds.
    let s = size_with_cost(&l, &i, &cost).unwrap();
    assert_eq!((s.qty, s.risk_at_stop, s.notional), (50, d(500), d(25_000)));
    assert_eq!(s.budget, d(100_000) * l.max_risk_per_trade_pct / d(100));
    // Wide cap so the floor binds: (budget - floor) / 10 = 94 shares, risk Rs 940 (raw cost would
    // give 95); with the production floor of Rs 60 this is the spec's 94 / Rs 940.
    let mut wide = l.clone();
    wide.max_position_value_pct = d(100);
    wide.max_notional_per_order_inr = d(1_000_000); // G16 not under test here
    let s = size_with_cost(&wide, &i, &cost).unwrap();
    let want = ((s.budget - floor) / d(10)).floor();
    assert_eq!((Decimal::from(s.qty), s.risk_at_stop), (want, want * d(10)));
    assert_eq!((s.qty, s.risk_at_stop), (94, d(940)));
}

#[test]
fn a_floor_of_zero_or_less_fails_closed() {
    for f in [d(0), d(-1)] {
        let mut l = limits();
        l.min_charges_slippage_buffer = f;
        assert!(matches!(
            size(&l, &input()),
            Err(SizingError::InputUnknown(_))
        ));
    }
}

proptest! {
    #[test]
    fn allowance_is_never_below_the_floor_or_over_the_budget(cost_paise in 0i64..50_000) {
        let mut l = limits();
        l.max_position_value_pct = d(100);
        let c = Decimal::new(cost_paise, 2);
        if let Ok(s) = size_with_cost(&l, &input(), &|_| Tri::Known(c)) {
            prop_assert!(s.risk_at_stop + c.max(d(60)) <= s.budget);
            // One more share at the larger allowance would not have fit.
            let more = s.risk_at_stop + d(10);
            prop_assert!(more + c.max(d(60)) > s.budget);
        }
    }
}

#[test]
fn flat_and_closure_paths_agree() {
    // Backtest passes a closure; the flat `size` wraps the same function: same input, same Sized.
    let l = limits();
    let a = size(&l, &input()).unwrap();
    let b = size_with_cost(&l, &input(), &|_| input().buffer).unwrap();
    assert_eq!(a, b);
}

#[test]
fn cost_that_eats_the_budget_is_no_trade() {
    let l = limits();
    let huge = |q: u32| Tri::Known(d(60) + d(i64::from(q)) * d(1000));
    assert_eq!(
        size_with_cost(&l, &input(), &huge),
        Err(SizingError::BudgetBelowOneLot)
    );
}

#[test]
fn prices_off_the_tick_are_rejected() {
    let mut i = input();
    i.stop = Decimal::new(48997, 2); // 489.97 on a 0.05 tick
    assert_eq!(size(&limits(), &i), Err(SizingError::PriceOffTick));
    let mut i = input();
    i.entry = Decimal::new(50003, 2);
    assert_eq!(size(&limits(), &i), Err(SizingError::PriceOffTick));
    let mut i = input();
    i.entry = Decimal::new(51235, 2); // 512.35
    i.stop = Decimal::new(50810, 2); // 508.10
    let s = size(&limits(), &i).unwrap();
    assert_eq!(s.risk_at_stop, d(i64::from(s.qty)) * Decimal::new(425, 2));
    assert!(s.risk_at_stop + d(60) <= s.budget);
}

proptest! {
    #[test]
    fn paise_prices_both_sides_stay_within_budget(
        eq in 10_000i64..10_000_000,
        entry_p in 5_000i64..500_000, // paise, tick 0.05 => multiples of 5
        dist_p in 1i64..100,
        sell in any::<bool>(),
        lot in prop::sample::select(vec![1u32, 5, 25, 75]),
        over in any::<bool>(),
    ) {
        let l = limits();
        let entry = Decimal::new(entry_p * 5, 2);
        let dist = Decimal::new(dist_p * 5, 2);
        prop_assume!(dist < entry);
        let mut i = input();
        i.equity = d(eq);
        i.entry = entry;
        i.stop = if sell { entry + dist } else { entry - dist };
        i.side = if sell { Side::Sell } else { Side::Buy };
        i.lot_size = lot;
        i.overridden = over;
        let cost = |q: u32| Tri::Known(l.min_charges_slippage_buffer + Decimal::new(i64::from(q), 1));
        if let Ok(s) = size_with_cost(&l, &i, &cost) {
            prop_assert_eq!(s.qty % lot, 0);
            let c = l.min_charges_slippage_buffer + Decimal::new(i64::from(s.qty), 1);
            prop_assert!(s.risk_at_stop + c <= s.budget);
            prop_assert!(s.notional <= d(eq) * l.max_position_value_pct / d(100));
        }
    }
}

fn cost60(_q: u32) -> Tri<Decimal> {
    Tri::Known(d(60))
}

/// Limits where the order cap (not G14) is the binding one: budget allows 94, G14 allows 250.
fn order_cap_limits(cap: i64) -> EffectiveLimits {
    let mut l = limits();
    l.max_position_value_pct = d(100);
    l.max_notional_per_order_inr = d(cap);
    l
}

#[test]
fn order_cap_binds_and_every_moving_field_follows() {
    // Uncapped: qty 94 (Rs 940 risk + 60 buffer = budget 1,000).
    let l = order_cap_limits(1_000_000);
    let free = size_with_reference(&l, &input(), &cost60, d(500)).unwrap();
    assert_eq!((free.qty, free.capped_by_order_cap), (94, false));
    // Cap Rs 30,000 at 500: 60 units.
    let mut tight = l.clone();
    tight.max_notional_per_order_inr = d(30_000);
    let s = size_with_reference(&tight, &input(), &cost60, d(500)).unwrap();
    assert_eq!(s.qty, 60);
    assert_eq!(s.lots, 60);
    assert_eq!(s.risk_at_stop, d(600));
    assert_eq!(s.budget, d(1000));
    assert_eq!(s.notional, d(30_000));
    assert!(s.capped_by_order_cap && !s.capped_by_position_cap && !s.reduced_by_override);
    // size_with_cost delegates with reference = entry: identical.
    assert_eq!(size_with_cost(&tight, &input(), &cost60).unwrap(), s);
}

#[test]
fn close_above_entry_shrinks_qty_and_matches_the_gate_reference() {
    let l = order_cap_limits(30_000);
    let at_entry = size_with_reference(&l, &input(), &cost60, d(500)).unwrap();
    let above = size_with_reference(&l, &input(), &cost60, d(600)).unwrap();
    assert_eq!((at_entry.qty, above.qty), (60, 50));
    assert!(d(600) * Decimal::from(above.qty) <= l.max_notional_per_order_inr);
    // A reference below entry never loosens: the entry is the floor.
    let below = size_with_reference(&l, &input(), &cost60, d(400)).unwrap();
    assert_eq!(below, at_entry);
}

#[test]
fn order_cap_equal_to_the_position_cap_is_reported_as_the_position_cap() {
    // G14 25% of 100,000 = 25,000 = the order cap: no extra reduction from the order cap.
    let mut l = limits();
    l.max_notional_per_order_inr = d(25_000);
    let s = size_with_reference(&l, &input(), &cost60, d(500)).unwrap();
    assert_eq!(s.qty, 50);
    assert!(s.capped_by_position_cap && !s.capped_by_order_cap);
}

#[test]
fn one_lot_above_the_order_cap_is_no_trade() {
    let mut l = order_cap_limits(400);
    assert_eq!(
        size_with_reference(&l, &input(), &cost60, d(500)),
        Err(SizingError::OrderCapBelowOneLot)
    );
    l.max_notional_per_order_inr = Decimal::ZERO;
    assert_eq!(
        size_with_reference(&l, &input(), &cost60, d(500)),
        Err(SizingError::OrderCapBelowOneLot)
    );
    // Lot of 50 at 500 = 25,000 per lot (inside the risk budget), above a 20,000 cap.
    let mut inp = input();
    inp.lot_size = 50;
    assert_eq!(
        size_with_reference(&order_cap_limits(20_000), &inp, &cost60, d(500)),
        Err(SizingError::OrderCapBelowOneLot)
    );
}

#[test]
fn bad_reference_cap_or_overflow_is_a_typed_error() {
    let l = order_cap_limits(30_000);
    for r in [Decimal::ZERO, d(-5)] {
        assert_eq!(
            size_with_reference(&l, &input(), &cost60, r),
            Err(SizingError::BadPrice)
        );
    }
    let mut neg = l.clone();
    neg.max_notional_per_order_inr = d(-1);
    assert_eq!(
        size_with_reference(&neg, &input(), &cost60, d(500)),
        Err(SizingError::Arithmetic)
    );
    // Reference at Decimal::MAX: cap 30,000 / MAX floors to 0 lots, no trade (typed, no panic).
    assert_eq!(
        size_with_reference(&l, &input(), &cost60, Decimal::MAX),
        Err(SizingError::OrderCapBelowOneLot)
    );
    // Cap at Decimal::MAX, entry 0.05: MAX / 0.05 overflows, which means "not binding" (saturates),
    // so the size is the one without the cap.
    let mut huge = l.clone();
    huge.max_notional_per_order_inr = Decimal::MAX;
    let mut tiny = input();
    tiny.entry = Decimal::new(5, 2);
    tiny.stop = Decimal::new(4, 2);
    tiny.tick_size = Decimal::new(1, 2);
    let capped = size_with_reference(&huge, &tiny, &cost60, Decimal::new(5, 2)).unwrap();
    assert!(!capped.capped_by_order_cap);
    assert_eq!(Ok(capped), size_with_cost(&l_wide(), &tiny, &cost60));
    // A negative cap that overflows the division is still a typed error (never saturates).
    let mut neg_huge = l;
    neg_huge.max_notional_per_order_inr = Decimal::MIN;
    assert_eq!(
        size_with_reference(&neg_huge, &tiny, &cost60, Decimal::new(5, 2)),
        Err(SizingError::Arithmetic)
    );
    // A tiny reference never loosens: the entry is the floor, so the result equals size_with_cost.
    assert_eq!(
        size_with_reference(&huge, &input(), &cost60, Decimal::new(1, 28)),
        size_with_cost(&huge, &input(), &cost60)
    );
}

/// Limits whose caps cannot bind (G16 not under test).
fn l_wide() -> EffectiveLimits {
    let mut l = order_cap_limits(1_000_000);
    l.max_notional_per_order_inr = Decimal::MAX;
    l
}
