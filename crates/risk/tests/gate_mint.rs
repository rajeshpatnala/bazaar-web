//! Gate mint tests: sizing at the final quantity, audit before mint, single use, only the gate mints.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use bazaar_core::clock::Clock;
use bazaar_core::instrument::{Exchange, Instrument, InstrumentType};
use bazaar_core::money::Qty;
use bazaar_risk::audit::{AuditError, AuditRecord, AuditSink};
use bazaar_risk::baseline::{BaselineError, LimitsBaseline};
use bazaar_risk::config::{Baseline, GateConfig};
use bazaar_risk::gate::{
    CostModel, Decision, Gate, GateInputs, GatedIntent, MintError, ModeEvidence,
};
use bazaar_risk::override_::{OverrideChallenge, OverrideError, OverrideRequest, ReasonType};
use bazaar_risk::rules::behaviour::EntryZone;
use bazaar_risk::rules::exposure::ExposureInputs;
use bazaar_risk::rules::inputs::{DataInputs, EventFlags, EvidenceInputs, MarginInputs};
use bazaar_risk::snapshot::PortfolioSnapshot;
use bazaar_risk::types::{OrderIntent, Side, Tri};
use bazaar_risk::verdict::RuleId;
use chrono::{DateTime, Duration, NaiveDate, TimeZone};
use chrono_tz::{Asia::Kolkata, Tz};
use rust_decimal::Decimal;
use std::cell::RefCell;
use std::collections::BTreeMap;

const TEXT: &str = include_str!("../../../fixtures/config/risk.toml");

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn now() -> DateTime<Tz> {
    Kolkata.with_ymd_and_hms(2026, 11, 19, 11, 0, 0).unwrap()
}

struct At;
impl Clock for At {
    fn now_ist(&self) -> DateTime<Tz> {
        now()
    }
}

struct Then(DateTime<Tz>);
impl Clock for Then {
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

/// Fake F9 cost: `base + per * qty`; `None` base = unknown.
#[derive(Debug)]
struct Cost(Option<Decimal>, Decimal);
impl CostModel for Cost {
    fn cost(&self, _side: Side, qty: u32, _price: Decimal) -> Tri<Decimal> {
        self.0.map_or(Tri::Unknown, |b| {
            Tri::Known(b + self.1 * Decimal::from(qty))
        })
    }
}

/// Records writes; fails when told to.
struct Sink {
    fail: bool,
    writes: RefCell<Vec<String>>,
}
impl Sink {
    fn new(fail: bool) -> Self {
        Self {
            fail,
            writes: RefCell::new(vec![]),
        }
    }
}
impl AuditSink for Sink {
    fn write(&self, r: &AuditRecord) -> Result<(), AuditError> {
        if self.fail {
            return Err(AuditError::new("down"));
        }
        self.writes
            .borrow_mut()
            .push(r.intent_digest().as_str().to_owned());
        Ok(())
    }
}

fn intent(qty: u32) -> OrderIntent {
    OrderIntent {
        symbol: "ABC".into(),
        side: Side::Buy,
        qty: Qty(qty),
        price: d("50"),
        stop: Some(d("40")),
        new_stop: None,
        lot_size: 1,
        tick_size: d("0.05"),
    }
}

fn snap() -> PortfolioSnapshot {
    let mut s = PortfolioSnapshot::unknown();
    s.as_of = Tri::Known(now());
    s.positions = Tri::Known(vec![]);
    s.consecutive_losses = Tri::Known(0);
    s.day_pnl = Tri::Known(Decimal::ZERO);
    s.week_pnl = Tri::Known(Decimal::ZERO);
    s.week_start = Tri::Known(NaiveDate::from_ymd_opt(2026, 11, 16).unwrap());
    s.entries_today = Tri::Known(0);
    s.orders_last_minute = Tri::Known(0);
    s.orders_today = Tri::Known(0);
    s.overrides_today = Tri::Known(0);
    s.overrides_week = Tri::Known(0);
    s.kill_switch = Tri::Known(false);
    s.charges_buffer = Tri::Known(d("20"));
    s.starting_cash = Tri::Known(d("250000"));
    s.realised_pnl = Tri::Known(Decimal::ZERO);
    s.equity_mtm = Tri::Known(d("250000"));
    s.marks = Tri::Known(vec![]);
    s.last_losing_exits = Tri::Known(vec![]);
    s
}

struct Env {
    cfg: GateConfig,
    data: DataInputs,
    inst: Instrument,
    mode: ModeEvidence,
    exposure: ExposureInputs,
    cost: Cost,
}

impl Env {
    fn new(oos: u32) -> Self {
        let end = now().date_naive();
        let series = (0..100u32)
            .map(|i| {
                let day = end - Duration::days(i64::from(99 - i));
                (day, Decimal::new(i64::from(i % 7) - 3, 3))
            })
            .collect();
        Self {
            cfg: GateConfig::from_toml_str(TEXT).unwrap(),
            data: DataInputs {
                inputs_as_of: Tri::Known(now()),
                evidence: EvidenceInputs {
                    oos_trades: Tri::Known(oos),
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
                quote_at: Tri::Known(now()),
                session_open: Tri::Known(true),
                margin: MarginInputs {
                    qty: 10,
                    required: Tri::Known(d("5000")),
                    available: Tri::Known(d("250000")),
                },
            },
            inst: Instrument {
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
            },
            mode: ModeEvidence {
                approval: None,
                compliance_fresh: Tri::Known(false),
                readiness_report: Tri::Known(false),
                owner_opt_in: Tri::Known(false),
                live: Tri::Known(false),
            },
            exposure: ExposureInputs {
                sectors: BTreeMap::from([("ABC".to_string(), "IT".to_string())]),
                returns: BTreeMap::from([("ABC".to_string(), series)]),
            },
            cost: Cost(Some(d("20")), Decimal::ZERO),
        }
    }

    fn inputs(&self) -> GateInputs<'_> {
        GateInputs {
            cost: &self.cost,
            data: &self.data,
            instrument: Some(&self.inst),
            defined_max_loss: None,
            mode: &self.mode,
            entry_zone: Tri::Known(EntryZone {
                low: d("49.9"),
                high: d("50.1"),
            }),
            live_price: Tri::Known(d("50")),
            exposure: Some(&self.exposure),
        }
    }

    fn decide(
        &self,
        gate: &Gate<'_>,
        i: &OrderIntent,
        req: Option<OverrideRequest>,
        sink: &Sink,
    ) -> Result<Decision, MintError> {
        gate.decide(i, &snap(), &self.inputs(), req, sink)
    }

    /// A request opened on a fresh verdict, after the confirm delay.
    fn request(&self, gate: &Gate<'_>, i: &OrderIntent) -> OverrideRequest {
        let e = gate.evaluate(i, &snap(), &self.inputs()).unwrap();
        let wait = i64::from(e.ctx().limits.override_confirm_delay_secs);
        OverrideRequest {
            challenge: OverrideChallenge::open(
                &Then(now() - Duration::seconds(wait)),
                i,
                e.verdict(),
            )
            .unwrap(),
            reason: ReasonType::NewInformation,
            note: "Fresh results changed my view of the evidence here.".into(),
        }
    }
}

fn minted(r: Result<Decision, MintError>) -> GatedIntent {
    match r {
        Ok(Decision::Minted(g)) => g,
        other => panic!("expected a mint, got {other:?}"),
    }
}

fn refused_rules(r: Result<Decision, MintError>) -> Vec<RuleId> {
    match r {
        Ok(Decision::Refused(e)) => e
            .verdict()
            .failures()
            .map(bazaar_risk::verdict::RuleResult::rule)
            .collect(),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn a_passing_entry_mints_with_gate_derived_figures() {
    let env = Env::new(50);
    let gate = Gate::new(&env.cfg, &Store, &At);
    let g = minted(env.decide(&gate, &intent(10), None, &Sink::new(false)));
    // 10 x (50 - 40) + Rs 20 charges.
    assert_eq!(
        (g.qty(), g.risk_rupees(), g.overridden()),
        (10, d("120"), false)
    );
    assert_eq!(g.evaluated_at(), now());
    assert_eq!(g.config_hash(), env.cfg.hash());
    assert_eq!((g.symbol(), g.side()), ("ABC", Side::Buy));
}

#[test]
fn g02_and_sizing_use_the_cost_of_the_final_quantity() {
    let mut env = Env::new(50);
    // Cost Rs 20 + Re 1 per share: 111 x 10 + 131 = 1,241 fits Rs 1,250; 112 x 10 + 132 does not,
    // although a flat Rs 20 would let it through.
    env.cost = Cost(Some(d("20")), Decimal::ONE);
    let gate = Gate::new(&env.cfg, &Store, &At);
    env.data.margin.qty = 111;
    let g = minted(env.decide(&gate, &intent(111), None, &Sink::new(false)));
    assert_eq!(g.risk_rupees(), d("1241"));
    env.data.margin.qty = 112;
    let rules = refused_rules(env.decide(&gate, &intent(112), None, &Sink::new(false)));
    assert_eq!(rules, vec![RuleId::G02]);
    // An unknown cost rejects the entry.
    env.cost = Cost(None, Decimal::ZERO);
    let rules = refused_rules(env.decide(&gate, &intent(10), None, &Sink::new(false)));
    assert!(rules.contains(&RuleId::G02));
}

#[test]
fn option_entries_are_not_sized_or_minted() {
    let mut env = Env::new(50);
    env.inst.instrument_type = InstrumentType::Call;
    env.inst.expiry = NaiveDate::from_ymd_opt(2026, 11, 26);
    env.inst.strike = Some(d("500"));
    let gate = Gate::new(&env.cfg, &Store, &At);
    let r = env.decide(&gate, &intent(10), None, &Sink::new(false));
    assert!(
        matches!(r, Err(MintError::NotSizable) | Ok(Decision::Refused(_))),
        "{r:?}"
    );
}

#[test]
fn a_failing_verdict_without_an_override_is_refused() {
    let env = Env::new(5);
    let gate = Gate::new(&env.cfg, &Store, &At);
    let rules = refused_rules(env.decide(&gate, &intent(10), None, &Sink::new(false)));
    assert_eq!(rules, vec![RuleId::G01]);
}

#[test]
fn an_override_on_a_passing_verdict_is_refused() {
    let env = Env::new(50);
    let gate = Gate::new(&env.cfg, &Store, &At);
    let req = env.request(&gate, &intent(10));
    let r = env.decide(&gate, &intent(10), Some(req), &Sink::new(false));
    assert!(matches!(
        r,
        Err(MintError::Override(OverrideError::NothingToOverride))
    ));
}

#[test]
fn override_writes_the_audit_record_then_mints_and_is_single_use() {
    let env = Env::new(5);
    let gate = Gate::new(&env.cfg, &Store, &At);
    let i = intent(10);
    // A sink error mints nothing and does not burn the digest.
    let bad = Sink::new(true);
    let r = env.decide(&gate, &i, Some(env.request(&gate, &i)), &bad);
    assert!(matches!(r, Err(MintError::Audit(_))), "{r:?}");
    let sink = Sink::new(false);
    let g = minted(env.decide(&gate, &i, Some(env.request(&gate, &i)), &sink));
    assert!(g.overridden());
    assert_eq!(
        sink.writes.borrow().as_slice(),
        [g.intent_digest().as_str()]
    );
    // Two requests for one digest: the second is refused and writes nothing.
    let r = env.decide(&gate, &i, Some(env.request(&gate, &i)), &sink);
    assert!(matches!(r, Err(MintError::OverrideUsed)), "{r:?}");
    assert_eq!(sink.writes.borrow().len(), 1);
}

#[test]
fn an_override_never_mints_more_than_the_reduced_size() {
    let env = Env::new(5);
    let gate = Gate::new(&env.cfg, &Store, &At);
    // 200 shares fail G02 (and G01); a Soft override mints at most the reduced gate size.
    let i = intent(200);
    let r = env.decide(&gate, &i, Some(env.request(&gate, &i)), &Sink::new(false));
    match r {
        Ok(Decision::Minted(g)) => assert!(g.qty() < 123 && g.overridden(), "{}", g.qty()),
        Err(MintError::Override(_)) => {} // the failed rule set is not overridable here
        other => panic!("{other:?}"),
    }
}

#[test]
fn only_gate_rs_builds_a_gated_intent() {
    fn scan(dir: &std::path::Path, hits: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                scan(&p, hits);
            } else if p.extension().is_some_and(|x| x == "rs")
                && p.file_name().is_some_and(|n| n != "gate.rs")
                && std::fs::read_to_string(&p)
                    .unwrap()
                    .contains("GatedIntent {")
            {
                hits.push(p.display().to_string());
            }
        }
    }
    let mut hits = vec![];
    scan(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut hits,
    );
    assert!(hits.is_empty(), "{hits:?}");
}
