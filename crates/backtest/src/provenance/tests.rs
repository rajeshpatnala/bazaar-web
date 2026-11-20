use std::path::Path;

use bazaar_core::config::parse_str;
use rust_decimal::Decimal;

use super::*;
use crate::config::BacktestConfig;

fn day(m: u32, dd: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, dd).unwrap()
}

fn charges() -> ChargesConfig {
    let row = |side: &str, v: &str| {
        format!(
            "[[rate_set]]\nexchange = \"NSE\"\nsegment = \"equity_delivery\"\nside = \"{side}\"\n\
             effective_from = \"2026-01-01\"\nverified_on = \"{v}\"\nsource = \"test\"\n\
             turnover_basis = \"notional\"\nbrokerage = {{ kind = \"zero\" }}\n\
             stt_ctt_pct = \"0\"\nexchange_txn_pct = \"0\"\nsebi_per_crore = \"0\"\n\
             ipft_pct = \"0\"\nstamp_pct = \"0\"\n\n"
        )
    };
    let mut t = String::from(
        "schema_version = 1\nverified_on = \"2026-06-01\"\nsource = \"test\"\n\n\
         [meta]\nbroker = \"t\"\ncurrency = \"INR\"\nmax_age_days = 90\n\
         history_policy = \"effective_dated\"\n\n\
         [gst]\nrate_pct = \"18\"\nverified_on = \"2026-01-05\"\nsource = \"test\"\n\n\
         [dp]\nper_scrip_total = \"15.34\"\nincludes_gst = true\nverified_on = \"2026-07-01\"\n\
         source = \"test\"\n\n",
    );
    t += &row("buy", "2026-01-15");
    t += &row("sell", "2026-03-01");
    parse_str(&t).unwrap()
}

fn trials() -> Trials {
    BacktestConfig::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config"))
        .unwrap()
        .trials
}

#[test]
fn sha256_of_abc_matches_the_reference() {
    assert_eq!(
        hash_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn verified_range_spans_file_gst_and_rate_sets() {
    assert_eq!(
        charges_file_verified_range(&charges()),
        Some((day(1, 5), day(7, 1)))
    );
}

#[test]
fn undated_charges_have_no_range() {
    let c = parse_str("schema_version = 1\nplaceholder = true\nsource = \"none\"\n").unwrap();
    assert_eq!(charges_file_verified_range(&c), None);
}

fn inputs<'a>(
    files: &'a [(&'a str, &'a [u8])],
    c: &'a ChargesConfig,
    uni: &'a [String],
    sets: &'a [(String, String)],
    t: &'a Trials,
) -> ProvenanceInputs<'a> {
    ProvenanceInputs {
        config_files: files,
        charges: c,
        data_from: day(1, 1),
        data_to: day(5, 1),
        history_policy: HistoryPolicy::Strict,
        universe: uni,
        parameter_sets: sets,
        trials: t,
    }
}

#[test]
fn build_fills_every_field_deterministically() {
    let c = charges();
    let t = trials();
    let files: [(&str, &[u8]); 2] = [("risk.toml", b"b"), ("backtest.toml", b"abc")];
    let uni = ["TCS".to_owned(), "INFY".to_owned(), "TCS".to_owned()];
    let ps = |s: &str, p: &str| (s.to_owned(), p.to_owned());
    // Same params under two strategies are two trials; an exact repeat counts once.
    let sets = [
        ps("a", "p=1"),
        ps("a", "p=2"),
        ps("a", "p=1"),
        ps("b", "p=1"),
    ];
    let p = build(&inputs(&files, &c, &uni, &sets, &t)).unwrap();
    assert_eq!(p.universe, ["INFY", "TCS"]);
    assert_eq!(p.parameter_sets, ["a: p=1", "a: p=2", "b: p=1"]);
    // Three distinct trials: tier 1 penalty (+0.1 profit factor), prior_trials 0.
    assert_eq!(p.trials, 3);
    assert_eq!(p.trial_penalty_pf, t.profit_factor_penalty(3));
    assert!(p.trial_penalty_pf > Decimal::ZERO);
    let keys: Vec<&String> = p.config_hashes.keys().collect();
    assert_eq!(keys, ["backtest.toml", "risk.toml"]);
    assert_eq!(p.config_hashes["backtest.toml"], hash_hex(b"abc"));
    assert_eq!(p.charges_verified_from, Some(day(1, 5)));
    assert_eq!(p.charges_verified_to, Some(day(7, 1)));
    assert_eq!((p.data_from, p.data_to), (Some(day(1, 1)), Some(day(5, 1))));
    assert!(!p.costs_approximate);
    assert_eq!(p, build(&inputs(&files, &c, &uni, &sets, &t)).unwrap());
}

#[test]
fn latest_for_all_marks_costs_approximate() {
    let c = charges();
    let t = trials();
    let mut i = inputs(&[], &c, &[], &[], &t);
    i.history_policy = HistoryPolicy::LatestForAll;
    let p = build(&i).unwrap();
    assert!(p.costs_approximate);
    // A run with no grid is one trial (policy section 4).
    assert_eq!(p.trials, 1);
    assert_eq!(p.trial_penalty_pf, Decimal::ZERO);
}

#[test]
fn reversed_data_range_and_undated_charges_are_invalid() {
    let c = charges();
    let t = trials();
    let mut i = inputs(&[], &c, &[], &[], &t);
    i.data_from = day(6, 1);
    assert!(matches!(build(&i), Err(BacktestError::Invalid(_))));
    let undated = parse_str("schema_version = 1\nplaceholder = true\nsource = \"none\"\n").unwrap();
    let i = inputs(&[], &undated, &[], &[], &t);
    assert!(matches!(build(&i), Err(BacktestError::Invalid(_))));
}
