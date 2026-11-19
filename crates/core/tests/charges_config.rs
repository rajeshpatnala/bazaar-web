//! `charges.toml` loader: Decimal-from-string, strict keys, effective-dated lookup.
//! All rates here are synthetic; the shipped file is covered by `real_charges.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::str::FromStr;

use bazaar_core::config::{
    Brokerage, ChargeExchange as Ex, ChargeSegment as Seg, ChargeSide as Side, ChargesConfig,
    ConfigError, NoRateSet, load, parse_str,
};
use chrono::NaiveDate;
use rust_decimal::Decimal;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/config");

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

const HEAD: &str = "schema_version = 1\nverified_on = \"2026-01-05\"\nsource = \"x\"\n\
[meta]\nbroker = \"b\"\ncurrency = \"INR\"\nmax_age_days = 90\nhistory_policy = \"effective_dated\"\n\
[gst]\nrate_pct = \"10\"\nverified_on = \"2026-01-05\"\nsource = \"s\"\n";

const SET: &str = "[[rate_set]]\nexchange = \"NSE\"\nsegment = \"equity_intraday\"\nside = \"buy\"\n\
effective_from = \"2024-01-01\"\nverified_on = \"2026-01-05\"\nsource = \"s\"\n\
turnover_basis = \"notional\"\nbrokerage = { kind = \"zero\" }\nstt_ctt_pct = \"0\"\n\
exchange_txn_pct = \"0.01\"\nsebi_per_crore = \"10\"\nipft_pct = \"0\"\nstamp_pct = \"0.01\"\n";

fn with_set(set: &str) -> String {
    format!("{HEAD}{set}")
}

fn err_text(text: &str) -> String {
    parse_str::<ChargesConfig>(text).unwrap_err().to_string()
}

#[test]
fn fixture_loads_with_exact_decimals() {
    let cfg: ChargesConfig = load(Path::new(FIXTURES)).unwrap();
    assert_eq!(cfg.rate_set.len(), 4);
    let r = cfg
        .rate_set_for(Ex::Nse, Seg::EquityIntraday, Side::Buy, d(2026, 1, 1))
        .unwrap();
    assert_eq!(r.exchange_txn_pct, dec("0.02"));
    assert_eq!(
        r.brokerage,
        Brokerage::FlatOrPctLower {
            flat: dec("20"),
            pct: dec("0.1")
        }
    );
    assert_eq!(cfg.gst.as_ref().unwrap().rate_pct, dec("10"));
}

#[test]
fn lookup_picks_latest_effective_from_not_after_date() {
    let cfg: ChargesConfig = load(Path::new(FIXTURES)).unwrap();
    let f = |date| {
        cfg.rate_set_for(Ex::Nse, Seg::EquityIntraday, Side::Buy, date)
            .map(|r| r.exchange_txn_pct)
    };
    assert_eq!(f(d(2024, 1, 1)).unwrap(), dec("0.01"));
    assert_eq!(f(d(2025, 3, 31)).unwrap(), dec("0.01"));
    assert_eq!(f(d(2025, 4, 1)).unwrap(), dec("0.02"));
    assert_eq!(f(d(2030, 1, 1)).unwrap(), dec("0.02"));
}

#[test]
fn lookup_with_nothing_in_force_is_an_error() {
    let cfg: ChargesConfig = load(Path::new(FIXTURES)).unwrap();
    // before the first effective_from
    let e = cfg
        .rate_set_for(Ex::Nse, Seg::EquityIntraday, Side::Buy, d(2023, 12, 31))
        .unwrap_err();
    assert_eq!(
        e,
        NoRateSet {
            exchange: Ex::Nse,
            segment: Seg::EquityIntraday,
            side: Side::Buy,
            date: d(2023, 12, 31)
        }
    );
    assert!(e.to_string().contains("no rate set in force"));
    // other side, other exchange: no fallback
    assert!(
        cfg.rate_set_for(Ex::Nse, Seg::EquityIntraday, Side::Sell, d(2026, 1, 1))
            .is_err()
    );
    assert!(
        cfg.rate_set_for(Ex::Bse, Seg::EquityIntraday, Side::Buy, d(2026, 1, 1))
            .is_err()
    );
}

#[test]
fn toml_float_for_a_rate_is_rejected() {
    let text = with_set(SET).replace("exchange_txn_pct = \"0.01\"", "exchange_txn_pct = 0.01");
    assert!(matches!(
        parse_str::<ChargesConfig>(&text),
        Err(ConfigError::Parse { .. })
    ));
    let text = with_set(SET).replace("rate_pct = \"10\"", "rate_pct = 10.0");
    assert!(parse_str::<ChargesConfig>(&text).is_err());
    let text = with_set(SET).replace("{ kind = \"zero\" }", "{ kind = \"flat\", flat = 20.0 }");
    assert!(parse_str::<ChargesConfig>(&text).is_err());
}

#[test]
fn non_numeric_and_negative_and_huge_rates_are_rejected() {
    let bad = with_set(SET).replace("\"0.01\"\nsebi", "\"abc\"\nsebi");
    assert!(parse_str::<ChargesConfig>(&bad).is_err());
    let neg = with_set(SET).replace("stamp_pct = \"0.01\"", "stamp_pct = \"-0.01\"");
    assert!(err_text(&neg).contains("rate_set[0].stamp_pct: must not be negative"));
    let big = with_set(SET).replace("stamp_pct = \"0.01\"", "stamp_pct = \"101\"");
    assert!(err_text(&big).contains("above the allowed maximum"));
}

#[test]
fn unknown_keys_are_load_errors_everywhere() {
    assert!(err_text(&format!("{}extra = 1\n", with_set(SET))).contains("unknown field"));
    let row = with_set(SET).replace("ipft_pct = \"0\"", "ipft_pct = \"0\"\nfoo = \"1\"");
    assert!(err_text(&row).contains("unknown field"));
    let meta = with_set(SET).replace("currency = \"INR\"", "currency = \"INR\"\nx = 1");
    assert!(err_text(&meta).contains("unknown field"));
    let brk = with_set(SET).replace("{ kind = \"zero\" }", "{ kind = \"zero\", flat = \"1\" }");
    assert!(parse_str::<ChargesConfig>(&brk).is_err());
    let kind = with_set(SET).replace("\"zero\"", "\"weird\"");
    assert!(parse_str::<ChargesConfig>(&kind).is_err());
}

#[test]
fn duplicate_key_is_a_load_error_but_a_new_effective_from_is_fine() {
    let dup = format!("{}{SET}", with_set(SET));
    let e = err_text(&dup);
    assert!(e.contains("rate_set[1]") && e.contains("duplicate"), "{e}");
    let later = SET.replace("2024-01-01", "2025-01-01");
    assert!(parse_str::<ChargesConfig>(&format!("{}{later}", with_set(SET))).is_ok());
    let other_side = SET.replace("\"buy\"", "\"sell\"");
    assert!(parse_str::<ChargesConfig>(&format!("{}{other_side}", with_set(SET))).is_ok());
}

#[test]
fn missing_source_or_verified_on_is_a_load_error() {
    let no_src = with_set(SET).replace(
        "verified_on = \"2026-01-05\"\nsource = \"s\"\nturnover",
        "verified_on = \"2026-01-05\"\nturnover",
    );
    assert!(err_text(&no_src).contains("source"));
    let blank = with_set(SET).replace("source = \"s\"\nturnover", "source = \"  \"\nturnover");
    assert!(err_text(&blank).contains("rate_set[0].source"));
    let no_ver = with_set(SET).replace(
        "verified_on = \"2026-01-05\"\nsource = \"s\"\nturnover",
        "source = \"s\"\nturnover",
    );
    assert!(err_text(&no_ver).contains("verified_on"));
    let gst_no_src = HEAD.replace("source = \"s\"\n", "");
    assert!(err_text(&gst_no_src).contains("source"));
    let file_no_ver = with_set(SET).replacen("verified_on = \"2026-01-05\"\n", "", 1);
    assert!(matches!(
        parse_str::<ChargesConfig>(&file_no_ver),
        Err(ConfigError::Invalid { .. })
    ));
}

#[test]
fn real_file_requires_meta_gst_and_dependent_tables() {
    let base = "schema_version = 1\nverified_on = \"2026-01-05\"\nsource = \"x\"\n";
    assert!(err_text(base).contains("meta: is required"));
    let no_gst = HEAD.split("[gst]").next().unwrap();
    assert!(err_text(no_gst).contains("gst: is required"));
    let delivery = SET.replace("equity_intraday", "equity_delivery");
    assert!(err_text(&with_set(&delivery)).contains("dp: is required"));
    let options = SET.replace("equity_intraday", "equity_options");
    assert!(err_text(&with_set(&options)).contains("option_exercise: is required"));
}

#[test]
fn placeholder_must_hold_no_tables() {
    let ph = "schema_version = 1\nplaceholder = true\nsource = \"x\"\n";
    assert!(parse_str::<ChargesConfig>(ph).is_ok());
    assert!(
        err_text(&format!("{ph}{}", &HEAD[HEAD.find("[meta]").unwrap()..])).contains("no tables")
    );
}
