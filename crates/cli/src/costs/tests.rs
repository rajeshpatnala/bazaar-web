//! Reference values are derived by hand from the fixture `tests/fixtures/charges.toml` (a copy
//! of the NSE equity rows of `config/charges.toml`, with `source` and `verified_on`); no rate
//! appears in this file. Each value was derived by hand with the F9 rounding rules (half-up per
//! line) and is asserted exactly, so the stated tolerance is 0; those rules reproduce the owner's
//! calculator samples within Rs 0.05 per line.
#![allow(clippy::unwrap_used, clippy::panic)]

use bazaar_backtest::types::ChargeBreakdown;
use bazaar_core::config::{ChargeSegment, parse_str};
use bazaar_core::testutil::FakeClock;
use chrono::{NaiveDate, TimeZone};
use chrono_tz::Asia::Kolkata;

use super::*;

const FIXTURE: &str = include_str!("../../tests/fixtures/charges.toml");

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn cfg() -> ChargesConfig {
    parse_str(FIXTURE).unwrap()
}

fn clock() -> Arc<dyn Clock> {
    Arc::new(FakeClock::at(
        Kolkata.with_ymd_and_hms(2026, 12, 1, 11, 0, 0).unwrap(),
    ))
}

fn f9(c: ChargesConfig) -> F9Charges {
    F9Charges::new(Arc::new(c), clock()).unwrap()
}

fn gate(c: ChargesConfig, bps: u32) -> GateCost {
    GateCost::new(Arc::new(c), clock(), bps).unwrap()
}

/// Per-line breakdown straight from the F9 model, to check every field that moves.
fn lines(
    c: &ChargesConfig,
    side: LegSide,
    product: Product,
    qty: u32,
    price: &str,
) -> ChargeBreakdown {
    let tick = Decimal::new(1, d(price).scale());
    let leg = Leg {
        instrument_token: 0,
        symbol: "TEST".into(),
        segment: Segment::Equity,
        instrument: LegInstrument::Cash,
        product,
        side,
        qty: Qty(qty),
        price: Price::snap_to_tick(d(price), tick).unwrap(),
        trade_date: NaiveDate::from_ymd_opt(2026, 12, 1).unwrap(),
    };
    F9CostModel::new(c, HistoryPolicy::Strict, ChargeExchange::Nse)
        .charges_with_dp(&mut DpLedger::new(), &leg)
        .unwrap()
}

/// Order: brokerage, stt, `exchange_txn`, gst, sebi, stamp, dp.
fn eq_lines(b: &ChargeBreakdown, want: [&str; 7]) {
    let got = [
        b.brokerage,
        b.stt,
        b.exchange_txn,
        b.gst,
        b.sebi,
        b.stamp,
        b.dp,
    ];
    for (g, w) in got.iter().zip(want) {
        assert_eq!(*g, d(w), "line mismatch in {b:?}");
    }
}

#[test]
fn cnc_sell_10_at_1000_every_line_and_total() {
    let c = cfg();
    let b = lines(&c, LegSide::Sell, Product::Delivery, 10, "1000");
    eq_lines(&b, ["0", "10", "0.31", "0.06", "0.01", "0", "15.34"]);
    let got = f9(c).charge("TEST", false, 10, d("1000"), "cnc").unwrap();
    assert_eq!(got, d("25.72"));
    assert_eq!(got, b.total().unwrap());
}

#[test]
fn cnc_buy_10_at_1000_has_stamp_and_no_dp() {
    let c = cfg();
    let b = lines(&c, LegSide::Buy, Product::Delivery, 10, "1000");
    eq_lines(&b, ["0", "10", "0.31", "0.06", "0.01", "2", "0"]);
    assert_eq!(
        f9(c).charge("TEST", true, 10, d("1000"), "cnc").unwrap(),
        d("12.38")
    );
}

#[test]
fn mis_sell_100_at_500_brokerage_is_the_lower_of_flat_and_pct() {
    let c = cfg();
    let b = lines(&c, LegSide::Sell, Product::Intraday, 100, "500");
    eq_lines(&b, ["15", "13", "1.54", "2.99", "0.05", "0", "0"]);
    assert_eq!(
        f9(c).charge("TEST", false, 100, d("500"), "mis").unwrap(),
        d("32.58")
    );
}

#[test]
fn mis_buy_100_at_500_every_line() {
    let c = cfg();
    let b = lines(&c, LegSide::Buy, Product::Intraday, 100, "500");
    eq_lines(&b, ["15", "0", "1.54", "2.99", "0.05", "2", "0"]);
    assert_eq!(
        f9(c).charge("TEST", true, 100, d("500"), "mis").unwrap(),
        d("21.58")
    );
}

#[test]
fn a_second_cnc_sell_of_the_same_scrip_pays_dp_again_never_less() {
    let f = f9(cfg());
    let a = f.charge("TEST", false, 10, d("1000"), "cnc").unwrap();
    let b = f.charge("TEST", false, 10, d("1000"), "cnc").unwrap();
    assert_eq!(a, b);
    assert_eq!(b, d("25.72"));
}

#[test]
fn price_with_paise_keeps_its_exact_turnover() {
    // 10 x 1000.37 = 10003.70: STT 10.0037 -> 10, exchange txn 0.3071 -> 0.31.
    let b = lines(&cfg(), LegSide::Sell, Product::Delivery, 10, "1000.37");
    assert_eq!(b.stt, d("10"));
    assert_eq!(b.exchange_txn, d("0.31"));
    // The adapter total equals the model's own total for that exact price.
    assert_eq!(
        f9(cfg())
            .charge("TEST", false, 10, d("1000.37"), "cnc")
            .unwrap(),
        b.total().unwrap()
    );
}

#[test]
fn f9_refuses_bad_inputs_as_unavailable() {
    let f = f9(cfg());
    let bad = [
        f.charge("TEST", true, 0, d("100"), "mis"),
        f.charge("TEST", true, 1, d("0"), "mis"),
        f.charge("TEST", true, 1, d("-5"), "mis"),
        f.charge("TEST", true, 1, d("100"), "nrml"),
        f.charge("TEST", true, 1, d("100"), ""),
        f.charge("", true, 1, d("100"), "mis"),
        f.charge("NI FTY", true, 1, d("100"), "mis"),
        f.charge("TEST", true, u32::MAX, Decimal::MAX, "mis"),
    ];
    for r in bad {
        assert_eq!(r, Err(SourceError::Unavailable));
    }
}

#[test]
fn missing_rate_row_gst_or_dp_is_unavailable_never_zero() {
    let strip_row = |seg: ChargeSegment| {
        let mut c = cfg();
        c.rate_set.retain(|r| r.segment != seg);
        c
    };
    let no_delivery = f9(strip_row(ChargeSegment::EquityDelivery));
    assert_eq!(
        no_delivery.charge("TEST", false, 10, d("1000"), "cnc"),
        Err(SourceError::Unavailable)
    );
    // The other product is unaffected.
    assert!(
        no_delivery
            .charge("TEST", false, 10, d("1000"), "mis")
            .is_ok()
    );
    let no_intraday = f9(strip_row(ChargeSegment::EquityIntraday));
    assert_eq!(
        no_intraday.charge("TEST", false, 10, d("1000"), "mis"),
        Err(SourceError::Unavailable)
    );
    let mut c = cfg();
    c.gst = None;
    assert_eq!(
        f9(c).charge("TEST", true, 10, d("1000"), "mis"),
        Err(SourceError::Unavailable)
    );
    let mut c = cfg();
    c.dp = None;
    let f = f9(c);
    assert_eq!(
        f.charge("TEST", false, 10, d("1000"), "cnc"),
        Err(SourceError::Unavailable)
    );
    // DP is only needed on a CNC sell.
    assert!(f.charge("TEST", true, 10, d("1000"), "cnc").is_ok());
}

#[test]
fn no_rate_set_in_force_on_the_trade_date_is_unavailable() {
    // Clock before every effective_from: strict policy finds no set.
    let early = Arc::new(FakeClock::at(
        Kolkata.with_ymd_and_hms(2020, 1, 1, 11, 0, 0).unwrap(),
    ));
    let f = F9Charges::new(Arc::new(cfg()), early).unwrap();
    assert_eq!(
        f.charge("TEST", true, 10, d("1000"), "mis"),
        Err(SourceError::Unavailable)
    );
}

#[test]
fn trade_date_is_the_ist_date_of_the_injected_clock() {
    // 00:30 IST on the rate set's `effective_from` is still the previous day in UTC: a UTC date
    // would find no rate set in force and fail.
    let late = Arc::new(FakeClock::at(
        Kolkata.with_ymd_and_hms(2026, 11, 20, 0, 30, 0).unwrap(),
    ));
    let f = F9Charges::new(Arc::new(cfg()), late).unwrap();
    assert_eq!(
        f.charge("TEST", true, 10, d("1000"), "cnc").unwrap(),
        d("12.38")
    );
}

#[test]
fn placeholder_config_fails_at_construction() {
    let mut c = cfg();
    c.placeholder = true;
    let c = Arc::new(c);
    assert_eq!(
        F9Charges::new(c.clone(), clock()).unwrap_err(),
        CostsError::Placeholder
    );
    assert_eq!(
        GateCost::new(c, clock(), 5).unwrap_err(),
        CostsError::Placeholder
    );
}

#[test]
fn gate_cost_is_the_worse_product_plus_slippage_sell() {
    // CNC sell 100 @ 500: 50 + 1.54 + 0.29 + 0.05 + 15.34 = 67.22; 5 bps of 50000 = 25.
    let c = cfg();
    let g = gate(c.clone(), 5);
    assert_eq!(g.cost(Side::Sell, 100, d("500")), Tri::Known(d("92.22")));
    let f = f9(c);
    assert_eq!(
        f.charge("T", false, 100, d("500"), "cnc").unwrap(),
        d("67.22")
    );
    assert_eq!(
        f.charge("T", false, 100, d("500"), "mis").unwrap(),
        d("32.58")
    );
}

#[test]
fn gate_cost_buy_with_and_without_slippage() {
    // CNC buy 100 @ 500: 50 + 8 + 1.54 + 0.29 + 0.05 = 59.88 (> MIS buy 21.58).
    let c = cfg();
    assert_eq!(
        gate(c.clone(), 0).cost(Side::Buy, 100, d("500")),
        Tri::Known(d("59.88"))
    );
    assert_eq!(
        gate(c, 5).cost(Side::Buy, 100, d("500")),
        Tri::Known(d("84.88"))
    );
}

#[test]
fn gate_cost_never_below_either_exact_charge() {
    let c = cfg();
    let g = gate(c.clone(), 0);
    let f = f9(c);
    for (side, buy) in [(Side::Buy, true), (Side::Sell, false)] {
        for (qty, px) in [(10_u32, "100"), (7, "99.35"), (500, "2500.5")] {
            let Tri::Known(got) = g.cost(side, qty, d(px)) else {
                panic!("known expected");
            };
            for p in ["mis", "cnc"] {
                assert!(got >= f.charge("T", buy, qty, d(px), p).unwrap());
            }
        }
    }
}

#[test]
fn gate_cost_is_unknown_on_every_config_gap_never_known_zero() {
    let mut no_gst = cfg();
    no_gst.gst = None;
    let mut no_dp = cfg();
    no_dp.dp = None;
    let mut no_cnc = cfg();
    no_cnc
        .rate_set
        .retain(|r| r.segment != ChargeSegment::EquityDelivery);
    let mut no_mis = cfg();
    no_mis
        .rate_set
        .retain(|r| r.segment != ChargeSegment::EquityIntraday);
    for c in [no_gst, no_cnc, no_mis] {
        assert_eq!(gate(c, 5).cost(Side::Buy, 10, d("100")), Tri::Unknown);
    }
    // DP missing matters for a sell (CNC) only.
    assert_eq!(
        gate(no_dp.clone(), 5).cost(Side::Sell, 10, d("100")),
        Tri::Unknown
    );
    assert!(matches!(
        gate(no_dp, 5).cost(Side::Buy, 10, d("100")),
        Tri::Known(_)
    ));
}

#[test]
fn gate_cost_unknown_on_bad_inputs_and_overflow() {
    let g = gate(cfg(), 5);
    assert_eq!(g.cost(Side::Buy, 0, d("100")), Tri::Unknown);
    assert_eq!(g.cost(Side::Buy, 1, d("0")), Tri::Unknown);
    assert_eq!(g.cost(Side::Sell, 1, d("-1")), Tri::Unknown);
    assert_eq!(g.cost(Side::Buy, u32::MAX, Decimal::MAX), Tri::Unknown);
}

#[test]
fn gate_cost_unknown_when_slippage_arithmetic_overflows() {
    let g = gate(cfg(), u32::MAX);
    assert_eq!(
        g.cost(Side::Buy, u32::MAX, d("10000000000000000")),
        Tri::Unknown
    );
}

#[test]
fn a_trade_whose_charges_all_round_to_zero_is_refused_not_free() {
    // 1 x Rs 10: every line rounds to 0.00; a zero charge must never reach the paper account.
    assert_eq!(
        f9(cfg()).charge("TEST", true, 1, d("10"), "mis"),
        Err(SourceError::Unavailable)
    );
    assert_eq!(gate(cfg(), 0).cost(Side::Buy, 1, d("10")), Tri::Unknown);
}

#[test]
fn fixture_rows_match_config_charges_toml() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../config/charges.toml");
    let real: ChargesConfig = parse_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let fix = cfg();
    let nse_eq = |c: &ChargesConfig| {
        c.rate_set
            .iter()
            .filter(|r| {
                r.exchange == ChargeExchange::Nse
                    && matches!(
                        r.segment,
                        ChargeSegment::EquityDelivery | ChargeSegment::EquityIntraday
                    )
            })
            .map(|r| format!("{r:?}"))
            .collect::<Vec<_>>()
    };
    assert_eq!(nse_eq(&fix).len(), 4);
    assert_eq!(nse_eq(&fix), nse_eq(&real), "fixture drifted from config");
    assert_eq!(format!("{:?}", fix.gst), format!("{:?}", real.gst));
    assert_eq!(format!("{:?}", fix.dp), format!("{:?}", real.dp));
}
