//! The shipped `config/charges.toml` (Zerodha page, owner-pasted 2026-11-20) through the typed loader.
//! Expected values are the owner's table in docs/owner-input/F2-charges.md.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::str::FromStr;

use bazaar_core::config::{
    Brokerage, ChargeExchange as Ex, ChargeSegment as Seg, ChargeSide as Side, ChargesConfig,
    TurnoverBasis, load,
};
use chrono::NaiveDate;
use rust_decimal::Decimal;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn cfg() -> ChargesConfig {
    load(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../config"
    )))
    .unwrap()
}

const ON: fn() -> NaiveDate = || d(2026, 11, 20);

#[test]
fn shipped_charges_are_real_dated_and_sourced() {
    let c = cfg();
    assert!(!c.placeholder);
    assert_eq!(c.verified_on, Some(ON()));
    assert_eq!(c.meta.as_ref().unwrap().broker, "zerodha");
    assert_eq!(c.gst.as_ref().unwrap().rate_pct, dec("18"));
    assert_eq!(c.option_exercise.as_ref().unwrap().stt_pct, dec("0.15"));
    // DP charge is unverified and must not ship.
    assert!(c.dp.is_none());
    assert_eq!(c.rate_set.len(), 12);
    for r in &c.rate_set {
        assert_eq!(r.verified_on, ON());
        assert_eq!(r.effective_from, ON());
        assert!(r.source.contains("zerodha.com/charges"));
    }
    // Delivery is not shipped (needs the unverified DP charge).
    assert!(c.rate_set.iter().all(|r| r.segment != Seg::EquityDelivery));
}

#[test]
fn intraday_rates_match_the_owner_table() {
    let c = cfg();
    let sell = c
        .rate_set_for(Ex::Nse, Seg::EquityIntraday, Side::Sell, ON())
        .unwrap();
    assert_eq!(
        sell.brokerage,
        Brokerage::FlatOrPctLower {
            flat: dec("20"),
            pct: dec("0.03")
        }
    );
    assert_eq!(sell.stt_ctt_pct, dec("0.025"));
    assert_eq!(sell.stamp_pct, dec("0"));
    assert_eq!(sell.exchange_txn_pct, dec("0.00307"));
    assert_eq!(sell.sebi_per_crore, dec("10"));
    assert_eq!(sell.turnover_basis, TurnoverBasis::Notional);
    let buy = c
        .rate_set_for(Ex::Nse, Seg::EquityIntraday, Side::Buy, ON())
        .unwrap();
    assert_eq!((buy.stt_ctt_pct, buy.stamp_pct), (dec("0"), dec("0.003")));
    let bse = c
        .rate_set_for(Ex::Bse, Seg::EquityIntraday, Side::Sell, ON())
        .unwrap();
    assert_eq!(bse.exchange_txn_pct, dec("0.00375"));
}

#[test]
fn futures_rates_match_the_owner_table() {
    let c = cfg();
    let sell = c
        .rate_set_for(Ex::Nse, Seg::EquityFutures, Side::Sell, ON())
        .unwrap();
    assert_eq!(sell.stt_ctt_pct, dec("0.05"));
    assert_eq!(sell.exchange_txn_pct, dec("0.00183"));
    let buy = c
        .rate_set_for(Ex::Nse, Seg::EquityFutures, Side::Buy, ON())
        .unwrap();
    assert_eq!(buy.stamp_pct, dec("0.002"));
    let bse = c
        .rate_set_for(Ex::Bse, Seg::EquityFutures, Side::Sell, ON())
        .unwrap();
    assert_eq!(bse.exchange_txn_pct, dec("0"));
}

#[test]
fn options_rates_match_the_owner_table() {
    let c = cfg();
    let sell = c
        .rate_set_for(Ex::Nse, Seg::EquityOptions, Side::Sell, ON())
        .unwrap();
    assert_eq!(sell.brokerage, Brokerage::Flat { flat: dec("20") });
    assert_eq!(sell.stt_ctt_pct, dec("0.15"));
    assert_eq!(sell.exchange_txn_pct, dec("0.03553"));
    assert_eq!(sell.turnover_basis, TurnoverBasis::Premium);
    let buy = c
        .rate_set_for(Ex::Nse, Seg::EquityOptions, Side::Buy, ON())
        .unwrap();
    assert_eq!((buy.stt_ctt_pct, buy.stamp_pct), (dec("0"), dec("0.003")));
    let bse = c
        .rate_set_for(Ex::Bse, Seg::EquityOptions, Side::Sell, ON())
        .unwrap();
    assert_eq!(bse.exchange_txn_pct, dec("0.0325"));
}

#[test]
fn no_rate_before_effective_date_or_for_delivery() {
    let c = cfg();
    assert!(
        c.rate_set_for(Ex::Nse, Seg::EquityIntraday, Side::Buy, d(2026, 11, 19))
            .is_err()
    );
    assert!(
        c.rate_set_for(Ex::Nse, Seg::EquityDelivery, Side::Sell, ON())
            .is_err()
    );
}
