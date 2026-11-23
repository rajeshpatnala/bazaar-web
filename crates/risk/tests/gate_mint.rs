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
use bazaar_risk::override_::{
    IntentDigest, OverrideChallenge, OverrideError, OverrideRequest, ReasonType,
};
use bazaar_risk::rules::behaviour::EntryZone;
use bazaar_risk::rules::exposure::ExposureInputs;
use bazaar_risk::rules::inputs::{DataInputs, EventFlags, EvidenceInputs, MarginInputs};
use bazaar_risk::snapshot::{Mark, PortfolioSnapshot, Position};
use bazaar_risk::types::{OrderIntent, Side, Tri};
use bazaar_risk::verdict::RuleId;
use chrono::{DateTime, Duration, NaiveDate, TimeZone};
use chrono_tz::{Asia::Kolkata, Tz};
use rust_decimal::Decimal;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

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
    cost: Box<dyn CostModel>,
    live: Decimal,
    held: Vec<Position>,
}

impl Env {
    fn new(oos: u32) -> Self {
        Self::with_text(TEXT, oos)
    }

    fn with_text(text: &str, oos: u32) -> Self {
        let end = now().date_naive();
        let series = (0..100u32)
            .map(|i| {
                let day = end - Duration::days(i64::from(99 - i));
                (day, Decimal::new(i64::from(i % 7) - 3, 3))
            })
            .collect();
        Self {
            cfg: GateConfig::from_toml_str(text).unwrap(),
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
            cost: Box::new(Cost(Some(d("20")), Decimal::ZERO)),
            live: d("50"),
            held: vec![],
        }
    }

    fn snapshot(&self) -> PortfolioSnapshot {
        let mut s = snap();
        s.positions = Tri::Known(self.held.clone());
        s.marks = Tri::Known(
            self.held
                .iter()
                .map(|p| Mark {
                    symbol: p.symbol.clone(),
                    price: d("50"),
                })
                .collect(),
        );
        s
    }

    fn inputs(&self) -> GateInputs<'_> {
        GateInputs {
            cost: &*self.cost,
            data: &self.data,
            instrument: Some(&self.inst),
            defined_max_loss: None,
            mode: &self.mode,
            entry_zone: Tri::Known(EntryZone {
                low: d("49.5"),
                high: d("50.1"),
            }),
            live_price: Tri::Known(self.live),
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
        gate.decide(i, &self.snapshot(), &self.inputs(), req, sink)
    }

    /// A request opened on a fresh verdict, after the confirm delay.
    fn request(&self, gate: &Gate<'_>, i: &OrderIntent) -> OverrideRequest {
        let e = gate.evaluate(i, &self.snapshot(), &self.inputs()).unwrap();
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

/// Cost that is cheap except for one share, so the one-lot estimate is far above the final cost.
#[derive(Debug)]
struct Spiky;
impl CostModel for Spiky {
    fn cost(&self, _: Side, qty: u32, _: Decimal) -> Tri<Decimal> {
        Tri::Known(d(if qty == 1 { "600" } else { "20" }))
    }
}

/// Cheap for the first `after` calls, dear afterwards: models a non-monotonic cost model.
#[derive(Debug)]
struct Jump {
    calls: Arc<AtomicU32>,
    after: u32,
}
impl CostModel for Jump {
    fn cost(&self, _: Side, _: u32, _: Decimal) -> Tri<Decimal> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        Tri::Known(d(if n >= self.after { "2000" } else { "20" }))
    }
}

fn sell(q: u32) -> OrderIntent {
    OrderIntent {
        side: Side::Sell,
        stop: Some(d("60")),
        ..intent(q)
    }
}

/// Sets the margin quantity to the intent's, as a caller must.
fn sized_for(i: OrderIntent, m: &mut MarginInputs) -> OrderIntent {
    m.qty = i.qty.0;
    i
}

fn held_long(qty: u32) -> Position {
    Position {
        symbol: "ABC".into(),
        side: Side::Buy,
        qty,
        stop: Tri::Known(d("40")),
        risk_price: Tri::Known(d("50")),
        avg_cost: Tri::Known(d("50")),
    }
}

fn minted(r: Result<Decision, MintError>) -> GatedIntent {
    match r {
        Ok(Decision::Minted(g)) => g,
        Ok(Decision::Refused(e)) => {
            let why: Vec<_> = e
                .verdict()
                .failures()
                .map(|f| f.message().to_owned())
                .collect();
            panic!("expected a mint, refused: {why:?}")
        }
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
fn minted_risk_uses_the_floor_when_the_real_cost_is_below_it() {
    // QUAN-153-1: cost Rs 5 is under the loaded floor; minted risk = qty x per-unit + floor, the
    // same allowance sizing reserved (not the raw Rs 5).
    let mut env = Env::new(50);
    let floor = env
        .cfg
        .effective(&Baseline::none())
        .min_charges_slippage_buffer;
    assert!(floor > d("5"));
    env.cost = Box::new(Cost(Some(d("5")), Decimal::ZERO));
    let gate = Gate::new(&env.cfg, &Store, &At);
    let g = minted(env.decide(&gate, &intent(10), None, &Sink::new(false)));
    assert_eq!(g.qty(), 10);
    assert_eq!(g.risk_rupees(), d("100") + floor);
    // A cost above the floor stays the cost.
    env.cost = Box::new(Cost(Some(d("20")), Decimal::ZERO));
    let gate = Gate::new(&env.cfg, &Store, &At);
    let g = minted(env.decide(&gate, &intent(10), None, &Sink::new(false)));
    assert_eq!(g.risk_rupees(), d("120"));
}

#[test]
fn g02_and_sizing_use_the_cost_of_the_final_quantity() {
    let mut env = Env::new(50);
    // Cost Rs 20 + Re 1 per share: 111 x 10 + 131 = 1,241 fits Rs 1,250; 112 x 10 + 132 does not,
    // although a flat Rs 20 would let it through.
    env.cost = Box::new(Cost(Some(d("20")), Decimal::ONE));
    let gate = Gate::new(&env.cfg, &Store, &At);
    let i = sized_for(intent(111), &mut env.data.margin);
    let g = minted(env.decide(&gate, &i, None, &Sink::new(false)));
    assert_eq!(g.risk_rupees(), d("1241"));
    let i = sized_for(intent(112), &mut env.data.margin);
    let rules = refused_rules(env.decide(&gate, &i, None, &Sink::new(false)));
    assert_eq!(rules, vec![RuleId::G02]);
    // An unknown cost rejects the entry.
    env.cost = Box::new(Cost(None, Decimal::ZERO));
    let i = sized_for(intent(10), &mut env.data.margin);
    let rules = refused_rules(env.decide(&gate, &i, None, &Sink::new(false)));
    assert!(rules.contains(&RuleId::G02));
}

#[test]
fn sizing_uses_the_side_aware_risk_reference() {
    // (side, live, qty that fits, rupee risk). Budget Rs 1,250; one share more must fail G02.
    let cases = [
        (Side::Sell, "49.80", 120, "1244"),
        (Side::Sell, "50", 123, "1250"),
        (Side::Sell, "50.05", 123, "1250"),
        (Side::Buy, "50.05", 122, "1246.10"),
    ];
    for (side, live, qty, risk) in cases {
        let mut env = Env::new(50);
        env.live = d(live);
        let gate = Gate::new(&env.cfg, &Store, &At);
        let mk = |q| {
            if side == Side::Buy {
                intent(q)
            } else {
                sell(q)
            }
        };
        let i = sized_for(mk(qty), &mut env.data.margin);
        let g = minted(env.decide(&gate, &i, None, &Sink::new(false)));
        assert_eq!(
            (g.qty(), g.risk_rupees()),
            (qty, d(risk)),
            "{side:?} {live}"
        );
        let i = sized_for(mk(qty + 1), &mut env.data.margin);
        let rules = refused_rules(env.decide(&gate, &i, None, &Sink::new(false)));
        assert!(rules.contains(&RuleId::G02), "{side:?} {live}: {rules:?}");
    }
}

#[test]
fn an_option_entry_is_not_sized() {
    let mut env = Env::new(50);
    env.inst.instrument_type = InstrumentType::Call;
    env.inst.expiry = NaiveDate::from_ymd_opt(2026, 11, 26);
    env.inst.strike = Some(d("500"));
    let gate = Gate::new(&env.cfg, &Store, &At);
    let r = env.decide(&gate, &intent(10), None, &Sink::new(false));
    assert!(matches!(r, Err(MintError::NotSizable)), "{r:?}");
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
fn the_gate_counts_its_own_overrides_against_the_day_cap_on_one_snapshot() {
    let mut env = Env::new(5);
    let gate = Gate::new(&env.cfg, &Store, &At);
    let sink = Sink::new(false);
    // The fixture allows 2 a day; the snapshot says 0 used and never changes.
    for q in [10, 11] {
        let i = sized_for(intent(q), &mut env.data.margin);
        minted(env.decide(&gate, &i, Some(env.request(&gate, &i)), &sink));
    }
    let i = sized_for(intent(12), &mut env.data.margin);
    let r = env.decide(&gate, &i, Some(env.request(&gate, &i)), &sink);
    let Err(e @ MintError::Override(OverrideError::DayCapReached { used: 2, max: 2 })) = r else {
        panic!("expected the day cap, got {r:?}");
    };
    assert!(!e.to_string().contains("DayCap"), "{e}");
    assert_eq!(sink.writes.borrow().len(), 2);
}

#[test]
fn an_override_mints_the_reduced_size_only() {
    let mut env = Env::new(5);
    let gate = Gate::new(&env.cfg, &Store, &At);
    // 123 is the full gate size (Rs 1,230 + Rs 20 = Rs 1,250); G01 is the only (Paper) failure.
    // Size factor 0.4 floors 123 lots to 49: 49 x 10 + 20 = Rs 510.
    let i = sized_for(intent(123), &mut env.data.margin);
    let req = env.request(&gate, &i);
    let g = minted(env.decide(&gate, &i, Some(req), &Sink::new(false)));
    assert_eq!(
        (g.qty(), g.risk_rupees(), g.overridden()),
        (49, d("510"), true)
    );
    // Short with live Rs 49.80 (risk 10.20 a share): gate size 120, reduced to 48, not 49.
    env.live = d("49.80");
    let gate = Gate::new(&env.cfg, &Store, &At);
    let i = sized_for(sell(120), &mut env.data.margin);
    let req = env.request(&gate, &i);
    let g = minted(env.decide(&gate, &i, Some(req), &Sink::new(false)));
    assert_eq!((g.qty(), g.risk_rupees()), (48, d("509.60")));
}

#[test]
fn a_hard_failure_or_a_paper_row_outside_paper_cannot_be_overridden() {
    let mut env = Env::new(50);
    let gate = Gate::new(&env.cfg, &Store, &At);
    let i = sized_for(intent(124), &mut env.data.margin);
    let req = env.request(&gate, &i);
    let r = env.decide(&gate, &i, Some(req), &Sink::new(false));
    assert!(
        matches!(
            r,
            Err(MintError::Override(OverrideError::NotOverridable(_)))
        ),
        "{r:?}"
    );
    // G01 is a Paper row: refused in semi_auto even with an approval bound to the order.
    let semi = TEXT.replace(
        "execution_mode = \"paper\"",
        "execution_mode = \"semi_auto\"",
    );
    let mut env = Env::with_text(&semi, 5);
    let i = intent(10);
    env.mode.approval = Some(IntentDigest::of(&i));
    let gate = Gate::new(&env.cfg, &Store, &At);
    let req = env.request(&gate, &i);
    let r = env.decide(&gate, &i, Some(req), &Sink::new(false));
    assert!(
        matches!(
            r,
            Err(MintError::Override(OverrideError::PaperOnly(RuleId::G01)))
        ),
        "{r:?}"
    );
}

#[test]
fn an_order_above_the_gate_size_is_oversized() {
    let mut env = Env::new(50);
    // One share costs Rs 600, so the one-lot estimate allows 65; 100 shares cost Rs 20 and pass G02.
    env.cost = Box::new(Spiky);
    let gate = Gate::new(&env.cfg, &Store, &At);
    let i = sized_for(intent(100), &mut env.data.margin);
    let r = env.decide(&gate, &i, None, &Sink::new(false));
    assert!(
        matches!(r, Err(MintError::OverSized { allowed: 65 })),
        "{r:?}"
    );
}

#[test]
fn a_cost_that_rises_at_the_mint_cannot_push_risk_over_budget() {
    let mut env = Env::new(50);
    let calls = Arc::new(AtomicU32::new(0));
    env.cost = Box::new(Jump {
        calls: calls.clone(),
        after: u32::MAX,
    });
    let gate = Gate::new(&env.cfg, &Store, &At);
    minted(env.decide(&gate, &intent(10), None, &Sink::new(false)));
    let total = calls.load(Ordering::SeqCst);
    // The last cost call of a decide is the mint's own.
    env.cost = Box::new(Jump {
        calls: Arc::new(AtomicU32::new(0)),
        after: total - 1,
    });
    let r = env.decide(&gate, &intent(10), None, &Sink::new(false));
    assert!(matches!(r, Err(MintError::OverBudget)), "{r:?}");
}

#[test]
fn an_intent_lot_or_tick_that_differs_from_the_instrument_master_is_refused() {
    let mut env = Env::new(50);
    env.inst.lot_size = 5;
    let gate = Gate::new(&env.cfg, &Store, &At);
    let r = env.decide(&gate, &intent(10), None, &Sink::new(false));
    assert!(matches!(r, Err(MintError::InstrumentMismatch)), "{r:?}");
    env.inst.lot_size = 1;
    env.inst.tick_size = d("0.01");
    let r = env.decide(&gate, &intent(10), None, &Sink::new(false));
    assert!(matches!(r, Err(MintError::InstrumentMismatch)), "{r:?}");
}

#[test]
fn an_entry_without_a_stop_mints_nothing() {
    let env = Env::new(50);
    let gate = Gate::new(&env.cfg, &Store, &At);
    let mut i = intent(10);
    i.stop = None;
    let r = env.decide(&gate, &i, None, &Sink::new(false));
    assert!(!matches!(r, Ok(Decision::Minted(_))), "{r:?}");
}

#[test]
fn exits_flips_and_stop_moves_mint_without_new_sizing_risk() {
    let mut env = Env::new(50);
    env.held = vec![held_long(10)];
    let gate = Gate::new(&env.cfg, &Store, &At);
    let sink = Sink::new(false);
    // Exit: whole position, no stop needed, risk 0.
    env.data.margin.qty = 0;
    let mut exit = sell(10);
    exit.stop = None;
    let g = minted(env.decide(&gate, &exit, None, &sink));
    assert_eq!((g.qty(), g.risk_rupees()), (10, Decimal::ZERO));
    // Flip: sell 15 against 10 held; only the 5 excess carries risk: 5 x 10 + Rs 20.
    env.data.margin.qty = 5;
    let flip = sell(15);
    let g = minted(env.decide(&gate, &flip, None, &sink));
    assert_eq!((g.qty(), g.risk_rupees()), (15, d("70")));
    // Stop move toward profit: no quantity, no risk.
    env.data.margin.qty = 0;
    let mut mv = sell(0);
    mv.stop = None;
    mv.new_stop = Some(d("45"));
    let g = minted(env.decide(&gate, &mv, None, &sink));
    assert_eq!((g.qty(), g.risk_rupees()), (0, Decimal::ZERO));
}

// Unreachable by construction, so not tested: `ReceiptMismatch` (the receipt is built from the
// accepted override, whose digest and hash the override engine already bound to this verdict) and
// `Arithmetic` (checked maths on bounded `Decimal` inputs).

#[test]
fn only_gate_rs_builds_a_gated_intent() {
    fn scan(dir: &std::path::Path, hits: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                scan(&p, hits);
            } else if p.extension().is_some_and(|x| x == "rs")
                && p.file_name().is_some_and(|n| n != "gate.rs")
            {
                // Comments dropped, whitespace removed: `GatedIntent  {` cannot slip past.
                let code: String = std::fs::read_to_string(&p)
                    .unwrap()
                    .lines()
                    .filter(|l| !l.trim_start().starts_with("//"))
                    .collect::<String>()
                    .split_whitespace()
                    .collect();
                if code.contains("GatedIntent{") {
                    hits.push(p.display().to_string());
                }
            }
        }
    }
    let mut hits = vec![];
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    scan(&src, &mut hits);
    assert!(hits.is_empty(), "{hits:?}");
}
