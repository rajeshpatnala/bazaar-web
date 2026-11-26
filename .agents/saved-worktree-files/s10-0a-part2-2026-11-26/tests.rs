use super::*;
use proptest::prelude::*;

fn d(s: &str) -> Decimal {
    s.parse().expect("decimal literal")
}

fn day(n: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, n).expect("date")
}

/// An owned ledger line with its fill fields.
#[derive(Debug, Clone)]
struct Row {
    entry: LedgerEntry,
    symbol: Option<&'static str>,
    qty: Option<u32>,
    product: Option<&'static str>,
}

fn row(session: u32, kind: &str, amount: Decimal, ref_id: Option<u128>) -> Row {
    Row {
        entry: LedgerEntry {
            epoch: session,
            kind: kind.to_owned(),
            amount,
            ref_id,
            ts_ist_rfc3339: "2026-11-02T10:00:00+05:30".to_owned(),
        },
        symbol: None,
        qty: None,
        product: None,
    }
}

/// The charge and trade lines of one fill, as `account.rs` appends them. `amount` and
/// `charge` are written as positive figures.
fn fill(session: u32, id: u128, kind: &str, qty: u32, amount: &str, charge: &str) -> [Row; 2] {
    let amount = if kind == "buy" { -d(amount) } else { d(amount) };
    let mut trade = row(session, kind, amount, Some(id));
    (trade.symbol, trade.qty, trade.product) = (Some("AAA"), Some(qty), Some("cnc"));
    [row(session, "charge", -d(charge), Some(id)), trade]
}

fn run(rows: &[Row]) -> Result<Replay, PoolError> {
    let lines: Vec<PoolLine<'_>> = rows
        .iter()
        .map(|r| PoolLine {
            entry: &r.entry,
            symbol: r.symbol,
            qty: r.qty,
            product: r.product,
        })
        .collect();
    replay(&lines, day(25))
}

fn key(session: u32, symbol: &str, product: &'static str) -> PoolKey {
    PoolKey {
        session: PaperSessionId::new(session).expect("session"),
        symbol: symbol.to_owned(),
        product,
    }
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

#[test]
fn a_lower_sale_the_next_day_is_that_days_loss_not_its_proceeds() {
    let mut rows = fill(1, 1, "buy", 10, "1000", "10").to_vec();
    let mut sale = fill(1, 2, "sell", 10, "900", "6");
    for r in &mut sale {
        r.entry.ts_ist_rfc3339 = "2026-11-03T10:00:00+05:30".to_owned();
    }
    rows.extend(sale);
    let out = run(&rows).expect("replay");
    assert_eq!(out.sales.len(), 1);
    assert_eq!(out.sales[0].date, day(3));
    assert_eq!(out.sales[0].key, key(1, "AAA", "cnc"));
    assert_eq!(out.sales[0].split, split("1000", "10", "-116"));
    assert_eq!(out.pools[&key(1, "AAA", "cnc")], pool(0, "0", "0"));
}

/// Buys 10@100 and 10@80 (charge 10 each), sell 10@90 (charge 6).
#[test]
fn owner_worked_case() {
    let rows = [
        fill(1, 1, "buy", 10, "1000", "10"),
        fill(1, 2, "buy", 10, "800", "10"),
        fill(1, 3, "sell", 10, "900", "6"),
    ]
    .concat();
    let out = run(&rows).expect("replay");
    assert_eq!(out.sales[0].split, split("900", "10", "-16"));
    let rest = out.pools[&key(1, "AAA", "cnc")];
    assert_eq!((rest.qty(), rest.cost()), (10, d("900")));
    assert_eq!(rest.unsold_charges(), d("10"));
    assert_eq!(rest.open_loss(d("90")), Ok(d("-10")));
    // A gain is not credited, so it cannot cancel the -16.
    assert_eq!(rest.open_loss(d("100")), Ok(Decimal::ZERO));
}

#[test]
fn unsold_buy_charges_count_at_an_unchanged_mark() {
    let out = run(&fill(1, 1, "buy", 10, "1000", "10")).expect("replay");
    assert_eq!(
        out.pools[&key(1, "AAA", "cnc")].open_loss(d("100")),
        Ok(d("-10"))
    );
}

#[test]
fn three_way_split_rounds_up_and_leaves_exactly_zero() {
    let mut rows = fill(1, 1, "buy", 3, "100.00", "10.00").to_vec();
    for id in 2..5 {
        rows.extend(fill(1, id, "sell", 1, "50", "1"));
    }
    let out = run(&rows).expect("replay");
    let splits: Vec<SaleSplit> = out.sales.iter().map(|s| s.split).collect();
    assert_eq!(
        splits,
        [
            split("33.34", "3.34", "12.32"),
            split("33.33", "3.33", "12.34"),
            split("33.33", "3.33", "12.34"),
        ]
    );
    assert_eq!(out.pools[&key(1, "AAA", "cnc")], pool(0, "0", "0"));
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
fn a_new_buy_after_a_full_exit_starts_a_new_average() {
    let rows = [
        fill(1, 1, "buy", 10, "1000", "10"),
        fill(1, 2, "sell", 10, "500", "5"),
        fill(1, 3, "buy", 5, "250", "4"),
    ]
    .concat();
    let out = run(&rows).expect("replay");
    assert_eq!(out.pools[&key(1, "AAA", "cnc")], pool(5, "250", "4"));
}

#[test]
fn sessions_symbols_and_products_never_share_a_pool() {
    let base = fill(1, 1, "buy", 10, "1000", "10");
    let mut rows = base.to_vec();
    rows.extend(fill(2, 2, "buy", 1, "80", "1"));
    let mut mis = fill(1, 3, "buy", 2, "160", "2");
    mis[1].product = Some("mis");
    rows.extend(mis);
    let mut other = fill(1, 4, "buy", 3, "240", "3");
    other[1].symbol = Some("BBB");
    rows.extend(other);
    let out = run(&rows).expect("replay");
    let expected = BTreeMap::from([
        (key(1, "AAA", "cnc"), pool(10, "1000", "10")),
        (key(1, "AAA", "mis"), pool(2, "160", "2")),
        (key(1, "BBB", "cnc"), pool(3, "240", "3")),
        (key(2, "AAA", "cnc"), pool(1, "80", "1")),
    ]);
    assert_eq!(out.pools, expected);
    // The cheap pools gain at 90; the dear one still counts in full.
    assert_eq!(
        out.pools[&key(1, "AAA", "cnc")].open_loss(d("90")),
        Ok(d("-110"))
    );
    assert_eq!(
        out.pools[&key(2, "AAA", "cnc")].open_loss(d("90")),
        Ok(d("0"))
    );
    // Shares held elsewhere cannot be sold from a pool.
    for (session, symbol, product) in [(2, "AAA", "mis"), (2, "BBB", "cnc"), (3, "AAA", "cnc")] {
        let mut sale = fill(session, 9, "sell", 1, "90", "1");
        (sale[1].symbol, sale[1].product) = (Some(symbol), Some(product));
        let rows = [rows.clone(), sale.to_vec()].concat();
        assert_eq!(run(&rows), Err(PoolError::SellAboveHeld));
    }
}

#[test]
fn replay_follows_line_order_not_timestamps() {
    let mut rows = fill(1, 1, "buy", 10, "1000", "10").to_vec();
    rows.extend(fill(1, 2, "sell", 10, "900", "6"));
    // The sale is stamped before the buy; by time it would sell shares not yet held.
    for r in &mut rows[2..] {
        r.entry.ts_ist_rfc3339 = "2026-11-01T09:00:00+05:30".to_owned();
    }
    assert_eq!(run(&rows).expect("replay").sales.len(), 1);
    rows.rotate_left(2);
    assert_eq!(run(&rows), Err(PoolError::SellAboveHeld));
}

#[test]
fn opening_lines_touch_no_pool() {
    let rows = [row(1, "opening", d("100000"), None)];
    assert_eq!(run(&rows), Ok(Replay::default()));
}

#[test]
fn sell_above_held_is_refused() {
    let mut rows = fill(1, 1, "buy", 10, "1000", "10").to_vec();
    rows.extend(fill(1, 2, "sell", 11, "900", "6"));
    assert_eq!(run(&rows), Err(PoolError::SellAboveHeld));
}

/// A buy with one thing broken must be refused for that thing.
fn broken(change: impl FnOnce(&mut [Row; 2]), expected: PoolError) {
    let mut rows = fill(1, 1, "buy", 10, "1000", "10");
    change(&mut rows);
    assert_eq!(run(&rows), Err(expected), "{rows:?}");
}

#[test]
fn bad_line_fields_are_typed_errors() {
    broken(|r| r[1].symbol = None, PoolError::MissingSymbol);
    broken(|r| r[1].symbol = Some(""), PoolError::MissingSymbol);
    broken(|r| r[1].qty = None, PoolError::MissingQty);
    broken(|r| r[1].qty = Some(0), PoolError::MissingQty);
    broken(|r| r[1].product = None, PoolError::MissingProduct);
    broken(|r| r[1].product = Some("CNC"), PoolError::MissingProduct);
    broken(|r| r[1].entry.kind = "fee".into(), PoolError::UnknownKind);
    broken(|r| r[0].entry.epoch = 0, PoolError::BadSession);
    for ts in [
        "",
        "2026-11-02",
        "2026-11-02T10:00:00Z",
        "2026-13-02T10:00:00+05:30",
    ] {
        broken(
            |r| r[1].entry.ts_ist_rfc3339 = ts.into(),
            PoolError::BadDate,
        );
    }
    broken(
        |r| r[0].entry.ts_ist_rfc3339 = "2026-11-26T00:00:00+05:30".into(),
        PoolError::FutureDate,
    );
}

#[test]
fn broken_charge_pairing_is_a_typed_error() {
    broken(|r| r[1].entry.ref_id = Some(2), PoolError::RefIdMismatch);
    broken(|r| r[1].entry.ref_id = None, PoolError::RefIdMismatch);
    broken(
        |r| (r[0].entry.ref_id, r[1].entry.ref_id) = (None, None),
        PoolError::RefIdMismatch,
    );
    broken(|r| r[1].entry.epoch = 2, PoolError::SessionMismatch);
    for amount in ["10", "0", "-0"] {
        broken(|r| r[0].entry.amount = d(amount), PoolError::WrongSign);
    }
    for amount in ["1000", "0", "-0"] {
        broken(|r| r[1].entry.amount = d(amount), PoolError::WrongSign);
    }
    let [charge, buy] = fill(1, 1, "buy", 10, "1000", "10");
    let mut negative_sell = fill(1, 2, "sell", 1, "90", "1");
    negative_sell[1].entry.amount = d("-90");
    let rows = [vec![charge.clone(), buy.clone()], negative_sell.to_vec()].concat();
    assert_eq!(run(&rows), Err(PoolError::WrongSign));

    let opening = row(1, "opening", d("1"), None);
    let cases = [
        (vec![charge.clone()], PoolError::OrphanCharge),
        (
            vec![charge.clone(), opening, buy.clone()],
            PoolError::OrphanCharge,
        ),
        (
            vec![charge.clone(), charge.clone(), buy.clone()],
            PoolError::DoubleCharge,
        ),
        (vec![buy.clone()], PoolError::MissingCharge),
        (vec![charge, buy.clone(), buy], PoolError::MissingCharge),
    ];
    for (rows, expected) in cases {
        assert_eq!(run(&rows), Err(expected), "{rows:?}");
    }
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
    assert_eq!(p.sell(1, Decimal::MIN, d("1")), Err(PoolError::Overflow));
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
