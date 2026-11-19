//! Sizing tests: spec example, caps, lots, floors, equity basis, rejects and properties.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use bazaar_risk::config::{Baseline, EffectiveLimits, GateConfig};
use bazaar_risk::sizing::{Sized, SizingError, SizingInput, size, size_with_cost, sizing_equity};
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
        case(&|i| i.buffer = d(59).into_known()),
        Err(SizingError::BufferBelowFloor)
    );
    assert_eq!(
        case(&|i| i.buffer = d(-5).into_known()),
        Err(SizingError::BufferBelowFloor)
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
        SizingError::BufferBelowFloor,
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
fn cost_below_floor_or_unknown_at_the_final_qty_rejects() {
    let l = limits();
    let low = |q: u32| Tri::Known(if q > 1 { d(10) } else { d(60) });
    assert_eq!(
        size_with_cost(&l, &input(), &low),
        Err(SizingError::BufferBelowFloor)
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
