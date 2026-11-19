//! Cross-rule properties of the gate: every rule runs, Hard and No rules are never overridable,
//! messages are plain and end with their rule id, and the verdict is deterministic.
//!
//! The generator varies the snapshot (each field known or unknown), side, stop side, ban list,
//! held position, stop move, events and mode evidence. Instrument type is fixed to equity and the
//! option and margin rules are covered by their own slice tests (`tests/gate.rs`, rule modules).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use bazaar_core::clock::Clock;
use bazaar_core::instrument::{Exchange, Instrument, InstrumentType};
use bazaar_core::money::Qty;
use bazaar_risk::baseline::{BaselineError, LimitsBaseline};
use bazaar_risk::config::{Baseline, GateConfig};
use bazaar_risk::gate::{CostModel, Evaluation, Gate, GateInputs, GateReject, ModeEvidence};
use bazaar_risk::override_::IntentDigest;
use bazaar_risk::override_::{
    OverrideChallenge, OverrideError, OverrideRequest, ReasonType, evaluate,
};
use bazaar_risk::rules::behaviour::EntryZone;
use bazaar_risk::rules::exposure::ExposureInputs;
use bazaar_risk::rules::inputs::{BanList, DataInputs, EventFlags, EvidenceInputs, MarginInputs};
use bazaar_risk::snapshot::{PortfolioSnapshot, Position};
use bazaar_risk::types::{OrderIntent, Side, Tri};
use bazaar_risk::verdict::{GateVerdict, OverrideLevel, RuleId, RuleResult};
use chrono::{DateTime, NaiveDate, TimeZone};
use chrono_tz::{Asia::Kolkata, Tz};
use proptest::prelude::*;
use rust_decimal::Decimal;
use std::collections::BTreeMap;

const TEXT: &str = include_str!("../../../fixtures/config/risk.toml");

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn now() -> DateTime<Tz> {
    Kolkata.with_ymd_and_hms(2026, 11, 19, 11, 0, 0).unwrap()
}

struct At(DateTime<Tz>);
impl Clock for At {
    fn now_ist(&self) -> DateTime<Tz> {
        self.0
    }
}

struct Store;
impl LimitsBaseline for Store {
    fn read(&self) -> Result<Baseline, BaselineError> {
        Ok(Baseline::none())
    }
}

#[derive(Debug)]
struct Flat(Decimal);
impl CostModel for Flat {
    fn cost(&self, _s: Side, _q: u32, _p: Decimal) -> Tri<Decimal> {
        Tri::Known(self.0)
    }
}

fn instrument() -> Instrument {
    Instrument {
        instrument_token: 1,
        exchange_token: 1,
        tradingsymbol: "ABC".into(),
        name: "ABC".into(),
        exchange: Exchange::Nse,
        instrument_type: InstrumentType::Equity,
        segment: "NSE".into(),
        expiry: None,
        strike: None,
        tick_size: d("0.05"),
        lot_size: 1,
    }
}

/// A snapshot where each knob is either `Known` (with the given value) or `Unknown`.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)] // a generated-input record, not a domain type
struct Knobs {
    qty: u32,
    buy: bool,
    price: i64,
    stop_gap: Option<i64>,
    live: Option<i64>,
    known: [bool; 8],
    losses: u32,
    day_loss: i64,
    week_loss: i64,
    entries: u32,
    orders_min: u32,
    kill: bool,
    stale_inputs: bool,
    session_open: bool,
    /// 0 = unknown list, 1 = known and the symbol is absent, 2 = known and the symbol is banned.
    ban: u8,
    /// A held position in the same symbol (long), with this quantity.
    held: Option<u32>,
    /// A stop move on the intent (`new_stop`), as a gap below the price.
    new_stop_gap: Option<i64>,
    /// results window, policy day, option on its own expiry.
    events: [bool; 3],
    /// approval for this intent, compliance fresh, readiness report, owner opt-in.
    mode: [bool; 4],
    live_mode: bool,
}

fn knobs() -> impl Strategy<Value = Knobs> {
    (
        (
            1u32..5000,
            any::<bool>(),
            1i64..3000,
            prop::option::of(-50i64..200),
            prop::option::of(1i64..3000),
        ),
        (
            prop::array::uniform8(any::<bool>()),
            0u32..8,
            0i64..30000,
            0i64..80000,
            0u32..12,
            0u32..80,
            any::<bool>(),
            any::<bool>(),
            any::<bool>(),
        ),
        (
            0u8..3,
            prop::option::of(1u32..200),
            prop::option::of(1i64..100),
            prop::array::uniform3(any::<bool>()),
            prop::array::uniform4(any::<bool>()),
            any::<bool>(),
        ),
    )
        .prop_map(
            |(
                (qty, buy, price, stop_gap, live),
                (known, losses, day_loss, week_loss, entries, orders_min, kill, stale, open),
                (ban, held, new_stop_gap, events, mode, live_mode),
            )| Knobs {
                qty,
                buy,
                price,
                stop_gap,
                live,
                known,
                losses,
                day_loss,
                week_loss,
                entries,
                orders_min,
                kill,
                stale_inputs: stale,
                session_open: open,
                ban,
                held,
                new_stop_gap,
                events,
                mode,
                live_mode,
            },
        )
}

fn tri<T>(on: bool, v: T) -> Tri<T> {
    if on { Tri::Known(v) } else { Tri::Unknown }
}

fn series(end: NaiveDate) -> Vec<(NaiveDate, Decimal)> {
    (0..100u32)
        .map(|i| {
            let day = end - chrono::Duration::days(i64::from(99 - i));
            (day, Decimal::new(i64::from(i % 7) - 3, 3))
        })
        .collect()
}

struct Run {
    cfg: GateConfig,
    data: DataInputs,
    inst: Instrument,
    mode: ModeEvidence,
    exposure: ExposureInputs,
    cost: Flat,
    intent: OrderIntent,
    snap: PortfolioSnapshot,
    live: Tri<Decimal>,
}

#[allow(clippy::too_many_lines)] // one literal per input record
fn build(k: &Knobs) -> Run {
    let intent = OrderIntent {
        symbol: "ABC".into(),
        side: if k.buy { Side::Buy } else { Side::Sell },
        qty: Qty(k.qty),
        price: Decimal::new(k.price * 5, 2) + d("1"),
        // The protective stop is on the loss side: below a buy, above a sell.
        stop: k.stop_gap.map(|g| {
            let signed = if k.buy { -g } else { g };
            Decimal::new((k.price + signed) * 5, 2) + d("1")
        }),
        new_stop: k
            .new_stop_gap
            .map(|g| Decimal::new((k.price - g) * 5, 2) + d("1")),
        lot_size: 1,
        tick_size: d("0.05"),
    };
    let mut s = PortfolioSnapshot::unknown();
    s.as_of = tri(k.known[0], now());
    let held: Vec<Position> = k
        .held
        .map(|qty| Position {
            symbol: "ABC".into(),
            side: Side::Buy,
            qty,
            stop: Tri::Known(d("490")),
            risk_price: Tri::Known(d("500")),
            avg_cost: Tri::Known(d("500")),
        })
        .into_iter()
        .collect();
    s.positions = tri(k.known[1], held);
    s.consecutive_losses = tri(k.known[2], k.losses);
    s.day_pnl = tri(k.known[3], -Decimal::from(k.day_loss));
    s.week_pnl = tri(k.known[4], -Decimal::from(k.week_loss));
    s.week_start = tri(k.known[5], NaiveDate::from_ymd_opt(2026, 11, 16).unwrap());
    s.entries_today = tri(k.known[6], k.entries);
    s.orders_last_minute = tri(k.known[7], k.orders_min);
    s.orders_today = Tri::Known(0);
    s.overrides_today = Tri::Known(0);
    s.overrides_week = Tri::Known(0);
    s.kill_switch = Tri::Known(k.kill);
    s.charges_buffer = Tri::Known(d("20"));
    s.starting_cash = Tri::Known(d("250000"));
    s.realised_pnl = Tri::Known(Decimal::ZERO);
    s.equity_mtm = Tri::Known(d("250000"));
    s.marks = Tri::Known(vec![]);
    s.last_losing_exits = Tri::Known(vec![]);
    let quote_at = if k.stale_inputs {
        now() - chrono::Duration::hours(3)
    } else {
        now()
    };
    let data = DataInputs {
        inputs_as_of: Tri::Known(quote_at),
        evidence: EvidenceInputs {
            oos_trades: Tri::Known(50),
            profit_factor: Tri::Known(d("2")),
            expectancy_r: Tri::Known(d("0.3")),
            walk_forward_pass: Tri::Known(true),
        },
        ban_list: match k.ban {
            0 => Tri::Unknown,
            1 => Tri::Known(BanList {
                banned_underlyings: vec!["XYZ".into()],
                fetched_at: now(),
            }),
            _ => Tri::Known(BanList {
                banned_underlyings: vec!["ABC".into()],
                fetched_at: now(),
            }),
        },
        stock_fno_underlying: Tri::Known(None),
        events: EventFlags {
            results_window: Tri::Known(k.events[0]),
            policy_day: Tri::Known(k.events[1]),
            option_on_own_expiry: Tri::Known(k.events[2]),
        },
        quote_at: Tri::Known(quote_at),
        session_open: Tri::Known(k.session_open),
        margin: MarginInputs {
            qty: k.qty,
            required: Tri::Known(d("5000")),
            available: Tri::Known(d("250000")),
        },
    };
    Run {
        cfg: GateConfig::from_toml_str(TEXT).unwrap(),
        data,
        inst: instrument(),
        mode: ModeEvidence {
            approval: k.mode[0].then(|| IntentDigest::of(&intent)),
            compliance_fresh: Tri::Known(k.mode[1]),
            readiness_report: Tri::Known(k.mode[2]),
            owner_opt_in: Tri::Known(k.mode[3]),
            live: Tri::Known(k.live_mode),
        },
        exposure: ExposureInputs {
            sectors: BTreeMap::from([("ABC".to_string(), "IT".to_string())]),
            returns: BTreeMap::from([("ABC".to_string(), series(now().date_naive()))]),
        },
        cost: Flat(d("20")),
        intent,
        snap: s,
        live: k.live.map_or(Tri::Unknown, |l| {
            Tri::Known(Decimal::new(l * 5, 2) + d("1"))
        }),
    }
}

fn eval(r: &Run) -> Result<Evaluation, GateReject> {
    let inputs = GateInputs {
        cost: &r.cost,
        data: &r.data,
        instrument: Some(&r.inst),
        defined_max_loss: None,
        mode: &r.mode,
        entry_zone: Tri::Known(EntryZone {
            low: d("1"),
            high: d("100000"),
        }),
        live_price: r.live,
        exposure: Some(&r.exposure),
    };
    Gate::new(&r.cfg, &Store, &At(now())).evaluate(&r.intent, &r.snap, &inputs)
}

/// The only rejects the generated inputs may produce: a stop move combined with a quantity (or a
/// side) the classifier refuses. Anything else (symbol, baseline, verdict) is a real defect.
fn expected_reject(e: &GateReject) -> bool {
    matches!(e, GateReject::Classify(_))
}

/// Words that would imply certainty. Whole words only: "ensure" and "measure" are fine.
fn certainty_word(msg: &str) -> Option<String> {
    msg.split(|c: char| !c.is_alphanumeric())
        .map(str::to_ascii_lowercase)
        .find(|w| {
            matches!(
                w.as_str(),
                "safe"
                    | "safely"
                    | "safer"
                    | "sure"
                    | "surely"
                    | "guarantee"
                    | "guaranteed"
                    | "guarantees"
                    | "certain"
                    | "certainly"
            )
        })
}

fn check_message(r: &RuleResult) {
    let m = r.message();
    assert!(
        m.ends_with(&format!("({})", r.rule())),
        "message must end with its rule id: {m:?}"
    );
    assert_eq!(certainty_word(m), None, "certainty wording in {m:?}");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn every_rule_runs_and_messages_are_plain(k in knobs()) {
        let run = build(&k);
        match eval(&run) {
            Ok(e) => {
                let ids: Vec<RuleId> =
                    e.verdict().results().iter().map(RuleResult::rule).collect();
                prop_assert_eq!(ids, RuleId::EVALUATION_ORDER.to_vec());
                for r in e.verdict().results() {
                    check_message(r);
                }
            }
            Err(e) => prop_assert!(expected_reject(&e), "{:?}", e),
        }
    }

    #[test]
    fn hard_and_no_failures_are_never_overridable(k in knobs()) {
        let run = build(&k);
        let e = match eval(&run) {
            Ok(e) => e,
            Err(e) => {
                prop_assert!(expected_reject(&e), "{:?}", e);
                return Ok(());
            }
        };
        let v = e.verdict();
        let blocked = v.failures().any(|r| {
            r.is_input_missing()
                || matches!(r.level(), OverrideLevel::HardCeiling | OverrideLevel::No)
        });
        let level_blocked = v.failures().any(|r| {
            matches!(r.level(), OverrideLevel::HardCeiling | OverrideLevel::No)
        });
        if level_blocked {
            prop_assert!(!v.overridable());
        }
        if blocked {
            let at = At(now());
            let ch = OverrideChallenge::open(&at, &run.intent, v).unwrap();
            let req = OverrideRequest {
                challenge: ch,
                reason: ReasonType::Other,
                note: "a long and careful written reason for this one".into(),
            };
            let out = evaluate(req, &run.intent, v, &run.snap, e.ctx(), e.clock());
            prop_assert!(
                matches!(
                    out,
                    Err(OverrideError::NotOverridable(_) | OverrideError::InputMissing(_))
                ),
                "{:?}", out
            );
        }
    }

    #[test]
    fn the_same_inputs_give_the_same_verdict(k in knobs()) {
        let run = build(&k);
        let (a, b) = (eval(&run), eval(&run));
        match (a, b) {
            (Ok(a), Ok(b)) => {
                prop_assert_eq!(a.verdict(), b.verdict());
                prop_assert_eq!(a.limits_digest(), b.limits_digest());
                prop_assert_eq!(a.ref_price(), b.ref_price());
                prop_assert_eq!(a.risk_ref(), b.risk_ref());
            }
            (Err(a), Err(b)) => {
                prop_assert!(expected_reject(&a), "{:?}", a);
                prop_assert_eq!(a, b);
            }
            (a, b) => prop_assert!(false, "runs differ: {:?} vs {:?}", a.map(|_| ()), b.map(|_| ())),
        }
    }
}

fn verdict_with_one_failure(cfg: &GateConfig, bad: RuleId) -> GateVerdict {
    let results: Vec<RuleResult> = RuleId::EVALUATION_ORDER
        .iter()
        .map(|&id| {
            if id == bad {
                RuleResult::fail(id, "1", "0", "Over the limit.")
            } else {
                RuleResult::pass(id, "0", "1")
            }
        })
        .collect();
    GateVerdict::new(cfg.hash().clone(), &results).unwrap()
}

/// The override level of every rule, written out from the rule catalogue (bz-trading-discipline
/// G-table), never read from `override_level()`. G12 (liquidity) has no `RuleId` in F6.
/// X01-X03 are data, mode and structure checks the table does not list; they are `No`.
const CATALOGUE: [(&str, OverrideLevel); 22] = [
    ("G01", OverrideLevel::Paper),
    ("G02", OverrideLevel::HardCeiling),
    ("G03", OverrideLevel::HardCeiling),
    ("G04a", OverrideLevel::Soft),
    ("G04b", OverrideLevel::HardCeiling),
    ("G05", OverrideLevel::Soft),
    ("G06", OverrideLevel::No),
    ("G07", OverrideLevel::Soft),
    ("G08", OverrideLevel::No),
    ("G09", OverrideLevel::HardCeiling),
    ("G10", OverrideLevel::No),
    ("G11", OverrideLevel::HardCeiling),
    ("G13", OverrideLevel::No),
    ("G14", OverrideLevel::Soft),
    ("G15", OverrideLevel::No),
    ("G16", OverrideLevel::HardCeiling),
    ("G17", OverrideLevel::No),
    ("G18", OverrideLevel::No),
    ("G19", OverrideLevel::HardCeiling),
    ("X01", OverrideLevel::No),
    ("X02", OverrideLevel::No),
    ("X03", OverrideLevel::No),
];

#[test]
fn every_rule_has_its_catalogue_override_level() {
    for id in RuleId::EVALUATION_ORDER {
        let expected = CATALOGUE
            .iter()
            .find(|(name, _)| *name == id.as_str())
            .unwrap_or_else(|| panic!("{id} missing from the catalogue table"))
            .1;
        assert_eq!(id.override_level(), expected, "{id}");
    }
    // G01-G16 of the catalogue: 12 Hard or No (G12 has no RuleId, it is No) and 5 open.
    let in_g_range = |n: &str| n.starts_with('G') && n[1..3].parse::<u8>().is_ok_and(|v| v <= 16);
    let hard_or_no = CATALOGUE
        .iter()
        .filter(|(n, l)| in_g_range(n) && !l.is_overridable())
        .count()
        + 1; // G12
    let open = CATALOGUE
        .iter()
        .filter(|(n, l)| in_g_range(n) && l.is_overridable())
        .count();
    assert_eq!((hard_or_no, open), (12, 5));
}

#[test]
fn whole_table_hard_and_no_rules_make_a_verdict_non_overridable() {
    let cfg = GateConfig::from_toml_str(TEXT).unwrap();
    for (name, level) in CATALOGUE {
        let id = RuleId::EVALUATION_ORDER
            .into_iter()
            .find(|r| r.as_str() == name)
            .unwrap();
        let v = verdict_with_one_failure(&cfg, id);
        if level.is_overridable() {
            assert!(v.overridable(), "{id} is the overridable kind");
            continue;
        }
        assert!(!v.overridable(), "{id} must never be overridable");
        // A Hard/No failure alongside an overridable one still blocks the whole verdict.
        let mut results: Vec<RuleResult> = v.results().to_vec();
        for r in &mut results {
            if r.rule() == RuleId::G04a {
                *r = RuleResult::fail(RuleId::G04a, "1", "0", "Over the limit.");
            }
        }
        let both = GateVerdict::new(cfg.hash().clone(), &results).unwrap();
        assert!(!both.overridable(), "{id} + G04a");
    }
}

#[test]
fn input_missing_is_never_overridable_even_on_a_soft_rule() {
    let cfg = GateConfig::from_toml_str(TEXT).unwrap();
    let soft = RuleId::EVALUATION_ORDER
        .into_iter()
        .find(|r| r.override_level() == OverrideLevel::Soft)
        .unwrap();
    let results: Vec<RuleResult> = RuleId::EVALUATION_ORDER
        .iter()
        .map(|&id| {
            if id == soft {
                RuleResult::input_missing(id, "unknown", "known", "The input is missing.")
            } else {
                RuleResult::pass(id, "0", "1")
            }
        })
        .collect();
    let v = GateVerdict::new(cfg.hash().clone(), &results).unwrap();
    let f = v.failures().next().unwrap();
    assert!(f.is_input_missing());
    check_message(f);
    assert!(
        !v.overridable(),
        "{soft} with a missing input must not be overridable"
    );
    // The override engine refuses it too, with the dedicated error.
    let run = build(&clean());
    let e = eval(&run).unwrap();
    let v = v.bound_to(IntentDigest::of(&run.intent));
    let at = At(now());
    let ch = OverrideChallenge::open(&at, &run.intent, &v).unwrap();
    let req = OverrideRequest {
        challenge: ch,
        reason: ReasonType::Other,
        note: "a long and careful written reason for this one".into(),
    };
    let out = evaluate(req, &run.intent, &v, &run.snap, e.ctx(), e.clock());
    assert_eq!(out, Err(OverrideError::InputMissing(soft)));
}

/// Regression: `GateVerdict::overridable()` once ignored `input_missing`, so a
/// lone input-missing failure on a Soft rule reports true although `override_::evaluate` refuses
/// it. Fixed in verdict.rs.
#[test]
fn verdict_overridable_ignores_input_missing() {
    let cfg = GateConfig::from_toml_str(TEXT).unwrap();
    let results: Vec<RuleResult> = RuleId::EVALUATION_ORDER
        .iter()
        .map(|&id| {
            if id == RuleId::G07 {
                RuleResult::input_missing(id, "unknown", "known", "The input is missing.")
            } else {
                RuleResult::pass(id, "0", "1")
            }
        })
        .collect();
    let v = GateVerdict::new(cfg.hash().clone(), &results).unwrap();
    assert!(!v.overridable());
}

#[test]
fn certainty_check_catches_words_and_spares_ensure() {
    assert!(certainty_word("This is safe (G01)").is_some());
    assert!(certainty_word("Guaranteed fill (G01)").is_some());
    assert!(certainty_word("Make sure of it (G01)").is_some());
    assert!(certainty_word("Ensure the measure (G01)").is_none());
}

fn clean() -> Knobs {
    Knobs {
        qty: 10,
        buy: true,
        price: 100,
        stop_gap: Some(10),
        live: Some(100),
        known: [true; 8],
        losses: 0,
        day_loss: 0,
        week_loss: 0,
        entries: 0,
        orders_min: 0,
        kill: false,
        stale_inputs: false,
        session_open: true,
        ban: 1,
        held: None,
        new_stop_gap: None,
        events: [false; 3],
        mode: [false; 4],
        live_mode: false,
    }
}

#[test]
fn clean_and_unknown_snapshots_both_follow_the_message_contract() {
    for known in [[true; 8], [false; 8]] {
        let run = build(&Knobs { known, ..clean() });
        let e = eval(&run).expect("a known-good input must produce a verdict, not a reject");
        assert_eq!(e.verdict().results().len(), RuleId::EVALUATION_ORDER.len());
        e.verdict().results().iter().for_each(check_message);
    }
}

#[test]
fn sells_and_stop_moves_are_driven_by_the_generator() {
    let sell = build(&Knobs {
        buy: false,
        ..clean()
    });
    assert!(sell.intent.stop.unwrap() > sell.intent.price - d("0.01"));
    assert!(eval(&sell).is_ok());
    let held = build(&Knobs {
        held: Some(5),
        ban: 2,
        ..clean()
    });
    assert!(eval(&held).is_ok());
}
