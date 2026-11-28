use super::*;
use proptest::prelude::*;

const DAY1: &str = "2026-11-23T10:00:00+05:30";
const DAY2: &str = "2026-11-24T10:00:00+05:30";

fn d(s: &str) -> Decimal {
    s.parse().expect("decimal literal")
}

fn day(n: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, n).expect("date")
}

fn line(kind: &'static str, amount: &str, ref_id: u128) -> ReplayLine<'static> {
    ReplayLine {
        session: 1,
        kind,
        amount: d(amount),
        ref_id: Some(ref_id),
        ts_ist_rfc3339: DAY1,
        symbol: None,
        qty: None,
        product: None,
    }
}

/// A buy or sell line of INFY CNC.
fn trade(kind: &'static str, qty: u32, amount: &str, ref_id: u128) -> ReplayLine<'static> {
    ReplayLine {
        symbol: Some("INFY"),
        qty: Some(qty),
        product: Some("cnc"),
        ..line(kind, amount, ref_id)
    }
}

/// A charge and its trade, as the account writes them.
fn pair(
    kind: &'static str,
    qty: u32,
    amount: &str,
    charge: &str,
    ref_id: u128,
) -> [ReplayLine<'static>; 2] {
    [
        line("charge", charge, ref_id),
        trade(kind, qty, amount, ref_id),
    ]
}

fn run(lines: &[ReplayLine<'_>]) -> Result<Replayed, ReplayError> {
    replay_pools(lines.iter().copied(), day(24))
}

fn key(session: u32, symbol: &str, product: Product) -> PoolKey {
    PoolKey {
        session: PaperSessionId::new(session).expect("session"),
        symbol: symbol.to_owned(),
        product,
    }
}

fn infy() -> PoolKey {
    key(1, "INFY", Product::Cnc)
}

fn figures(p: &Pool) -> (u32, Decimal, Decimal) {
    (p.qty(), p.cost(), p.unsold_charges())
}

fn split(cost_out: &str, charge_out: &str, result: &str) -> SaleSplit {
    SaleSplit {
        cost_out: d(cost_out),
        charge_out: d(charge_out),
        result: d(result),
    }
}

#[test]
fn buy_day_one_sell_lower_day_two_is_that_days_loss() {
    let mut lines = vec![line("opening", "100000", 0)];
    lines.extend(pair("buy", 10, "-1000", "-10", 1));
    let sell = pair("sell", 10, "900", "-6", 2);
    lines.extend(sell.map(|l| ReplayLine {
        ts_ist_rfc3339: DAY2,
        ..l
    }));
    let out = run(&lines).expect("replay");
    assert_eq!(
        out.sales,
        vec![Sale {
            key: infy(),
            date: day(24),
            split: split("1000", "10", "-116"),
        }]
    );
    assert_eq!(out.pools, HashMap::from([(infy(), Pool::default())]));
}

#[test]
fn the_owner_worked_case_holds_through_replay() {
    let mut lines = vec![line("opening", "100000", 0)];
    lines.extend(pair("buy", 10, "-1000", "-10", 1));
    lines.extend(pair("buy", 10, "-800", "-10", 2));
    lines.extend(pair("sell", 10, "900", "-6", 3));
    let out = run(&lines).expect("replay");
    assert_eq!(out.sales.len(), 1);
    assert_eq!(out.sales[0].split, split("900", "10", "-16"));
    let pool = out.pools[&infy()];
    assert_eq!(figures(&pool), (10, d("900"), d("10")));
    assert_eq!(pool.open_loss(d("90")), Ok(d("-10")));
    assert_eq!(pool.open_loss(d("100")), Ok(Decimal::ZERO));

    lines.extend(pair("sell", 10, "932", "-6", 4));
    let out = run(&lines).expect("replay");
    assert_eq!(out.sales[1].split, split("900", "10", "16"));
    assert_eq!(out.pools[&infy()], Pool::default());
}

#[test]
fn a_three_way_split_replays_to_exactly_zero() {
    let mut lines = pair("buy", 3, "-100.00", "-10.00", 1).to_vec();
    for ref_id in 2..5 {
        lines.extend(pair("sell", 1, "50", "-1", ref_id));
    }
    let out = run(&lines).expect("replay");
    let splits: Vec<_> = out.sales.iter().map(|s| s.split).collect();
    assert_eq!(
        splits,
        vec![
            split("33.34", "3.34", "12.32"),
            split("33.33", "3.33", "12.34"),
            split("33.33", "3.33", "12.34"),
        ]
    );
    assert_eq!(out.pools[&infy()], Pool::default());
}

#[test]
fn sessions_symbols_and_products_never_share_a_pool() {
    let keys = [
        (1, "INFY", "cnc", Product::Cnc),
        (1, "INFY", "mis", Product::Mis),
        (1, "INFY", "nrml", Product::Nrml),
        (1, "TCS", "cnc", Product::Cnc),
        (2, "INFY", "cnc", Product::Cnc),
    ];
    let mut lines = Vec::new();
    for ((session, symbol, product, _), ref_id) in keys.iter().zip(1u32..) {
        let [c, t] = pair("buy", ref_id, "-100", "-1", ref_id.into());
        lines.push(ReplayLine {
            session: *session,
            ..c
        });
        lines.push(ReplayLine {
            session: *session,
            symbol: Some(symbol),
            product: Some(product),
            ..t
        });
    }
    let out = run(&lines).expect("replay");
    assert_eq!(out.pools.len(), keys.len());
    for ((session, symbol, _, product), qty) in keys.iter().zip(1u32..) {
        assert_eq!(out.pools[&key(*session, symbol, *product)].qty(), qty);
    }
    // A sale cannot take another pool's shares.
    let [c, t] = pair("sell", 2, "100", "-1", 9);
    lines.extend([c, t]);
    assert_eq!(
        run(&lines),
        Err(ReplayError {
            line: lines.len() - 1,
            fault: LineFault::Pool(PoolError::SellAboveHeld),
        })
    );
}

#[test]
fn lines_apply_in_the_order_given_never_by_timestamp() {
    let [c1, buy] = pair("buy", 1, "-100", "-1", 1);
    let [c2, sell] = pair("sell", 1, "90", "-1", 2);
    // The later lines carry the earlier date (a clock that stepped back).
    let later = |l| ReplayLine {
        ts_ist_rfc3339: DAY2,
        ..l
    };
    let out = run(&[later(c1), later(buy), c2, sell]).expect("replay");
    assert_eq!(out.sales[0].date, day(23));
    assert_eq!(
        run(&[c2, sell, later(c1), later(buy)]),
        Err(ReplayError {
            line: 1,
            fault: LineFault::Pool(PoolError::SellAboveHeld),
        })
    );
}

#[test]
fn the_sale_is_dated_by_its_sell_line() {
    let mut lines = pair("buy", 1, "-100", "-1", 1).to_vec();
    let [charge, sell] = pair("sell", 1, "90", "-1", 2);
    let sell = ReplayLine {
        ts_ist_rfc3339: "2026-11-24T00:00:00+05:30",
        ..sell
    };
    let charge = ReplayLine {
        ts_ist_rfc3339: "2026-11-23T23:59:59+05:30",
        ..charge
    };
    lines.extend([charge, sell]);
    assert_eq!(run(&lines).expect("replay").sales[0].date, day(24));
}

#[test]
fn an_empty_ledger_and_an_opening_alone_have_no_pools_or_sales() {
    assert_eq!(run(&[]), Ok(Replayed::default()));
    assert_eq!(
        run(&[line("opening", "100000", 0)]),
        Ok(Replayed::default())
    );
}

fn refused(lines: &[ReplayLine<'_>], at: usize, fault: LineFault) {
    assert_eq!(run(lines), Err(ReplayError { line: at, fault }), "{fault}");
}

#[test]
fn every_broken_pairing_is_refused_by_name() {
    let [charge, buy] = pair("buy", 1, "-100", "-1", 7);
    let opening = line("opening", "100000", 0);
    refused(&[charge], 0, LineFault::OrphanCharge);
    let [c2, b2] = pair("buy", 1, "-100", "-1", 8);
    // A trailing orphan after a good pair, and a charge cut off by an opening.
    refused(&[c2, b2, charge], 2, LineFault::OrphanCharge);
    refused(&[charge, opening, buy], 0, LineFault::OrphanCharge);
    refused(&[charge, charge, buy], 1, LineFault::TwoCharges);
    refused(&[buy], 0, LineFault::MissingCharge);
    refused(&[charge, buy, buy], 2, LineFault::MissingCharge);
    refused(&[buy, charge], 0, LineFault::MissingCharge);
    for (charge_ref, trade_ref) in [(Some(7), Some(8)), (None, Some(7)), (Some(7), None)] {
        let lines = [
            ReplayLine {
                ref_id: charge_ref,
                ..charge
            },
            ReplayLine {
                ref_id: trade_ref,
                ..buy
            },
        ];
        refused(&lines, 1, LineFault::PairRefId);
    }
    let both_none = [charge, buy].map(|l| ReplayLine { ref_id: None, ..l });
    refused(&both_none, 1, LineFault::PairRefId);
    refused(
        &[charge, ReplayLine { session: 2, ..buy }],
        1,
        LineFault::PairSession,
    );
}

#[test]
fn a_wrong_sign_or_zero_amount_is_refused_on_every_kind() {
    let [charge, buy] = pair("buy", 1, "-100", "-1", 1);
    for bad in ["1", "0", "-0.0000"] {
        let c = ReplayLine {
            amount: d(bad),
            ..charge
        };
        refused(&[c, buy], 0, LineFault::WrongSign);
    }
    for bad in ["100", "0"] {
        let b = ReplayLine {
            amount: d(bad),
            ..buy
        };
        refused(&[charge, b], 1, LineFault::WrongSign);
    }
    for bad in ["-100", "0"] {
        let [c, s] = pair("sell", 1, bad, "-1", 2);
        refused(&[charge, buy, c, s], 3, LineFault::WrongSign);
        refused(&[line("opening", bad, 0)], 0, LineFault::WrongSign);
    }
}

#[test]
fn a_bad_line_is_refused_by_name() {
    let [charge, buy] = pair("buy", 1, "-100", "-1", 1);
    let on_trade: [(ReplayLine<'_>, LineFault); 8] = [
        (
            ReplayLine {
                symbol: None,
                ..buy
            },
            LineFault::MissingSymbol,
        ),
        (
            ReplayLine {
                symbol: Some("  "),
                ..buy
            },
            LineFault::MissingSymbol,
        ),
        (ReplayLine { qty: None, ..buy }, LineFault::MissingQty),
        (
            ReplayLine {
                qty: Some(0),
                ..buy
            },
            LineFault::MissingQty,
        ),
        (
            ReplayLine {
                product: None,
                ..buy
            },
            LineFault::MissingProduct,
        ),
        (
            ReplayLine {
                product: Some("CNC"),
                ..buy
            },
            LineFault::UnknownProduct,
        ),
        (
            ReplayLine {
                kind: "dividend",
                ..buy
            },
            LineFault::UnknownKind,
        ),
        (ReplayLine { session: 0, ..buy }, LineFault::BadSession),
    ];
    for (bad, fault) in on_trade {
        refused(&[charge, bad], 1, fault);
    }
}

#[test]
fn a_bad_or_future_date_is_refused_on_every_kind() {
    let opening = line("opening", "100000", 0);
    let [charge, buy] = pair("buy", 1, "-100", "-1", 1);
    for (ts, fault) in [
        ("", LineFault::BadDate),
        ("2026-11-24", LineFault::BadDate),
        ("2026-02-30T10:00:00+05:30", LineFault::BadDate),
        ("2026-11-24T10:00:00Z", LineFault::BadDate),
        ("2026-11-24T10:00:00+00:00", LineFault::BadDate),
        ("2026-11-25T00:00:00+05:30", LineFault::FutureDate),
    ] {
        let at = |l| ReplayLine {
            ts_ist_rfc3339: ts,
            ..l
        };
        refused(&[at(opening)], 0, fault);
        refused(&[at(charge), buy], 0, fault);
        refused(&[charge, at(buy)], 1, fault);
    }
    // Any time on today's date is not in the future.
    let late = [charge, buy].map(|l| ReplayLine {
        ts_ist_rfc3339: "2026-11-24T23:59:59+05:30",
        ..l
    });
    assert!(run(&late).is_ok());
}

#[test]
fn a_sale_above_what_is_held_fails_the_whole_replay() {
    let mut lines = pair("buy", 5, "-500", "-1", 1).to_vec();
    lines.extend(pair("sell", 6, "600", "-1", 2));
    refused(&lines, 3, LineFault::Pool(PoolError::SellAboveHeld));
}

#[test]
fn bounds_are_the_stated_figures() {
    assert_eq!(MAX_LINE_AMOUNT, d("100000000000"));
    assert_eq!(MAX_POOL_AMOUNT, d("1000000000000"));
    assert_eq!((MAX_SCALE, MAX_POOL_QTY), (4, 100_000_000));
}

#[test]
fn an_amount_finer_than_the_bound_is_refused_never_rounded() {
    let [charge, buy] = pair("buy", 1, "-100", "-1", 1);
    for fine in ["-1.00001", "-0.0000000000000000000000000001"] {
        let c = ReplayLine {
            amount: d(fine),
            ..charge
        };
        refused(&[c, buy], 0, LineFault::TooPrecise);
    }
    let b = ReplayLine {
        amount: d("-100.00001"),
        ..buy
    };
    refused(&[charge, b], 1, LineFault::TooPrecise);
    refused(
        &[line("opening", "100000.00001", 0)],
        0,
        LineFault::TooPrecise,
    );
    // The bound itself, and trailing zeros beyond it, are fine.
    let ok = [
        ReplayLine {
            amount: d("-1.0001"),
            ..charge
        },
        ReplayLine {
            amount: d("-100.000000000000"),
            ..buy
        },
    ];
    let out = run(&ok).expect("replay");
    assert_eq!(figures(&out.pools[&infy()]), (1, d("100"), d("1.0001")));
}

#[test]
fn an_amount_above_the_line_bound_is_refused() {
    let [charge, buy] = pair("buy", 1, "-100000000000", "-100000000000", 1);
    assert!(run(&[charge, buy]).is_ok());
    let over = |l| ReplayLine {
        amount: d("-100000000000.0001"),
        ..l
    };
    refused(&[over(charge), buy], 0, LineFault::AmountTooLarge);
    refused(&[charge, over(buy)], 1, LineFault::AmountTooLarge);
    let huge = ReplayLine {
        amount: Decimal::MAX,
        ..line("opening", "1", 0)
    };
    refused(&[huge], 0, LineFault::AmountTooLarge);
}

#[test]
fn a_pool_above_its_bounds_is_refused() {
    // Quantity: at the bound, then one more share.
    let mut lines = pair("buy", MAX_POOL_QTY, "-1", "-1", 1).to_vec();
    assert!(run(&lines).is_ok());
    lines.extend(pair("buy", 1, "-1", "-1", 2));
    refused(&lines, 3, LineFault::PoolTooLarge);

    // Cost, then charges: ten lines at the line bound reach the pool bound.
    for (amount, charge) in [("-100000000000", "-1"), ("-1", "-100000000000")] {
        let mut lines: Vec<_> = (0..10)
            .flat_map(|r| pair("buy", 1, amount, charge, r))
            .collect();
        assert!(run(&lines).is_ok());
        lines.extend(pair("buy", 1, amount, charge, 10));
        refused(&lines, 21, LineFault::PoolTooLarge);
    }
}

/// One generated trade: pool picked by `slot`, a buy or a sale of part of what is held.
#[derive(Debug, Clone, Copy)]
struct Op {
    slot: usize,
    sell: bool,
    qty: u32,
    amount: u32,
    charge: u32,
}

fn op() -> impl Strategy<Value = Op> {
    (
        0usize..8,
        any::<bool>(),
        1u32..60,
        1u32..100_000_000,
        1u32..1_000_000,
    )
        .prop_map(|(slot, sell, qty, amount, charge)| Op {
            slot,
            sell,
            qty,
            amount,
            charge,
        })
}

/// Two sessions x two symbols x two products.
fn slot_of(slot: usize) -> (u32, &'static str, Product) {
    (
        if slot & 4 == 0 { 1 } else { 2 },
        if slot & 2 == 0 { "INFY" } else { "TCS" },
        if slot & 1 == 0 {
            Product::Cnc
        } else {
            Product::Mis
        },
    )
}

proptest! {
    /// Over any mix of buys and sales, per pool: the sum of sale results equals the cash flow
    /// plus the cost held plus the unsold charges, and a pool at quantity 0 holds nothing.
    #[test]
    fn no_money_is_lost_or_counted_twice(ops in proptest::collection::vec(op(), 1..60)) {
        let mut held = [0u32; 8];
        let mut flow = [Decimal::ZERO; 8];
        let mut used = [false; 8];
        let mut sales = 0;
        let mut rows = Vec::new();
        for (ref_id, o) in (0u128..).zip(&ops) {
            let sell = o.sell && held[o.slot] > 0;
            let qty = if sell { 1 + (o.qty - 1) % held[o.slot] } else { o.qty };
            if sell {
                held[o.slot] -= qty;
                sales += 1;
            } else {
                held[o.slot] += qty;
            }
            // Four decimals: finer than the paisa that shares round to.
            let money = Decimal::new(i64::from(o.amount), 4);
            let money = if sell { money } else { -money };
            let charge = -Decimal::new(i64::from(o.charge), 4);
            flow[o.slot] += money + charge;
            used[o.slot] = true;
            rows.push((ref_id, o.slot, sell, qty, money, charge));
        }
        let lines = rows.iter().flat_map(|&(ref_id, slot, sell, qty, money, charge)| {
            let (session, symbol, product) = slot_of(slot);
            let c = ReplayLine { session, amount: charge, ..line("charge", "-1", ref_id) };
            let t = ReplayLine {
                session,
                kind: if sell { "sell" } else { "buy" },
                amount: money,
                symbol: Some(symbol),
                qty: Some(qty),
                product: Some(product.as_str()),
                ..line("buy", "-1", ref_id)
            };
            [c, t]
        });
        let out = replay_pools(lines, day(24)).expect("replay");
        prop_assert_eq!(out.sales.len(), sales);
        prop_assert_eq!(out.pools.len(), used.iter().filter(|u| **u).count());
        for slot in (0..8).filter(|s| used[*s]) {
            let (session, symbol, product) = slot_of(slot);
            let k = key(session, symbol, product);
            let pool = out.pools[&k];
            let results: Decimal =
                out.sales.iter().filter(|s| s.key == k).map(|s| s.split.result).sum();
            prop_assert_eq!(results, flow[slot] + pool.cost() + pool.unsold_charges());
            prop_assert_eq!(pool.qty(), held[slot]);
            if pool.qty() == 0 {
                prop_assert_eq!(pool, Pool::default());
            } else {
                prop_assert!(pool.cost() >= Decimal::ZERO);
                prop_assert!(pool.unsold_charges() >= Decimal::ZERO);
            }
        }
    }

    /// At the largest pool the bounds allow, the paisa share is still exactly the ceiling of
    /// `cost * sold / held` (checked in integers), and nothing is rounded away.
    #[test]
    fn at_the_bounds_the_share_lands_on_the_right_paisa(
        held in (MAX_POOL_QTY - 1_000)..=MAX_POOL_QTY, sold in 1u32..MAX_POOL_QTY - 1_000,
        short in 0i64..1_000_000,
    ) {
        // Ten lines that add up to just under the pool bound, in units of 0.0001.
        let tenth: i64 = 1_000_000_000_000_000;
        let last = tenth - short;
        let mut lines = Vec::new();
        for r in 0..10u32 {
            let units = if r == 9 { last } else { tenth };
            let qty = if r == 9 { held - 9 } else { 1 };
            let [c, t] = pair("buy", qty, "-1", "-1", r.into());
            lines.push(c);
            lines.push(ReplayLine { amount: -Decimal::new(units, 4), ..t });
        }
        lines.extend(pair("sell", sold, "1", "-1", 10));
        let out = run(&lines).expect("replay");
        let total = 9 * i128::from(tenth) + i128::from(last);
        // One paisa is 100 units.
        let per_paisa = i128::from(held) * 100;
        let paise = (total * i128::from(sold) + per_paisa - 1) / per_paisa;
        let cost_out = Decimal::from_i128_with_scale(paise, 2);
        prop_assert_eq!(out.sales[0].split.cost_out, cost_out);
        prop_assert_eq!(
            out.pools[&infy()].cost() + cost_out,
            Decimal::from_i128_with_scale(total, 4)
        );
    }
}
