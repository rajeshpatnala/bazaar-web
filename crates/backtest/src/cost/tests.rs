use std::path::Path;

use bazaar_core::config::{ChargesConfig, parse_str};
use bazaar_core::money::{Price, Qty};
use rust_decimal::Decimal;
use std::str::FromStr;

use super::*;

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn real_text() -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/charges.toml");
    std::fs::read_to_string(p).unwrap()
}

fn cfg() -> ChargesConfig {
    parse_str(&real_text()).unwrap()
}

fn date(y: i32, m: u32, dd: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, dd).unwrap()
}

fn leg(
    instrument: LegInstrument,
    product: Product,
    side: LegSide,
    price: &str,
    qty: u32,
    on: NaiveDate,
) -> Leg {
    Leg {
        instrument_token: 7,
        symbol: "TEST".into(),
        segment: if instrument == LegInstrument::Cash {
            Segment::Equity
        } else {
            Segment::EquityDerivatives
        },
        instrument,
        product,
        side,
        qty: Qty(qty),
        price: Price::snap_to_tick(d(price), d("0.05")).unwrap(),
        trade_date: on,
    }
}

const D0: (i32, u32, u32) = (2026, 11, 20);

fn day0() -> NaiveDate {
    date(D0.0, D0.1, D0.2)
}

#[test]
fn delivery_sell_worked_example() {
    // 510 x 100 = 51000: STT 0.1% = 51 (whole rupee), txn 1.5657 -> 1.57, SEBI 0.051 -> 0.05,
    // GST 18% x (0 + 1.57 + 0.05) = 0.2916 -> 0.29, stamp 0 on a sell.
    let cfg = cfg();
    let m = CostModel::new(&cfg, HistoryPolicy::Strict);
    let c = m
        .charges(&leg(
            LegInstrument::Cash,
            Product::Delivery,
            LegSide::Sell,
            "510",
            100,
            day0(),
        ))
        .unwrap();
    assert_eq!(c.brokerage, d("0"));
    assert_eq!(c.stt, d("51"));
    assert_eq!(c.exchange_txn, d("1.57"));
    assert_eq!(c.sebi, d("0.05"));
    assert_eq!(c.gst, d("0.29"));
    assert_eq!(c.stamp, d("0"));
    assert_eq!(c.dp, d("0"));
}

#[test]
fn intraday_brokerage_is_the_lower_of_flat_and_pct() {
    let cfg = cfg();
    let m = CostModel::new(&cfg, HistoryPolicy::Strict);
    // 500 x 100 = 50000: 0.03% = 15 < 20.
    let small = m
        .charges(&leg(
            LegInstrument::Cash,
            Product::Intraday,
            LegSide::Buy,
            "500",
            100,
            day0(),
        ))
        .unwrap();
    assert_eq!(small.brokerage, d("15"));
    // 5000 x 100 = 500000: 0.03% = 150 > 20, capped at the flat amount.
    let big = m
        .charges(&leg(
            LegInstrument::Cash,
            Product::Intraday,
            LegSide::Buy,
            "5000",
            100,
            day0(),
        ))
        .unwrap();
    assert_eq!(big.brokerage, d("20"));
}

#[test]
fn stt_and_stamp_round_half_up_to_whole_rupees() {
    let cfg = cfg();
    let m = CostModel::new(&cfg, HistoryPolicy::Strict);
    // Intraday sell 51000: STT 0.025% = 12.75 -> 13.
    let s = m
        .charges(&leg(
            LegInstrument::Cash,
            Product::Intraday,
            LegSide::Sell,
            "510",
            100,
            day0(),
        ))
        .unwrap();
    assert_eq!(s.stt, d("13"));
    // Delivery buy 50000: stamp 0.015% = 7.5 -> 8 (half-up, not banker's).
    let b = m
        .charges(&leg(
            LegInstrument::Cash,
            Product::Delivery,
            LegSide::Buy,
            "500",
            100,
            day0(),
        ))
        .unwrap();
    assert_eq!(b.stamp, d("8"));
}

#[test]
fn gst_base_excludes_stt_stamp_and_dp() {
    // Delivery buy: brokerage 0, txn 1.535 -> 1.54 (half-up), SEBI 0.05: GST = 18% x 1.59 =
    // 0.2862 -> 0.29. STT 50 and stamp 8 would give ~18 if they were in the base.
    let cfg = cfg();
    let m = CostModel::new(&cfg, HistoryPolicy::Strict);
    let c = m
        .charges(&leg(
            LegInstrument::Cash,
            Product::Delivery,
            LegSide::Buy,
            "500",
            100,
            day0(),
        ))
        .unwrap();
    assert_eq!(c.gst, d("0.29"));
    assert_eq!(c.stt, d("50"));
}

#[test]
fn dp_once_per_scrip_per_day_on_delivery_sell_only() {
    let cfg = cfg();
    let m = CostModel::new(&cfg, HistoryPolicy::Strict);
    let mut ledger = DpLedger::new();
    let buy = leg(
        LegInstrument::Cash,
        Product::Delivery,
        LegSide::Buy,
        "500",
        100,
        day0(),
    );
    let sell = leg(
        LegInstrument::Cash,
        Product::Delivery,
        LegSide::Sell,
        "510",
        100,
        day0(),
    );
    assert_eq!(m.charges_with_dp(&mut ledger, &buy).unwrap().dp, d("0"));
    // Includes GST already (15.34 = 3.5 + 9.5 + 2.34): not added again.
    assert_eq!(
        m.charges_with_dp(&mut ledger, &sell).unwrap().dp,
        d("15.34")
    );
    assert_eq!(m.charges_with_dp(&mut ledger, &sell).unwrap().dp, d("0"));
    // Same scrip next day pays again; another scrip the same day pays too.
    let mut next = sell.clone();
    next.trade_date = date(2026, 11, 21);
    assert_eq!(
        m.charges_with_dp(&mut ledger, &next).unwrap().dp,
        d("15.34")
    );
    let mut other = sell.clone();
    other.instrument_token = 8;
    assert_eq!(
        m.charges_with_dp(&mut ledger, &other).unwrap().dp,
        d("15.34")
    );
}

#[test]
fn dp_not_charged_on_intraday_sell() {
    let cfg = cfg();
    let m = CostModel::new(&cfg, HistoryPolicy::Strict);
    let mut ledger = DpLedger::new();
    let sell = leg(
        LegInstrument::Cash,
        Product::Intraday,
        LegSide::Sell,
        "510",
        100,
        day0(),
    );
    assert_eq!(m.charges_with_dp(&mut ledger, &sell).unwrap().dp, d("0"));
}

#[test]
fn dp_gets_gst_added_when_config_says_it_is_excluded() {
    let text = real_text().replace("includes_gst = true", "includes_gst = false");
    let cfg: ChargesConfig = parse_str(&text).unwrap();
    let m = CostModel::new(&cfg, HistoryPolicy::Strict);
    let mut ledger = DpLedger::new();
    let sell = leg(
        LegInstrument::Cash,
        Product::Delivery,
        LegSide::Sell,
        "510",
        100,
        day0(),
    );
    // 15.34 x 1.18 = 18.1012 -> 18.10
    assert_eq!(
        m.charges_with_dp(&mut ledger, &sell).unwrap().dp,
        d("18.10")
    );
}

#[test]
fn option_sell_stt_is_on_premium() {
    // 120 x 1 = 120: STT 0.15% = 0.18 -> 0 (whole rupee), flat brokerage 20.
    let cfg = cfg();
    let m = CostModel::new(&cfg, HistoryPolicy::Strict);
    let call = LegInstrument::Option(crate::types::OptionRight::Call);
    let c = m
        .charges(&leg(call, Product::Carry, LegSide::Sell, "120", 1, day0()))
        .unwrap();
    assert_eq!(c.brokerage, d("20"));
    assert_eq!(c.stt, d("0"));
    // 120 x 1000: 0.15% of 120000 = 180.
    let big = m
        .charges(&leg(
            call,
            Product::Carry,
            LegSide::Sell,
            "120",
            1000,
            day0(),
        ))
        .unwrap();
    assert_eq!(big.stt, d("180"));
}

#[test]
fn bse_uses_its_own_rates() {
    let cfg = cfg();
    let m = CostModel::new(&cfg, HistoryPolicy::Strict).with_exchange(ChargeExchange::Bse);
    // 50000 x 0.00375% = 1.875 -> 1.88
    let c = m
        .charges(&leg(
            LegInstrument::Cash,
            Product::Intraday,
            LegSide::Buy,
            "500",
            100,
            day0(),
        ))
        .unwrap();
    assert_eq!(c.exchange_txn, d("1.88"));
}

fn two_date_cfg() -> ChargesConfig {
    // Test-only second rate set from 2027-01-01 with a distinct flat brokerage of 5.
    let extra = r#"
[[rate_set]]
exchange = "NSE"
segment = "equity_intraday"
side = "buy"
effective_from = "2027-01-01"
verified_on = "2027-01-01"
source = "test fixture"
turnover_basis = "notional"
brokerage = { kind = "flat", flat = "5" }
stt_ctt_pct = "0"
exchange_txn_pct = "0.00307"
sebi_per_crore = "10"
ipft_pct = "0"
stamp_pct = "0.003"
"#;
    parse_str(&format!("{}\n{extra}", real_text())).unwrap()
}

#[test]
fn each_leg_uses_the_rate_set_in_force_on_its_trade_date() {
    let cfg = two_date_cfg();
    let m = CostModel::new(&cfg, HistoryPolicy::Strict);
    let mk = |on| {
        leg(
            LegInstrument::Cash,
            Product::Intraday,
            LegSide::Buy,
            "5000",
            100,
            on,
        )
    };
    assert_eq!(
        m.charges(&mk(date(2026, 12, 31))).unwrap().brokerage,
        d("20")
    );
    assert_eq!(m.charges(&mk(date(2027, 1, 1))).unwrap().brokerage, d("5"));
}

#[test]
fn strict_policy_errors_before_the_earliest_rate_set() {
    let cfg = cfg();
    let m = CostModel::new(&cfg, HistoryPolicy::Strict);
    let before = date(2026, 11, 19);
    let r = m.charges(&leg(
        LegInstrument::Cash,
        Product::Delivery,
        LegSide::Buy,
        "500",
        1,
        before,
    ));
    assert!(matches!(r, Err(BacktestError::NoRateSet(d)) if d == before));
    assert!(!m.costs_approximate());
}

#[test]
fn latest_for_all_uses_the_newest_set_and_flags_approximate() {
    let cfg = two_date_cfg();
    let m = CostModel::new(&cfg, HistoryPolicy::LatestForAll);
    assert!(m.costs_approximate());
    let old = leg(
        LegInstrument::Cash,
        Product::Intraday,
        LegSide::Buy,
        "5000",
        100,
        date(2020, 1, 1),
    );
    assert_eq!(m.charges(&old).unwrap().brokerage, d("5"));
}

#[test]
fn impossible_segment_product_mixes_are_invalid() {
    let cfg = cfg();
    let m = CostModel::new(&cfg, HistoryPolicy::Strict);
    let carry_cash = leg(
        LegInstrument::Cash,
        Product::Carry,
        LegSide::Buy,
        "500",
        1,
        day0(),
    );
    assert!(matches!(
        m.charges(&carry_cash),
        Err(BacktestError::Invalid(_))
    ));
    let del_fut = leg(
        LegInstrument::Future,
        Product::Delivery,
        LegSide::Buy,
        "500",
        1,
        day0(),
    );
    assert!(matches!(
        m.charges(&del_fut),
        Err(BacktestError::Invalid(_))
    ));
    let mut wrong_seg = leg(
        LegInstrument::Future,
        Product::Carry,
        LegSide::Buy,
        "500",
        1,
        day0(),
    );
    wrong_seg.segment = Segment::Equity;
    assert!(matches!(
        m.charges(&wrong_seg),
        Err(BacktestError::Invalid(_))
    ));
    let mut cash_seg = leg(
        LegInstrument::Cash,
        Product::Intraday,
        LegSide::Buy,
        "500",
        1,
        day0(),
    );
    cash_seg.segment = Segment::Currency;
    assert!(matches!(
        m.charges(&cash_seg),
        Err(BacktestError::Invalid(_))
    ));
}

#[test]
fn missing_gst_or_dp_table_is_an_error_not_a_default() {
    let mut cfg = cfg();
    cfg.dp = None;
    let m = CostModel::new(&cfg, HistoryPolicy::Strict);
    let sell = leg(
        LegInstrument::Cash,
        Product::Delivery,
        LegSide::Sell,
        "510",
        1,
        day0(),
    );
    assert!(matches!(
        m.charges_with_dp(&mut DpLedger::new(), &sell),
        Err(BacktestError::Invalid(_))
    ));
    cfg.gst = None;
    let m = CostModel::new(&cfg, HistoryPolicy::Strict);
    assert!(matches!(m.charges(&sell), Err(BacktestError::Invalid(_))));
}

#[test]
fn overflowing_turnover_is_an_error() {
    let cfg = cfg();
    let m = CostModel::new(&cfg, HistoryPolicy::Strict);
    let mut l = leg(
        LegInstrument::Cash,
        Product::Intraday,
        LegSide::Buy,
        "500",
        1,
        day0(),
    );
    l.price = Price::snap_to_tick(Decimal::MAX, d("1")).unwrap();
    l.qty = Qty(u32::MAX);
    assert!(m.charges(&l).is_err());
}
