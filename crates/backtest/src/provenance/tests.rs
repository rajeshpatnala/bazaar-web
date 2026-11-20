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
fn the_option_exercise_table_moves_the_file_range_at_either_end() {
    use bazaar_core::config::OptionExercise;
    let ex = |on| OptionExercise {
        stt_pct: Decimal::ONE,
        verified_on: on,
        source: "test".into(),
    };
    let mut c = charges();
    c.option_exercise = Some(ex(day(8, 1)));
    assert_eq!(
        charges_file_verified_range(&c),
        Some((day(1, 5), day(8, 1)))
    );
    c.option_exercise = Some(ex(day(1, 1)));
    assert_eq!(
        charges_file_verified_range(&c),
        Some((day(1, 1), day(7, 1)))
    );
}

#[test]
fn every_dated_source_moves_the_file_range_at_both_ends() {
    type Set = fn(&mut ChargesConfig, NaiveDate);
    use bazaar_core::config::OptionExercise;
    let early = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
    let late = NaiveDate::from_ymd_opt(2027, 1, 1).unwrap();
    // The fixture's own range is (01-05, 07-01), so each source starts inside it.
    let sources: [(&str, Set); 6] = [
        ("file", |c, d| c.verified_on = Some(d)),
        ("gst", |c, d| c.gst.as_mut().unwrap().verified_on = d),
        ("dp", |c, d| c.dp.as_mut().unwrap().verified_on = d),
        ("option_exercise", |c, d| {
            c.option_exercise = Some(OptionExercise {
                stt_pct: Decimal::ONE,
                verified_on: d,
                source: "test".into(),
            });
        }),
        ("rate_set buy", |c, d| c.rate_set[0].verified_on = d),
        ("rate_set sell", |c, d| c.rate_set[1].verified_on = d),
    ];
    for (name, set) in sources {
        let (mut lo, mut hi) = (charges(), charges());
        set(&mut lo, early);
        set(&mut hi, late);
        let lo_range = charges_file_verified_range(&lo);
        let hi_range = charges_file_verified_range(&hi);
        // Moving a source to an extreme date must put that date at that end of the range.
        assert_eq!(lo_range.map(|r| r.0), Some(early), "{name}: low end");
        assert_eq!(hi_range.map(|r| r.1), Some(late), "{name}: high end");
    }
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
        kind: RunKind::Plain,
        charges_used: ChargesUsed::Range(day(1, 5), day(7, 1)),
    }
}

#[test]
fn walk_forward_gate_evidence_comes_from_the_result() {
    use crate::cost::CostModel;
    use crate::walkforward::{ParamSet, WfParams, walk_forward};
    let charges = charges();
    let trials = trials();
    let mut inp = inputs(&[], &charges, &[], &[], &trials);
    assert_eq!(build(&inp).unwrap().gate_evidence, None);
    // A real (empty-data) walk-forward result: the kind takes its text, so it cannot be None.
    let cfg =
        BacktestConfig::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config")).unwrap();
    let sets = [ParamSet {
        id: "s".into(),
        params: "p".into(),
    }];
    let params = WfParams {
        capital: Decimal::ONE_THOUSAND,
        sets: &sets,
        regimes: 1,
        cost: CostModel::new(
            &charges,
            HistoryPolicy::Strict,
            bazaar_core::config::ChargeExchange::Nse,
        ),
    };
    let res = walk_forward(&cfg, &[], &params, &mut |_, _, _, _| unreachable!()).unwrap();
    inp.kind = RunKind::of(&res);
    let prov = build(&inp).unwrap();
    assert_eq!(prov.gate_evidence.as_deref(), Some(res.gate_evidence));
    assert_ne!(res.gate_evidence.trim(), "");
}

#[test]
fn a_run_with_no_legs_says_no_charges_were_levied() {
    let charges = charges();
    let trials = trials();
    let mut inp = inputs(&[], &charges, &[], &[], &trials);
    inp.charges_used = ChargesUsed::of(None);
    let p = build(&inp).unwrap();
    assert_eq!(
        (
            p.charges_verified_from,
            p.charges_verified_to,
            p.no_charges_levied
        ),
        (None, None, true)
    );
    assert_eq!(
        ChargesUsed::of(Some((day(2, 1), day(3, 1)))),
        ChargesUsed::Range(day(2, 1), day(3, 1))
    );
}

#[test]
fn the_provenance_records_the_used_charges_range_not_the_file_range() {
    let charges = charges();
    let trials = trials();
    let mut inp = inputs(&[], &charges, &[], &[], &trials);
    // The whole-file range is a different pair, so this proves the used range is what is stored.
    assert_eq!(
        charges_file_verified_range(&charges),
        Some((day(1, 5), day(7, 1)))
    );
    inp.charges_used = ChargesUsed::Range(day(3, 1), day(4, 1));
    let p = build(&inp).unwrap();
    assert_eq!(
        (
            p.charges_verified_from,
            p.charges_verified_to,
            p.no_charges_levied
        ),
        (Some(day(3, 1)), Some(day(4, 1)), false)
    );
    // A range outside the file's own range means the wrong config or legs: refused.
    inp.charges_used = ChargesUsed::Range(day(1, 4), day(4, 1));
    assert!(matches!(build(&inp), Err(BacktestError::Invalid(_))));
    inp.charges_used = ChargesUsed::Range(day(3, 1), day(7, 2));
    assert!(matches!(build(&inp), Err(BacktestError::Invalid(_))));
    inp.charges_used = ChargesUsed::Range(day(4, 1), day(3, 1));
    assert!(matches!(build(&inp), Err(BacktestError::Invalid(_))));
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
