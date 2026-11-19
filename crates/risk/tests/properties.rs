//! Cross-rule properties of the gate: every rule runs, Hard and No rules are never overridable,
//! messages are plain and end with their rule id, and the verdict is deterministic.
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
use bazaar_risk::gate::{CostModel, Gate, GateInputs, ModeEvidence};
use bazaar_risk::override_::{
    OverrideChallenge, OverrideError, OverrideRequest, ReasonType, evaluate,
};
use bazaar_risk::rules::behaviour::EntryZone;
use bazaar_risk::rules::exposure::ExposureInputs;
use bazaar_risk::rules::inputs::{DataInputs, EventFlags, EvidenceInputs, MarginInputs};
use bazaar_risk::snapshot::PortfolioSnapshot;
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
    )
        .prop_map(
            |(
                (qty, buy, price, stop_gap, live),
                (known, losses, day_loss, week_loss, entries, orders_min, kill, stale, open),
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

fn build(k: &Knobs) -> Run {
    let intent = OrderIntent {
        symbol: "ABC".into(),
        side: if k.buy { Side::Buy } else { Side::Sell },
        qty: Qty(k.qty),
        price: Decimal::new(k.price * 5, 2) + d("1"),
        stop: k
            .stop_gap
            .map(|g| Decimal::new(k.price * 5 - g * 5, 2) + d("1")),
        new_stop: None,
        lot_size: 1,
        tick_size: d("0.05"),
    };
    let mut s = PortfolioSnapshot::unknown();
    s.as_of = tri(k.known[0], now());
    s.positions = tri(k.known[1], vec![]);
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
        ban_list: Tri::Unknown,
        stock_fno_underlying: Tri::Known(None),
        events: EventFlags {
            results_window: Tri::Known(false),
            policy_day: Tri::Known(false),
            option_on_own_expiry: Tri::Known(false),
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
            approval: None,
            compliance_fresh: Tri::Known(false),
            readiness_report: Tri::Known(false),
            owner_opt_in: Tri::Known(false),
            live: Tri::Known(false),
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

fn eval(r: &Run) -> Option<bazaar_risk::gate::Evaluation> {
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
    Gate::new(&r.cfg, &Store, &At(now()))
        .evaluate(&r.intent, &r.snap, &inputs)
        .ok()
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
        if let Some(e) = eval(&run) {
            let ids: Vec<RuleId> = e.verdict().results().iter().map(RuleResult::rule).collect();
            prop_assert_eq!(ids, RuleId::EVALUATION_ORDER.to_vec());
            for r in e.verdict().results() {
                check_message(r);
            }
        }
    }

    #[test]
    fn hard_and_no_failures_are_never_overridable(k in knobs()) {
        let run = build(&k);
        let Some(e) = eval(&run) else { return Ok(()); };
        let v = e.verdict();
        let blocked = v.failures().any(|r| {
            r.is_input_missing()
                || matches!(r.level(), OverrideLevel::HardCeiling | OverrideLevel::No)
        });
        if blocked {
            prop_assert!(!v.overridable());
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
            (Some(a), Some(b)) => {
                prop_assert_eq!(a.verdict(), b.verdict());
                prop_assert_eq!(a.limits_digest(), b.limits_digest());
                prop_assert_eq!(a.ref_price(), b.ref_price());
                prop_assert_eq!(a.risk_ref(), b.risk_ref());
            }
            (None, None) => {}
            _ => prop_assert!(false, "one run rejected and the other did not"),
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

#[test]
fn whole_table_hard_and_no_rules_make_a_verdict_non_overridable() {
    let cfg = GateConfig::from_toml_str(TEXT).unwrap();
    let (mut hard_or_no, mut open) = (0, 0);
    for id in RuleId::EVALUATION_ORDER {
        let v = verdict_with_one_failure(&cfg, id);
        match id.override_level() {
            OverrideLevel::HardCeiling | OverrideLevel::No => {
                hard_or_no += 1;
                assert!(!v.overridable(), "{id} must never be overridable");
            }
            OverrideLevel::Soft | OverrideLevel::Paper => {
                open += 1;
                assert!(v.overridable(), "{id} is the overridable kind");
            }
        }
        // A Hard/No failure alongside an overridable one still blocks the whole verdict.
        if !id.override_level().is_overridable() {
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
    assert!(hard_or_no > 0 && open > 0);
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
}

#[test]
fn certainty_check_catches_words_and_spares_ensure() {
    assert!(certainty_word("This is safe (G01)").is_some());
    assert!(certainty_word("Guaranteed fill (G01)").is_some());
    assert!(certainty_word("Make sure of it (G01)").is_some());
    assert!(certainty_word("Ensure the measure (G01)").is_none());
}

#[test]
fn clean_and_unknown_snapshots_both_follow_the_message_contract() {
    let k = Knobs {
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
    };
    for known in [[true; 8], [false; 8]] {
        let run = build(&Knobs { known, ..k.clone() });
        let e = eval(&run).unwrap();
        assert_eq!(e.verdict().results().len(), RuleId::EVALUATION_ORDER.len());
        e.verdict().results().iter().for_each(check_message);
    }
}
