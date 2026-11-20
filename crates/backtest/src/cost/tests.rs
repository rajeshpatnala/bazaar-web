use bazaar_core::config::{ChargeExchange, ChargesConfig, parse_str};
use bazaar_core::money::{Price, Qty};
use rust_decimal::Decimal;
use std::str::FromStr;

use super::*;

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

/// Inline rates (same numbers as the shipped file, but owned by these tests, so a legitimate
/// rate change in `config/charges.toml` cannot break a worked example; the real file is
/// exercised in `tests/cost_samples.rs`).
fn row(
    exch: &str,
    seg: &str,
    side: &str,
    from: &str,
    basis: &str,
    brokerage: &str,
    (stt, txn, stamp): (&str, &str, &str),
) -> String {
    format!(
        "[[rate_set]]\nexchange = \"{exch}\"\nsegment = \"{seg}\"\nside = \"{side}\"\n\
         effective_from = \"{from}\"\nverified_on = \"{from}\"\nsource = \"test\"\n\
         turnover_basis = \"{basis}\"\nbrokerage = {brokerage}\nstt_ctt_pct = \"{stt}\"\n\
         exchange_txn_pct = \"{txn}\"\nsebi_per_crore = \"10\"\nipft_pct = \"0\"\n\
         stamp_pct = \"{stamp}\"\n\n"
    )
}

fn real_text() -> String {
    const FLAT_PCT: &str = "{ kind = \"flat_or_pct_lower\", flat = \"20\", pct = \"0.03\" }";
    const ZERO: &str = "{ kind = \"zero\" }";
    const FLAT: &str = "{ kind = \"flat\", flat = \"20\" }";
    let mut t = String::from(
        "schema_version = 1\nverified_on = \"2026-06-01\"\nsource = \"test\"\n\n\
         [meta]\nbroker = \"t\"\ncurrency = \"INR\"\nmax_age_days = 90\n\
         history_policy = \"effective_dated\"\n\n\
         [gst]\nrate_pct = \"18\"\nverified_on = \"2026-06-01\"\nsource = \"test\"\n\n\
         [dp]\nper_scrip_total = \"15.34\"\nincludes_gst = true\nverified_on = \"2026-06-01\"\n\
         source = \"test\"\n\n\
         [option_exercise]\nstt_pct = \"0.15\"\nverified_on = \"2026-06-01\"\nsource = \"test\"\n\n",
    );
    let f = "2026-01-01";
    t += &row(
        "NSE",
        "equity_delivery",
        "buy",
        f,
        "notional",
        ZERO,
        ("0.1", "0.00307", "0.015"),
    );
    t += &row(
        "NSE",
        "equity_delivery",
        "sell",
        f,
        "notional",
        ZERO,
        ("0.1", "0.00307", "0"),
    );
    t += &row(
        "NSE",
        "equity_intraday",
        "buy",
        f,
        "notional",
        FLAT_PCT,
        ("0", "0.00307", "0.003"),
    );
    t += &row(
        "NSE",
        "equity_intraday",
        "sell",
        f,
        "notional",
        FLAT_PCT,
        ("0.025", "0.00307", "0"),
    );
    t += &row(
        "BSE",
        "equity_intraday",
        "buy",
        f,
        "notional",
        FLAT_PCT,
        ("0", "0.00375", "0.003"),
    );
    t += &row(
        "NSE",
        "equity_futures",
        "buy",
        f,
        "notional",
        FLAT_PCT,
        ("0", "0.00183", "0.002"),
    );
    t += &row(
        "NSE",
        "equity_options",
        "buy",
        f,
        "premium",
        FLAT,
        ("0", "0.03553", "0.003"),
    );
    t += &row(
        "NSE",
        "equity_options",
        "sell",
        f,
        "premium",
        FLAT,
        ("0.15", "0.03553", "0"),
    );
    t
}

fn cfg() -> ChargesConfig {
    parse_str(&real_text()).unwrap()
}

fn nse(cfg: &ChargesConfig) -> CostModel<'_> {
    CostModel::new(cfg, HistoryPolicy::Strict, ChargeExchange::Nse)
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

const D0: (i32, u32, u32) = (2026, 6, 1);

fn day0() -> NaiveDate {
    date(D0.0, D0.1, D0.2)
}

#[test]
fn delivery_sell_worked_example() {
    // 510 x 100 = 51000: STT 0.1% = 51 (whole rupee), txn 1.5657 -> 1.57, SEBI 0.051 -> 0.05,
    // GST 18% x (0 + 1.57 + 0.05) = 0.2916 -> 0.29, stamp 0 on a sell.
    let cfg = cfg();
    let m = nse(&cfg);
    let c = m
        .charges_excl_dp(&leg(
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
    let m = nse(&cfg);
    // 500 x 100 = 50000: 0.03% = 15 < 20.
    let small = m
        .charges_excl_dp(&leg(
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
        .charges_excl_dp(&leg(
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
    let m = nse(&cfg);
    // Intraday sell 51000: STT 0.025% = 12.75 -> 13.
    let s = m
        .charges_excl_dp(&leg(
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
        .charges_excl_dp(&leg(
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
    // 0.2862 -> 0.29. STT 50 and stamp 8 would give about 10.73 (0.18 x 59.59) if they were in the base.
    let cfg = cfg();
    let m = nse(&cfg);
    let c = m
        .charges_excl_dp(&leg(
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
    let m = nse(&cfg);
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
    next.trade_date = date(2026, 6, 2);
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
    let m = nse(&cfg);
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
    let m = nse(&cfg);
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
    let m = nse(&cfg);
    let call = LegInstrument::Option(crate::types::OptionRight::Call);
    let c = m
        .charges_excl_dp(&leg(call, Product::Carry, LegSide::Sell, "120", 1, day0()))
        .unwrap();
    assert_eq!(c.brokerage, d("20"));
    assert_eq!(c.stt, d("0"));
    // 120 x 1000: 0.15% of 120000 = 180.
    let big = m
        .charges_excl_dp(&leg(
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
    let m = CostModel::new(&cfg, HistoryPolicy::Strict, ChargeExchange::Bse);
    // 50000 x 0.00375% = 1.875 -> 1.88
    let c = m
        .charges_excl_dp(&leg(
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
    let m = nse(&cfg);
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
        m.charges_excl_dp(&mk(date(2026, 12, 31)))
            .unwrap()
            .brokerage,
        d("20")
    );
    assert_eq!(
        m.charges_excl_dp(&mk(date(2027, 1, 1))).unwrap().brokerage,
        d("5")
    );
}

#[test]
fn strict_policy_errors_before_the_earliest_rate_set() {
    let cfg = cfg();
    let m = nse(&cfg);
    let before = date(2025, 12, 31);
    let r = m.charges_excl_dp(&leg(
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
    let m = CostModel::new(&cfg, HistoryPolicy::LatestForAll, ChargeExchange::Nse);
    assert!(m.costs_approximate());
    let old = leg(
        LegInstrument::Cash,
        Product::Intraday,
        LegSide::Buy,
        "5000",
        100,
        date(2020, 1, 1),
    );
    assert_eq!(m.charges_excl_dp(&old).unwrap().brokerage, d("5"));
}

#[test]
fn impossible_segment_product_mixes_are_invalid() {
    let cfg = cfg();
    let m = nse(&cfg);
    let carry_cash = leg(
        LegInstrument::Cash,
        Product::Carry,
        LegSide::Buy,
        "500",
        1,
        day0(),
    );
    assert!(matches!(
        m.charges_excl_dp(&carry_cash),
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
        m.charges_excl_dp(&del_fut),
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
        m.charges_excl_dp(&wrong_seg),
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
        m.charges_excl_dp(&cash_seg),
        Err(BacktestError::Invalid(_))
    ));
}

#[test]
fn missing_gst_or_dp_table_is_an_error_not_a_default() {
    let mut cfg = cfg();
    cfg.dp = None;
    let m = nse(&cfg);
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
    let m = nse(&cfg);
    assert!(matches!(
        m.charges_excl_dp(&sell),
        Err(BacktestError::Invalid(_))
    ));
}

#[test]
fn overflowing_turnover_is_an_error() {
    let cfg = cfg();
    let m = nse(&cfg);
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
    assert!(m.charges_excl_dp(&l).is_err());
}

#[test]
fn zero_quantity_or_price_is_invalid() {
    let cfg = cfg();
    let m = nse(&cfg);
    let call = LegInstrument::Option(crate::types::OptionRight::Call);
    let q0 = leg(call, Product::Carry, LegSide::Buy, "100", 0, day0());
    assert!(
        matches!(m.charges_excl_dp(&q0), Err(BacktestError::Invalid(s)) if s == "zero quantity")
    );
    let sell0 = leg(
        LegInstrument::Cash,
        Product::Delivery,
        LegSide::Sell,
        "100",
        0,
        day0(),
    );
    let mut ledger = DpLedger::new();
    assert!(m.charges_with_dp(&mut ledger, &sell0).is_err());
    let p0 = leg(call, Product::Carry, LegSide::Buy, "0", 75, day0());
    assert!(matches!(m.charges_excl_dp(&p0), Err(BacktestError::Invalid(s)) if s == "zero price"));
}

#[test]
fn turnover_basis_must_match_the_instrument() {
    // An option row marked notional (or a cash row marked premium) is a config error.
    let text = real_text().replace("\"premium\"", "\"notional\"");
    let cfg: ChargesConfig = parse_str(&text).unwrap();
    let call = LegInstrument::Option(crate::types::OptionRight::Call);
    let r = nse(&cfg).charges_excl_dp(&leg(call, Product::Carry, LegSide::Buy, "100", 75, day0()));
    assert!(matches!(r, Err(BacktestError::Invalid(s)) if s.contains("turnover basis")));
    let text = real_text().replacen("\"notional\"", "\"premium\"", 1);
    let cfg: ChargesConfig = parse_str(&text).unwrap();
    let cash = leg(
        LegInstrument::Cash,
        Product::Delivery,
        LegSide::Buy,
        "100",
        1,
        day0(),
    );
    assert!(matches!(
        nse(&cfg).charges_excl_dp(&cash),
        Err(BacktestError::Invalid(_))
    ));
}

#[test]
fn a_missing_row_is_invalid_not_no_rate_set() {
    // No NSE futures sell row in the inline config: a config gap, not a date problem.
    let cfg = cfg();
    let fut = leg(
        LegInstrument::Future,
        Product::Carry,
        LegSide::Sell,
        "24000",
        1,
        day0(),
    );
    assert!(matches!(
        nse(&cfg).charges_excl_dp(&fut),
        Err(BacktestError::Invalid(_))
    ));
}

#[test]
fn latest_for_all_takes_the_newest_set_even_if_future_dated() {
    let cfg = two_date_cfg(); // second intraday buy set from 2027-01-01, brokerage flat 5
    let m = CostModel::new(&cfg, HistoryPolicy::LatestForAll, ChargeExchange::Nse);
    let l = leg(
        LegInstrument::Cash,
        Product::Intraday,
        LegSide::Buy,
        "5000",
        100,
        day0(),
    );
    assert_eq!(m.charges_excl_dp(&l).unwrap().brokerage, d("5"));
}

#[test]
fn leg_is_approximate_counts_only_legs_off_their_own_rate_set() {
    let cfg = two_date_cfg();
    let latest = CostModel::new(&cfg, HistoryPolicy::LatestForAll, ChargeExchange::Nse);
    let mk = |on| {
        leg(
            LegInstrument::Cash,
            Product::Intraday,
            LegSide::Buy,
            "500",
            1,
            on,
        )
    };
    assert!(latest.leg_is_approximate(&mk(day0())).unwrap());
    assert!(latest.leg_is_approximate(&mk(date(2020, 1, 1))).unwrap());
    assert!(!latest.leg_is_approximate(&mk(date(2027, 2, 1))).unwrap());
    assert!(!nse(&cfg).leg_is_approximate(&mk(day0())).unwrap());
}

fn range_of(cfg: &ChargesConfig, l: &Leg) -> Result<Option<(NaiveDate, NaiveDate)>, BacktestError> {
    nse(cfg).used_verified_range([l])
}

fn cash(product: Product, side: LegSide) -> Leg {
    leg(LegInstrument::Cash, product, side, "500", 1, day0())
}

#[test]
fn the_dp_date_widens_the_range_for_a_delivery_sell_only() {
    let base = (date(2026, 1, 1), date(2026, 6, 1));
    let mut cfg = cfg();
    let sell = cash(Product::Delivery, LegSide::Sell);
    assert_eq!(range_of(&cfg, &sell).unwrap(), Some(base));
    // DP the latest date in the set.
    cfg.dp.as_mut().unwrap().verified_on = date(2026, 9, 1);
    assert_eq!(
        range_of(&cfg, &sell).unwrap(),
        Some((base.0, date(2026, 9, 1)))
    );
    // A delivery buy and an intraday sell do not levy DP: no widening.
    assert_eq!(
        range_of(&cfg, &cash(Product::Delivery, LegSide::Buy)).unwrap(),
        Some(base)
    );
    assert_eq!(
        range_of(&cfg, &cash(Product::Intraday, LegSide::Sell)).unwrap(),
        Some(base)
    );
    // DP the earliest date in the set.
    cfg.dp.as_mut().unwrap().verified_on = date(2025, 6, 1);
    assert_eq!(
        range_of(&cfg, &sell).unwrap(),
        Some((date(2025, 6, 1), base.1))
    );
    assert_eq!(
        range_of(&cfg, &cash(Product::Delivery, LegSide::Buy)).unwrap(),
        Some(base)
    );
}

#[test]
fn a_missing_gst_or_dp_table_is_invalid_where_it_is_needed() {
    let mut cfg = cfg();
    cfg.dp = None;
    let sell = cash(Product::Delivery, LegSide::Sell);
    let intraday = cash(Product::Intraday, LegSide::Buy);
    assert!(matches!(
        range_of(&cfg, &sell),
        Err(BacktestError::Invalid(_))
    ));
    assert!(range_of(&cfg, &intraday).unwrap().is_some());
    cfg.gst = None;
    assert!(matches!(
        range_of(&cfg, &intraday),
        Err(BacktestError::Invalid(_))
    ));
}

#[test]
fn latest_for_all_reports_the_newest_set_date() {
    let cfg = two_date_cfg();
    let latest = CostModel::new(&cfg, HistoryPolicy::LatestForAll, ChargeExchange::Nse);
    let early = leg(
        LegInstrument::Cash,
        Product::Intraday,
        LegSide::Buy,
        "500",
        1,
        date(2025, 12, 31),
    );
    // Strict has no set that early; latest_for_all uses the 2027 set (verified 2027-01-01).
    assert!(matches!(
        range_of(&cfg, &early),
        Err(BacktestError::NoRateSet(_))
    ));
    assert_eq!(
        latest.used_verified_range([&early]).unwrap(),
        Some((date(2026, 6, 1), date(2027, 1, 1)))
    );
}

#[test]
fn used_verified_range_follows_the_sets_the_legs_actually_used() {
    let cfg = two_date_cfg();
    let m = nse(&cfg);
    let buy = |on| {
        leg(
            LegInstrument::Cash,
            Product::Intraday,
            LegSide::Buy,
            "500",
            1,
            on,
        )
    };
    assert_eq!(m.used_verified_range([]).unwrap(), None);
    // Rate set verified 2026-01-01, GST table 2026-06-01.
    let early = buy(day0());
    assert_eq!(
        m.used_verified_range([&early]).unwrap(),
        Some((date(2026, 1, 1), date(2026, 6, 1)))
    );
    // The 2027 set (verified 2027-01-01) widens the upper end only when a leg used it.
    let late = buy(date(2027, 2, 1));
    assert_eq!(
        m.used_verified_range([&early, &late]).unwrap(),
        Some((date(2026, 1, 1), date(2027, 1, 1)))
    );
    // A leg before the earliest set is an error, not a silently skipped leg.
    let before = buy(date(2025, 12, 31));
    assert!(matches!(
        m.used_verified_range([&before]),
        Err(BacktestError::NoRateSet(_))
    ));
}
