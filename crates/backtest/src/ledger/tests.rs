use std::str::FromStr;

use bazaar_core::config::{ChargeExchange, ChargesConfig, parse_str};
use bazaar_core::money::Qty;
use bazaar_core::segment::Segment;
use chrono::TimeZone;
use chrono_tz::Asia::Kolkata;

use super::*;
use crate::types::{HistoryPolicy, Leg, LegInstrument, Trade};

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn px(s: &str) -> Price {
    Price::snap_to_tick(d(s), d("0.05")).unwrap()
}

fn row(seg: &str, side: &str, brokerage: &str, stt: &str) -> String {
    format!(
        "[[rate_set]]\nexchange = \"NSE\"\nsegment = \"{seg}\"\nside = \"{side}\"\n\
         effective_from = \"2026-01-01\"\nverified_on = \"2026-01-01\"\nsource = \"test\"\n\
         turnover_basis = \"notional\"\nbrokerage = {brokerage}\nstt_ctt_pct = \"{stt}\"\n\
         exchange_txn_pct = \"0\"\nsebi_per_crore = \"0\"\nipft_pct = \"0\"\nstamp_pct = \"0\"\n\n"
    )
}

/// Rates owned by these tests: delivery STT 0.1% and no brokerage; intraday flat Rs 20 and
/// sell STT 0.025%; no exchange, SEBI or stamp lines; GST 18%; DP 15.34 incl. GST.
fn cfg() -> ChargesConfig {
    let zero = "{ kind = \"zero\" }";
    let flat = "{ kind = \"flat\", flat = \"20\" }";
    let mut t = String::from(
        "schema_version = 1\nverified_on = \"2026-06-01\"\nsource = \"test\"\n\n\
         [meta]\nbroker = \"t\"\ncurrency = \"INR\"\nmax_age_days = 90\n\
         history_policy = \"effective_dated\"\n\n\
         [gst]\nrate_pct = \"18\"\nverified_on = \"2026-06-01\"\nsource = \"test\"\n\n\
         [dp]\nper_scrip_total = \"15.34\"\nincludes_gst = true\nverified_on = \"2026-06-01\"\n\
         source = \"test\"\n\n",
    );
    t += &row("equity_delivery", "buy", zero, "0.1");
    t += &row("equity_delivery", "sell", zero, "0.1");
    t += &row("equity_intraday", "buy", flat, "0");
    t += &row("equity_intraday", "sell", flat, "0.025");
    parse_str(&t).unwrap()
}

fn day(dd: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 6, dd).unwrap()
}

fn fill(token: u32, product: Product, side: LegSide, price: &str, qty: u32, dd: u32) -> Fill {
    Fill {
        leg: Leg {
            instrument_token: token,
            symbol: format!("S{token}"),
            segment: Segment::Equity,
            instrument: LegInstrument::Cash,
            product,
            side,
            qty: Qty(qty),
            price: px(price),
            trade_date: day(dd),
        },
        kind: crate::types::FillKind::Entry,
        bar_time: Kolkata.with_ymd_and_hms(2026, 6, dd, 9, 15, 0).unwrap(),
        reference_price: px(price),
        slippage_per_unit: Decimal::ZERO,
        charges: ChargeBreakdown::default(),
    }
}

fn ledger(cfg: &ChargesConfig) -> Ledger<'_> {
    Ledger::new(
        CostModel::new(cfg, HistoryPolicy::Strict, ChargeExchange::Nse),
        d("100000"),
    )
}

fn at(dd: u32) -> DateTime<Tz> {
    Kolkata.with_ymd_and_hms(2026, 6, dd, 15, 30, 0).unwrap()
}

#[test]
fn delivery_long_round_trip_pays_dp_once_and_nets_everything() {
    // Buy 100 @ 500, sell 100 @ 510: gross 1000. STT 50 (buy) + 51 (sell) + DP 15.34 = 116.34.
    // Net 883.66; risk |500 - 490| x 100 = 1000, so R = 0.88366 -> 0.8837 (4 dp).
    let cfg = cfg();
    let mut l = ledger(&cfg);
    let buy = fill(1, Product::Delivery, LegSide::Buy, "500", 100, 1);
    assert!(l.apply("s", buy, Some(px("490"))).unwrap().is_none());
    let sell = fill(1, Product::Delivery, LegSide::Sell, "510", 100, 2);
    let t = l.apply("s", sell, None).unwrap().unwrap();
    assert_eq!(t.gross_pnl, d("1000"));
    assert_eq!(t.charges.stt, d("101"));
    assert_eq!(t.charges.dp, d("15.34"));
    assert_eq!(t.charges.total().unwrap(), d("116.34"));
    assert_eq!(t.net_pnl, d("883.66"));
    assert_eq!(t.risk_inr, d("1000"));
    assert_eq!(t.r_multiple, Some(d("0.8837")));
    assert_eq!(l.realised_net(), d("883.66"));
    assert!(l.open_position(1).is_none());
}

#[test]
fn dp_is_once_per_scrip_per_day_and_again_next_day() {
    // Round trip of 10 @ 100 each way: STT 1 + 1. First sell of the day carries DP 15.34.
    let cfg = cfg();
    let mut l = ledger(&cfg);
    let trip = |l: &mut Ledger<'_>, token, dd| {
        l.apply(
            "s",
            fill(token, Product::Delivery, LegSide::Buy, "100", 10, dd),
            None,
        )
        .unwrap();
        l.apply(
            "s",
            fill(token, Product::Delivery, LegSide::Sell, "100", 10, dd),
            None,
        )
        .unwrap()
        .unwrap()
    };
    assert_eq!(trip(&mut l, 1, 3).net_pnl, d("-17.34"));
    assert_eq!(trip(&mut l, 1, 3).net_pnl, d("-2"));
    assert_eq!(trip(&mut l, 2, 3).net_pnl, d("-17.34"));
    assert_eq!(trip(&mut l, 1, 4).net_pnl, d("-17.34"));
    assert_eq!(l.realised_net(), d("-54.02"));
}

#[test]
fn intraday_short_round_trip_by_hand() {
    // Short 100 @ 510, cover @ 500: gross 1000. Sell leg: brokerage 20, STT 0.025% x 51000 =
    // 12.75 -> 13, GST 18% x 20 = 3.60. Buy leg: brokerage 20, GST 3.60. Charges 60.20.
    // Net 939.80; risk |510 - 520| x 100 = 1000, R 0.9398. No DP on intraday.
    let cfg = cfg();
    let mut l = ledger(&cfg);
    l.apply(
        "s",
        fill(1, Product::Intraday, LegSide::Sell, "510", 100, 5),
        Some(px("520")),
    )
    .unwrap();
    let t = l
        .apply(
            "s",
            fill(1, Product::Intraday, LegSide::Buy, "500", 100, 5),
            None,
        )
        .unwrap()
        .unwrap();
    assert_eq!(t.gross_pnl, d("1000"));
    assert_eq!(t.charges.total().unwrap(), d("60.20"));
    assert_eq!(t.charges.dp, d("0"));
    assert_eq!(t.net_pnl, d("939.80"));
    assert_eq!(t.r_multiple, Some(d("0.9398")));
}

#[test]
fn short_loss_is_negative_and_r_is_negative() {
    // Short 10 @ 100, cover @ 105: gross -50. Charges: 20+3.6 each leg + STT 0.025% x 1000 =
    // 0.25 -> 0 = 47.20. Net -97.20; risk |100 - 105| x 10 = 50; R -1.944 (4 dp: -1.944).
    let cfg = cfg();
    let mut l = ledger(&cfg);
    l.apply(
        "s",
        fill(1, Product::Intraday, LegSide::Sell, "100", 10, 5),
        Some(px("105")),
    )
    .unwrap();
    let t = l
        .apply(
            "s",
            fill(1, Product::Intraday, LegSide::Buy, "105", 10, 5),
            None,
        )
        .unwrap()
        .unwrap();
    assert_eq!(t.gross_pnl, d("-50"));
    assert_eq!(t.net_pnl, d("-97.20"));
    assert_eq!(t.r_multiple, Some(d("-1.944")));
}

#[test]
fn zero_risk_gives_no_r_multiple() {
    let cfg = cfg();
    let mut l = ledger(&cfg);
    l.apply(
        "s",
        fill(1, Product::Intraday, LegSide::Buy, "100", 10, 5),
        None,
    )
    .unwrap();
    let t = l
        .apply(
            "s",
            fill(1, Product::Intraday, LegSide::Sell, "101", 10, 5),
            None,
        )
        .unwrap()
        .unwrap();
    assert_eq!(t.risk_inr, d("0"));
    assert_eq!(t.r_multiple, None);
}

#[test]
fn square_off_closes_only_intraday_positions_in_token_order() {
    // Intraday long 10 @ 100 squared off @ 99: gross -10. Charges 20+3.6 (buy) + 20+3.6 and STT
    // 0.025% x 990 = 0.2475 -> 0 (sell) = 47.20. Net -57.20. The delivery position stays open.
    let cfg = cfg();
    let mut l = ledger(&cfg);
    l.apply(
        "s",
        fill(2, Product::Intraday, LegSide::Buy, "100", 10, 6),
        None,
    )
    .unwrap();
    l.apply(
        "s",
        fill(1, Product::Intraday, LegSide::Buy, "100", 10, 6),
        None,
    )
    .unwrap();
    l.apply(
        "s",
        fill(3, Product::Delivery, LegSide::Buy, "100", 10, 6),
        None,
    )
    .unwrap();
    let trades = l
        .square_off_intraday(day(6), |p| {
            Ok(fill(
                p.entry.leg.instrument_token,
                Product::Intraday,
                LegSide::Sell,
                "99",
                10,
                6,
            ))
        })
        .unwrap();
    assert_eq!(trades.len(), 2);
    assert_eq!(trades[0].entry.leg.instrument_token, 1);
    assert_eq!(trades[1].entry.leg.instrument_token, 2);
    assert_eq!(trades[0].net_pnl, d("-57.20"));
    assert!(l.open_position(3).is_some());
    assert!(l.open_position(1).is_none());
}

#[test]
fn square_off_skips_positions_entered_after_the_date() {
    let cfg = cfg();
    let mut l = ledger(&cfg);
    l.apply(
        "s",
        fill(1, Product::Intraday, LegSide::Buy, "100", 10, 8),
        None,
    )
    .unwrap();
    let trades = l
        .square_off_intraday(day(5), |_| {
            Err(BacktestError::Invalid("must not be called".into()))
        })
        .unwrap();
    assert_eq!(trades, Vec::<Trade>::new());
    assert!(l.open_position(1).is_some());
}

#[test]
fn square_off_propagates_the_builder_error() {
    let cfg = cfg();
    let mut l = ledger(&cfg);
    l.apply(
        "s",
        fill(1, Product::Intraday, LegSide::Buy, "100", 10, 6),
        None,
    )
    .unwrap();
    let r = l.square_off_intraday(day(6), |_| Err(BacktestError::Invalid("no bar".into())));
    assert_eq!(r.unwrap_err(), BacktestError::Invalid("no bar".into()));
    assert!(l.open_position(1).is_some());
}

#[test]
fn same_side_partial_and_product_mismatch_are_invalid_and_leave_state_alone() {
    let cfg = cfg();
    let mut l = ledger(&cfg);
    l.apply(
        "s",
        fill(1, Product::Delivery, LegSide::Buy, "100", 10, 1),
        None,
    )
    .unwrap();
    let same = fill(1, Product::Delivery, LegSide::Buy, "100", 10, 1);
    assert!(matches!(
        l.apply("s", same, None),
        Err(BacktestError::Invalid(_))
    ));
    let partial = fill(1, Product::Delivery, LegSide::Sell, "100", 5, 2);
    assert!(matches!(
        l.apply("s", partial, None),
        Err(BacktestError::Invalid(_))
    ));
    let other = fill(1, Product::Intraday, LegSide::Sell, "100", 10, 2);
    assert!(matches!(
        l.apply("s", other, None),
        Err(BacktestError::Invalid(_))
    ));
    assert!(l.open_position(1).is_some());
    assert_eq!(l.trades(), &[] as &[Trade]);
}

#[test]
fn a_failed_close_does_not_consume_the_dp_slot() {
    // The sell is costed (DP booked) but the realised total overflows: the slot is released, so
    // the retry on the same day still pays DP.
    let cfg = cfg();
    let mut l = ledger(&cfg);
    l.apply(
        "s",
        fill(1, Product::Delivery, LegSide::Buy, "100", 10, 1),
        None,
    )
    .unwrap();
    l.realised = Decimal::MAX;
    let sell = fill(1, Product::Delivery, LegSide::Sell, "110", 10, 1);
    assert!(matches!(
        l.apply("s", sell, None),
        Err(BacktestError::Overflow(_))
    ));
    assert!(l.open_position(1).is_some());
    l.realised = Decimal::ZERO;
    let t = l
        .apply(
            "s",
            fill(1, Product::Delivery, LegSide::Sell, "110", 10, 1),
            None,
        )
        .unwrap()
        .unwrap();
    assert_eq!(t.charges.dp, d("15.34"));
}

#[test]
fn wrong_side_or_equal_stops_are_invalid() {
    let cfg = cfg();
    let mut l = ledger(&cfg);
    for stop in ["100", "105"] {
        let buy = fill(1, Product::Intraday, LegSide::Buy, "100", 10, 5);
        assert!(matches!(
            l.apply("s", buy, Some(px(stop))),
            Err(BacktestError::Invalid(_))
        ));
    }
    for stop in ["100", "95"] {
        let sell = fill(1, Product::Intraday, LegSide::Sell, "100", 10, 5);
        assert!(matches!(
            l.apply("s", sell, Some(px(stop))),
            Err(BacktestError::Invalid(_))
        ));
    }
    assert!(l.open_position(1).is_none());
}

#[test]
fn opening_delivery_sell_is_invalid() {
    let cfg = cfg();
    let mut l = ledger(&cfg);
    let sell = fill(1, Product::Delivery, LegSide::Sell, "100", 10, 5);
    assert!(matches!(
        l.apply("s", sell, None),
        Err(BacktestError::Invalid(_))
    ));
    assert!(l.open_position(1).is_none());
}

#[test]
fn exit_before_entry_is_invalid() {
    let cfg = cfg();
    let mut l = ledger(&cfg);
    l.apply(
        "s",
        fill(1, Product::Intraday, LegSide::Buy, "100", 10, 5),
        None,
    )
    .unwrap();
    let earlier_day = fill(1, Product::Intraday, LegSide::Sell, "100", 10, 4);
    assert!(matches!(
        l.apply("s", earlier_day, None),
        Err(BacktestError::Invalid(_))
    ));
    let mut earlier_bar = fill(1, Product::Intraday, LegSide::Sell, "100", 10, 5);
    earlier_bar.bar_time = Kolkata.with_ymd_and_hms(2026, 6, 5, 9, 0, 0).unwrap();
    assert!(matches!(
        l.apply("s", earlier_bar, None),
        Err(BacktestError::Invalid(_))
    ));
    assert!(l.open_position(1).is_some());
    assert_eq!(l.trades(), &[] as &[Trade]);
}

#[test]
fn mark_rejects_time_going_backwards() {
    let cfg = cfg();
    let mut l = ledger(&cfg);
    l.mark(at(2), &BTreeMap::new()).unwrap();
    assert!(matches!(
        l.mark(at(1), &BTreeMap::new()),
        Err(BacktestError::Invalid(_))
    ));
    assert_eq!(l.equity_curve().len(), 1);
}

#[test]
fn mark_overflow_is_an_error_and_leaves_state_alone() {
    let cfg = cfg();
    let mut l = Ledger::new(
        CostModel::new(&cfg, HistoryPolicy::Strict, ChargeExchange::Nse),
        Decimal::MAX,
    );
    l.apply(
        "s",
        fill(1, Product::Intraday, LegSide::Buy, "100", 10, 5),
        None,
    )
    .unwrap();
    let marks = BTreeMap::from([(1_u32, px("105"))]);
    assert!(matches!(
        l.mark(at(5), &marks),
        Err(BacktestError::Overflow(_))
    ));
    assert_eq!(l.equity_curve(), &[] as &[EquityPoint]);
    assert_eq!(l.open_position(1).unwrap().last_mark, px("100"));
}

#[test]
fn drawdown_overflow_is_an_error() {
    let c = vec![EquityPoint {
        time: at(1),
        equity: Decimal::MIN,
    }];
    assert!(matches!(
        max_drawdown(Decimal::MAX, &c),
        Err(BacktestError::Overflow(_))
    ));
}

#[test]
fn drawdown_is_measured_from_starting_capital() {
    // Capital 100000, equity only ever falls: 98000 then 97000 -> fall 3000, 3%.
    let dd = max_drawdown(d("100000"), &curve(&["98000", "97000"])).unwrap();
    assert_eq!(dd.max_abs, d("3000"));
    assert_eq!(dd.max_pct, d("0.03"));
}

#[test]
fn non_positive_peak_skips_the_percent() {
    let dd = max_drawdown(d("0"), &curve(&["-10"])).unwrap();
    assert_eq!(dd.max_abs, d("10"));
    assert_eq!(dd.max_pct, d("0"));
}

proptest::proptest! {
    #[test]
    fn flat_equity_equals_capital_plus_realised(
        trips in proptest::collection::vec((1_u32..500, 1_u32..500, 1_u32..20, proptest::bool::ANY), 1..4)
    ) {
        let cfg = cfg();
        let mut l = ledger(&cfg);
        for (entry, exit, qty, long) in trips {
            let (open, close) = if long { (LegSide::Buy, LegSide::Sell) } else { (LegSide::Sell, LegSide::Buy) };
            l.apply("s", fill(1, Product::Intraday, open, &entry.to_string(), qty, 5), None).unwrap();
            l.apply("s", fill(1, Product::Intraday, close, &exit.to_string(), qty, 5), None).unwrap();
            let eq = l.mark(at(5), &BTreeMap::new()).unwrap();
            proptest::prop_assert_eq!(eq, d("100000") + l.realised_net());
        }
        let dd = l.drawdown().unwrap();
        proptest::prop_assert!(dd.max_abs >= Decimal::ZERO);
        proptest::prop_assert!(dd.max_pct >= Decimal::ZERO && dd.max_pct <= Decimal::ONE);
    }
}

#[test]
fn equity_includes_open_entry_charges_and_unrealised_gross() {
    // Capital 100000; long 100 @ 500 delivery open (entry STT 50); mark 505: +500 unrealised.
    let cfg = cfg();
    let mut l = ledger(&cfg);
    l.apply(
        "s",
        fill(1, Product::Delivery, LegSide::Buy, "500", 100, 1),
        None,
    )
    .unwrap();
    let marks = BTreeMap::from([(1_u32, px("505"))]);
    assert_eq!(l.mark(at(1), &marks).unwrap(), d("100450"));
    // No mark: the last mark (505) is carried forward.
    assert_eq!(l.mark(at(2), &BTreeMap::new()).unwrap(), d("100450"));
    // A short marked down gains.
    l.apply(
        "s",
        fill(2, Product::Intraday, LegSide::Sell, "100", 10, 3),
        None,
    )
    .unwrap();
    let marks = BTreeMap::from([(1_u32, px("500")), (2_u32, px("98"))]);
    // 100000 - 50 (delivery STT) - 23.60 (intraday sell: 20 + 3.6, STT 0.25 -> 0) + 20 gain.
    assert_eq!(l.mark(at(3), &marks).unwrap(), d("99946.40"));
    assert_eq!(l.equity_curve().len(), 3);
}

fn curve(values: &[&str]) -> Vec<EquityPoint> {
    values
        .iter()
        .enumerate()
        .map(|(i, v)| EquityPoint {
            time: at(u32::try_from(i).unwrap() + 1),
            equity: d(v),
        })
        .collect()
}

#[test]
fn drawdown_on_a_known_curve() {
    // Peaks 100 -> 110 -> 120. Falls: 11 (110 -> 99), 20 (110 -> 90, 20/110 = 0.181818).
    let dd = max_drawdown(d("100"), &curve(&["100", "110", "99", "105", "90", "120"])).unwrap();
    assert_eq!(dd.max_abs, d("20"));
    assert_eq!(dd.max_pct, d("0.181818"));
}

#[test]
fn drawdown_abs_and_pct_can_peak_at_different_points() {
    // 1000 -> 60 is a fall of 940 (94%); 200 -> 180 is 20 (10%).
    let dd = max_drawdown(d("1000"), &curve(&["1000", "900", "100", "60"])).unwrap();
    assert_eq!(dd.max_abs, d("940"));
    assert_eq!(dd.max_pct, d("0.94"));
    let dd = max_drawdown(d("100"), &curve(&["100", "90", "200", "180"])).unwrap();
    assert_eq!(dd.max_abs, d("20"));
    assert_eq!(dd.max_pct, d("0.1"));
}

#[test]
fn drawdown_of_empty_or_rising_curve_is_zero() {
    let zero = Drawdown {
        max_abs: d("0"),
        max_pct: d("0"),
    };
    assert_eq!(max_drawdown(d("100"), &[]).unwrap(), zero);
    assert_eq!(
        max_drawdown(d("1"), &curve(&["1", "2", "3"])).unwrap(),
        zero
    );
}

#[test]
fn ledger_drawdown_uses_the_recorded_curve() {
    let cfg = cfg();
    let mut l = ledger(&cfg);
    l.apply(
        "s",
        fill(1, Product::Delivery, LegSide::Buy, "500", 100, 1),
        None,
    )
    .unwrap();
    l.mark(at(1), &BTreeMap::from([(1_u32, px("510"))]))
        .unwrap(); // 100950
    l.mark(at(2), &BTreeMap::from([(1_u32, px("490"))]))
        .unwrap(); // 98950
    let dd = l.drawdown().unwrap();
    assert_eq!(dd.max_abs, d("2000"));
}

#[test]
fn overflowing_fill_is_an_error_not_a_panic() {
    let cfg = cfg();
    let mut l = ledger(&cfg);
    let mut big = fill(1, Product::Intraday, LegSide::Buy, "100", 10, 1);
    big.leg.price = Price::snap_to_tick(Decimal::MAX, d("1")).unwrap();
    big.leg.qty = Qty(u32::MAX);
    assert!(l.apply("s", big, None).is_err());
    assert!(l.open_position(1).is_none());
}
