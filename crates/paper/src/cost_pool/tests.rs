use super::*;
use proptest::prelude::*;

fn d(s: &str) -> Decimal {
    s.parse().expect("decimal literal")
}

fn pool(qty: u32, cost: &str, charges: &str) -> Pool {
    Pool {
        qty,
        cost: d(cost),
        charges: d(charges),
    }
}

fn split(cost_out: &str, charge_out: &str, result: &str) -> SaleSplit {
    SaleSplit {
        cost_out: d(cost_out),
        charge_out: d(charge_out),
        result: d(result),
    }
}

/// `qty` shares bought for `cost` with `charge`, on top of `p`.
fn bought(p: &mut Pool, qty: u32, cost: &str, charge: &str) {
    p.buy(qty, d(cost), d(charge)).expect("buy");
}

#[test]
fn a_lower_sale_is_a_loss_not_its_proceeds() {
    let mut p = Pool::default();
    bought(&mut p, 10, "1000", "10");
    assert_eq!(
        p.sell(10, d("900"), d("6")),
        Ok(split("1000", "10", "-116"))
    );
    assert_eq!(p, pool(0, "0", "0"));
}

/// Buys 10@100 and 10@80 (charge 10 each), sell 10@90 (charge 6).
#[test]
fn owner_worked_case() {
    let mut p = Pool::default();
    bought(&mut p, 10, "1000", "10");
    bought(&mut p, 10, "800", "10");
    assert_eq!(p.sell(10, d("900"), d("6")), Ok(split("900", "10", "-16")));
    assert_eq!((p.qty(), p.cost()), (10, d("900")));
    assert_eq!(p.unsold_charges(), d("10"));
    assert_eq!(p.open_loss(d("90")), Ok(d("-10")));
    // A gain is not credited, so it cannot cancel the -16.
    assert_eq!(p.open_loss(d("100")), Ok(Decimal::ZERO));
    // At a mark of 90 the day figure is realised plus open loss.
    assert_eq!(
        p.open_loss(d("90")).map(|open| d("-16") + open),
        Ok(d("-26"))
    );
    // Break-even: the rest at 93.2 with a further 6 of sale charge.
    let exit = p.sell(10, d("932"), d("6")).expect("full exit");
    assert_eq!(exit, split("900", "10", "16"));
    assert_eq!(d("-16") + exit.result, Decimal::ZERO);
    assert_eq!(p, pool(0, "0", "0"));
}

/// The rest stops losing after the sale: the figure ends under 2 paise lower, never higher.
#[test]
fn a_partial_sale_that_ends_the_loss_never_raises_the_figure() {
    for (cost, charges, mark, sale, figures) in [
        (
            "100.01",
            "10.01",
            "36.67",
            split("66.68", "6.68", "-0.02"),
            ("-0.01", "-0.02"),
        ),
        // Amounts finer than a paisa.
        (
            "100.001",
            "10.001",
            "36.667",
            split("66.67", "6.67", "-0.006"),
            ("-0.001", "-0.006"),
        ),
    ] {
        let mut p = pool(3, cost, charges);
        let before = p.open_loss(d(mark)).expect("open loss");
        let s = p
            .sell(2, d(mark) * Decimal::from(2u32), Decimal::ZERO)
            .expect("sell");
        assert_eq!(s, sale);
        assert!(d(mark) >= p.cost() + p.unsold_charges(), "rest not losing");
        assert_eq!(p.open_loss(d(mark)), Ok(Decimal::ZERO));
        let after = s.result;
        assert_eq!((before, after), (d(figures.0), d(figures.1)));
        assert!(before >= after && before - after < paise(2));
    }
}

#[test]
fn unsold_buy_charges_count_at_an_unchanged_mark() {
    assert_eq!(pool(10, "1000", "10").open_loss(d("100")), Ok(d("-10")));
}

#[test]
fn three_way_split_rounds_up_and_leaves_exactly_zero() {
    let mut p = Pool::default();
    bought(&mut p, 3, "100.00", "10.00");
    for expected in [
        split("33.34", "3.34", "12.32"),
        split("33.33", "3.33", "12.34"),
        split("33.33", "3.33", "12.34"),
    ] {
        assert_eq!(p.sell(1, d("50"), d("1")), Ok(expected));
    }
    assert_eq!(p, pool(0, "0", "0"));
}

#[test]
fn a_new_buy_after_a_full_exit_starts_a_new_average() {
    let mut p = pool(10, "1000", "10");
    p.sell(10, d("500"), d("5")).expect("sell");
    bought(&mut p, 5, "250", "4");
    assert_eq!(p, pool(5, "250", "4"));
}

#[test]
fn a_refused_trade_names_why_and_leaves_the_pool_unchanged() {
    let start = pool(10, "1000", "10");
    for (qty, amount, charge, expected) in [
        (0, "1", "1", PoolError::MissingQty),
        (1, "-1", "1", PoolError::WrongSign),
        (1, "1", "-1", PoolError::WrongSign),
        // A zero buy cost or zero sale proceeds is a missing figure.
        (1, "0", "1", PoolError::ZeroAmount),
        (1, "0", "0", PoolError::ZeroAmount),
    ] {
        let mut p = start;
        assert_eq!(p.buy(qty, d(amount), d(charge)), Err(expected));
        assert_eq!(p.sell(qty, d(amount), d(charge)), Err(expected));
        assert_eq!(p, start);
    }
    for (mut p, qty) in [(start, 11), (Pool::default(), 1)] {
        let before = p;
        assert_eq!(p.sell(qty, d("1"), d("1")), Err(PoolError::SellAboveHeld));
        assert_eq!(p, before);
    }
}

#[test]
fn a_share_never_exceeds_what_is_left() {
    // Charges finer than a paisa: rounding up must not take more than there is.
    let mut p = pool(2, "0.001", "0.004");
    assert_eq!(
        p.sell(1, d("1"), d("0")),
        Ok(split("0.001", "0.004", "0.995"))
    );
    assert_eq!(p, pool(1, "0", "0"));
}

#[test]
fn overflow_is_a_typed_error_and_leaves_the_pool_unchanged() {
    let full = Pool {
        qty: 1,
        cost: Decimal::MAX,
        charges: Decimal::ONE,
    };
    for (qty, cost, charge) in [(u32::MAX, "1", "1"), (1, "1", "1")] {
        let mut p = full;
        assert_eq!(p.buy(qty, d(cost), d(charge)), Err(PoolError::Overflow));
        assert_eq!(p, full);
    }
    assert_eq!(full.open_loss(Decimal::ONE), Err(PoolError::Overflow));
    let two = Pool { qty: 3, ..full };
    assert_eq!(two.open_loss(Decimal::MAX), Err(PoolError::Overflow));
    let mut p = Pool { qty: 3, ..full };
    assert_eq!(p.sell(2, d("1"), d("1")), Err(PoolError::Overflow));
    assert_eq!(p, two);
    let mut p = full;
    // Tiny proceeds, the largest charge and the largest cost: the result cannot be held.
    assert_eq!(p.sell(1, d("1"), Decimal::MAX), Err(PoolError::Overflow));
    assert_eq!(p, full);
}

#[test]
fn a_mark_that_is_not_positive_is_refused_never_zero() {
    for mark in ["0", "-1"] {
        assert_eq!(
            pool(1, "10", "1").open_loss(d(mark)),
            Err(PoolError::BadMark)
        );
    }
}

#[test]
fn paper_session_ids_start_at_one() {
    assert_eq!(PaperSessionId::new(0), None);
    let id = PaperSessionId::new(2).expect("session");
    assert_eq!((id.get(), id.to_string().as_str()), (2, "P#2"));
}

#[test]
fn selling_part_of_a_winning_pool_at_the_mark_realises_that_parts_gain() {
    let mut p = pool(10, "1000", "10");
    assert_eq!(p.open_loss(d("110")), Ok(Decimal::ZERO));
    // Gain at 110 is 90; selling half realises 45.
    assert_eq!(p.sell(5, d("550"), d("0")), Ok(split("500", "5", "45")));
    assert_eq!(p.open_loss(d("110")), Ok(Decimal::ZERO));
}

/// Paise as rupees.
fn paise(n: u32) -> Decimal {
    Decimal::new(i64::from(n), 2)
}

proptest! {
    /// A partial sale at the mark, before its sale charge, never raises a losing figure.
    /// `pick` and `level` scale the quantity sold and a mark that leaves the pool losing.
    #[test]
    fn a_partial_sale_at_the_mark_never_raises_a_losing_figure(
        held in 2u32..500, pick in 0u32..500, cost in 100_000u32..10_000_000,
        charges in 1u32..100_000, level in 1u32..1_000,
    ) {
        let mut p = Pool { qty: held, cost: paise(cost), charges: paise(charges) };
        let sold = 1 + pick % (held - 1);
        let top = u64::from((cost + charges - 1) / held);
        let mark = paise(u32::try_from(top * u64::from(level) / 999).expect("mark").max(1));
        let before = p.open_loss(mark).expect("open loss");
        prop_assert!(before < Decimal::ZERO);
        let s = p.sell(sold, mark * Decimal::from(sold), Decimal::ZERO).expect("sell");
        let rest = p.open_loss(mark).expect("open loss");
        let after = s.result + rest;
        if rest < Decimal::ZERO {
            prop_assert_eq!(after, before);
        } else {
            prop_assert!(after <= before && before - after < paise(2), "{} {}", before, after);
        }
    }

    /// A buy at the mark, before its charge and slippage, leaves a losing figure unchanged.
    #[test]
    fn a_buy_at_the_mark_leaves_a_losing_figure_unchanged(
        held in 1u32..500, more in 1u32..500, cost in 100_000u32..10_000_000, level in 1u32..1_000,
    ) {
        let mut p = Pool { qty: held, cost: paise(cost), charges: paise(100) };
        let top = u64::from(cost / held);
        let mark = paise(u32::try_from(top * u64::from(level) / 999).expect("mark").max(1));
        let before = p.open_loss(mark).expect("open loss");
        prop_assert!(before < Decimal::ZERO);
        p.buy(more, mark * Decimal::from(more), Decimal::ZERO).expect("buy");
        prop_assert_eq!(p.open_loss(mark), Ok(before));
    }

    /// However a pool is sold off, all cost and charges come out once and nothing is left.
    #[test]
    fn cost_and_charges_come_out_exactly_once(
        chunks in proptest::collection::vec(1u32..50, 1..12),
        cost in 1u32..10_000_000, charges in 1u32..100_000,
    ) {
        let total: u32 = chunks.iter().sum();
        let mut p = Pool { qty: total, cost: paise(cost), charges: paise(charges) };
        let (mut cost_out, mut charge_out) = (Decimal::ZERO, Decimal::ZERO);
        for q in chunks {
            let s = p.sell(q, Decimal::ONE, Decimal::ZERO).expect("sell");
            cost_out += s.cost_out;
            charge_out += s.charge_out;
            prop_assert!(p.cost >= Decimal::ZERO && p.charges >= Decimal::ZERO);
        }
        prop_assert_eq!(p, Pool::default());
        prop_assert_eq!((cost_out, charge_out), (paise(cost), paise(charges)));
    }
}
