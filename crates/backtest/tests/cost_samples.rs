//! The five owner calculator samples against the real `config/charges.toml` (AC 1).
//! A sample is a buy leg plus a sell leg; lines are summed and compared within the stated tolerance.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::str::FromStr;

use bazaar_backtest::{
    CostModel, HistoryPolicy, Leg, LegInstrument, LegSide, OptionRight, Product,
};
use bazaar_core::config::{ChargeExchange, ChargesConfig, parse_str};
use bazaar_core::money::{Price, Qty};
use bazaar_core::segment::Segment;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde_json::Value;

fn dec(v: &Value, k: &str) -> Decimal {
    Decimal::from_str(v[k].as_str().unwrap()).unwrap()
}

fn root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn calculator_samples_match_within_tolerance() {
    let cfg: ChargesConfig =
        parse_str(&std::fs::read_to_string(root().join("config/charges.toml")).unwrap()).unwrap();
    let fx: Value = serde_json::from_str(
        &std::fs::read_to_string(root().join("fixtures/charges/calculator_2026-11-20.json"))
            .unwrap(),
    )
    .unwrap();
    let tol = dec(&fx, "tolerance_per_line");
    let on = NaiveDate::from_ymd_opt(2026, 11, 20).unwrap();
    let samples = fx["samples"].as_array().unwrap();
    assert_eq!(samples.len(), 5);
    for s in samples {
        let exchange = match s["exchange"].as_str().unwrap() {
            "NSE" => ChargeExchange::Nse,
            "BSE" => ChargeExchange::Bse,
            other => panic!("unknown exchange {other}"),
        };
        let instrument = match s["instrument"].as_str().unwrap() {
            "cash" => LegInstrument::Cash,
            "future" => LegInstrument::Future,
            "option_call" => LegInstrument::Option(OptionRight::Call),
            other => panic!("unknown instrument {other}"),
        };
        let product = match s["product"].as_str().unwrap() {
            "delivery" => Product::Delivery,
            "intraday" => Product::Intraday,
            "carry" => Product::Carry,
            other => panic!("unknown product {other}"),
        };
        let mk = |side, key| Leg {
            instrument_token: 1,
            symbol: "S".into(),
            segment: if instrument == LegInstrument::Cash {
                Segment::Equity
            } else {
                Segment::EquityDerivatives
            },
            instrument,
            product,
            side,
            qty: Qty(u32::try_from(s["qty"].as_u64().unwrap()).unwrap()),
            price: Price::snap_to_tick(dec(s, key), Decimal::new(5, 2)).unwrap(),
            trade_date: on,
        };
        let model = CostModel::new(&cfg, HistoryPolicy::Strict, exchange);
        let b = model.charges_excl_dp(&mk(LegSide::Buy, "buy")).unwrap();
        let sl = model.charges_excl_dp(&mk(LegSide::Sell, "sell")).unwrap();
        let sum = b.checked_add(&sl).unwrap();
        let id = s["id"].as_u64().unwrap();
        let lines = [
            ("brokerage", sum.brokerage),
            ("stt", sum.stt),
            ("exchange_txn", sum.exchange_txn),
            ("gst", sum.gst),
            ("sebi", sum.sebi),
            ("stamp", sum.stamp),
        ];
        for (name, got) in lines {
            let want = dec(s, name);
            assert!(
                (got - want).abs() <= tol,
                "sample {id} {name}: got {got}, calculator {want}"
            );
        }
        let total = sum.total().unwrap();
        let want_total = dec(s, "total");
        let total_tol = tol * Decimal::from(lines.len());
        assert!(
            (total - want_total).abs() <= tol,
            "sample {id} total beyond one line tolerance: got {total}, calculator {want_total}"
        );
        assert!(
            (total - want_total).abs() <= total_tol,
            "sample {id} total: got {total}, calculator {want_total}"
        );
    }
}
