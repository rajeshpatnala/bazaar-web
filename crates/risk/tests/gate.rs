//! Gate core tests: order, fail-closed paths, reference price, mode inputs, override through the gate.
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
use bazaar_risk::gate::{
    Evaluation, Gate, GateInputs, GateReject, ModeEvidence, live_trading_compiled,
};
use bazaar_risk::override_::{
    IntentDigest, OverrideChallenge, OverrideError, OverrideRequest, ReasonType, evaluate,
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
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};

const TEXT: &str = include_str!("../../../fixtures/config/risk.toml");

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn now() -> DateTime<Tz> {
    Kolkata.with_ymd_and_hms(2026, 11, 19, 11, 0, 0).unwrap()
}

/// Counts reads so a test can prove the gate reads the clock once.
struct CountingClock(AtomicU32);
impl Clock for CountingClock {
    fn now_ist(&self) -> DateTime<Tz> {
        let n = self.0.fetch_add(1, Ordering::SeqCst);
        now() + Duration::seconds(i64::from(n))
    }
}

struct At(DateTime<Tz>);
impl Clock for At {
    fn now_ist(&self) -> DateTime<Tz> {
        self.0
    }
}

struct Store(Result<Baseline, BaselineError>);
impl LimitsBaseline for Store {
    fn read(&self) -> Result<Baseline, BaselineError> {
        self.0.clone()
    }
}

fn intent(side: Side, qty: u32) -> OrderIntent {
    OrderIntent {
        symbol: "ABC".into(),
        side,
        qty: Qty(qty),
        price: d("500"),
        stop: Some(d("490")),
        new_stop: None,
        lot_size: 1,
        tick_size: d("0.05"),
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

fn held_long(qty: u32) -> Position {
    Position {
        symbol: "ABC".into(),
        side: Side::Buy,
        qty,
        stop: Tri::Known(d("490")),
        risk_price: Tri::Known(d("500")),
        avg_cost: Tri::Known(d("500")),
    }
}

fn data(oos: u32) -> DataInputs {
    DataInputs {
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
    }
}

fn series(end: NaiveDate) -> Vec<(NaiveDate, Decimal)> {
    (0..100u32)
        .map(|i| {
            let day = end - Duration::days(i64::from(99 - i));
            (day, Decimal::new(i64::from(i % 7) - 3, 3))
        })
        .collect()
}

fn exposure(end: NaiveDate) -> ExposureInputs {
    ExposureInputs {
        sectors: BTreeMap::from([("ABC".to_string(), "IT".to_string())]),
        returns: BTreeMap::from([("ABC".to_string(), series(end))]),
    }
}

fn mode_evidence(approval: Option<IntentDigest>) -> ModeEvidence {
    ModeEvidence {
        approval,
        compliance_fresh: Tri::Known(false),
        readiness_report: Tri::Known(false),
        owner_opt_in: Tri::Known(false),
        live: Tri::Known(false),
    }
}

struct Rig {
    cfg: GateConfig,
    data: DataInputs,
    inst: Instrument,
    mode: ModeEvidence,
    exposure: ExposureInputs,
    live: Tri<Decimal>,
}

impl Rig {
    fn new(text: &str) -> Self {
        Self {
            cfg: GateConfig::from_toml_str(text).unwrap(),
            data: data(50),
            inst: instrument(),
            mode: mode_evidence(None),
            exposure: exposure(now().date_naive()),
            live: Tri::Known(d("500")),
        }
    }

    fn run(
        &self,
        store: &Store,
        clock: &dyn Clock,
        i: &OrderIntent,
        s: &PortfolioSnapshot,
    ) -> Result<Evaluation, GateReject> {
        let inputs = GateInputs {
            data: &self.data,
            instrument: Some(&self.inst),
            defined_max_loss: None,
            mode: &self.mode,
            entry_zone: Tri::Known(EntryZone {
                low: d("499"),
                high: d("501"),
            }),
            live_price: self.live,
            exposure: Some(&self.exposure),
        };
        Gate::new(&self.cfg, store, clock).evaluate(i, s, &inputs)
    }

    fn eval(&self, i: &OrderIntent, s: &PortfolioSnapshot) -> Evaluation {
        self.run(&Store(Ok(Baseline::none())), &At(now()), i, s)
            .unwrap()
    }
}

fn failed(e: &Evaluation) -> Vec<RuleId> {
    e.verdict()
        .failures()
        .map(bazaar_risk::verdict::RuleResult::rule)
        .collect()
}

#[test]
fn clean_entry_passes_every_rule_in_order_and_is_deterministic() {
    let rig = Rig::new(TEXT);
    let (i, s) = (intent(Side::Buy, 10), snap());
    let a = rig.eval(&i, &s);
    assert_eq!(failed(&a), vec![], "{:?}", a.verdict().results());
    let order: Vec<RuleId> = a
        .verdict()
        .results()
        .iter()
        .map(bazaar_risk::verdict::RuleResult::rule)
        .collect();
    assert_eq!(order, RuleId::EVALUATION_ORDER.to_vec());
    let b = rig.eval(&i, &s);
    assert_eq!(a.verdict(), b.verdict());
    assert_eq!(a.limits_digest(), b.limits_digest());
    assert_eq!(a.verdict().intent_digest(), Some(&IntentDigest::of(&i)));
}

#[test]
fn unknown_snapshot_rejects_the_entry_and_lists_every_failure() {
    let rig = Rig::new(TEXT);
    let e = rig.eval(&intent(Side::Buy, 10), &PortfolioSnapshot::unknown());
    let f = failed(&e);
    for r in [
        RuleId::G16,
        RuleId::G15,
        RuleId::G03,
        RuleId::G08,
        RuleId::G19,
        RuleId::G18,
        RuleId::G17,
        RuleId::G02,
        RuleId::G14,
        RuleId::G04a,
        RuleId::G04b,
    ] {
        assert!(f.contains(&r), "{r} should fail on an unknown snapshot");
    }
    assert!(!e.verdict().passed());
    assert_eq!(e.verdict().results().len(), RuleId::EVALUATION_ORDER.len());
}

#[test]
fn non_canonical_symbols_are_rejected() {
    let rig = Rig::new(TEXT);
    for bad in [
        "abc",
        "AB C",
        "",
        "ABC\n",
        "ABC\u{200b}",
        "ÀBC",
        &"A".repeat(33),
    ] {
        let mut i = intent(Side::Buy, 10);
        i.symbol = bad.into();
        let r = rig.run(&Store(Ok(Baseline::none())), &At(now()), &i, &snap());
        assert!(matches!(r, Err(GateReject::Symbol(_))), "{bad:?}");
    }
    // Differs from the instrument master's trading symbol.
    let mut other = Rig::new(TEXT);
    other.inst.tradingsymbol = "XYZ".into();
    let r = other.run(
        &Store(Ok(Baseline::none())),
        &At(now()),
        &intent(Side::Buy, 10),
        &snap(),
    );
    assert!(matches!(r, Err(GateReject::Symbol(_))));
    // A non-canonical held symbol in the snapshot.
    let mut s = snap();
    let mut p = held_long(5);
    p.symbol = "abc".into();
    s.positions = Tri::Known(vec![p]);
    let r = rig.run(
        &Store(Ok(Baseline::none())),
        &At(now()),
        &intent(Side::Buy, 10),
        &s,
    );
    assert!(matches!(r, Err(GateReject::Symbol(_))));
    for ok in ["M&M", "BAJAJ-AUTO", "NIFTY26NOVFUT", "A_1"] {
        let mut i = intent(Side::Buy, 10);
        i.symbol = ok.into();
        let mut rig = Rig::new(TEXT);
        rig.inst.tradingsymbol = ok.into();
        assert!(
            rig.run(&Store(Ok(Baseline::none())), &At(now()), &i, &snap())
                .is_ok(),
            "{ok}"
        );
    }
}

#[test]
fn baseline_read_error_is_a_reject_with_no_verdict() {
    let rig = Rig::new(TEXT);
    let store = Store(Err(BaselineError("disk".into())));
    let r = rig.run(&store, &At(now()), &intent(Side::Buy, 10), &snap());
    assert!(matches!(r, Err(GateReject::Baseline(_))));
}

#[test]
fn unclassifiable_order_is_a_reject() {
    let rig = Rig::new(TEXT);
    let r = rig.run(
        &Store(Ok(Baseline::none())),
        &At(now()),
        &intent(Side::Buy, 0),
        &snap(),
    );
    assert!(matches!(r, Err(GateReject::Classify(_))));
}

#[test]
fn limits_digest_follows_the_baseline() {
    let rig = Rig::new(TEXT);
    let (i, s) = (intent(Side::Buy, 10), snap());
    let loose = rig.eval(&i, &s);
    let tight = Store(Ok(Baseline {
        max_risk_per_trade_pct: Some(d("0.1")),
        ..Baseline::none()
    }));
    let t = rig.run(&tight, &At(now()), &i, &s).unwrap();
    assert_ne!(loose.limits_digest(), t.limits_digest());
    assert_eq!(loose.verdict().config_hash(), t.verdict().config_hash());
}

#[test]
fn one_reference_price_is_the_worse_of_intent_and_live() {
    let mut rig = Rig::new(TEXT);
    let (i, s) = (intent(Side::Buy, 10), snap());
    rig.live = Tri::Known(d("500.5"));
    assert_eq!(rig.eval(&i, &s).ref_price(), d("500.5"));
    rig.live = Tri::Known(d("499.5"));
    assert_eq!(rig.eval(&i, &s).ref_price(), d("500"));
    rig.live = Tri::Unknown;
    let e = rig.eval(&i, &s);
    assert_eq!(e.ref_price(), d("500"));
    assert!(
        failed(&e).contains(&RuleId::G10),
        "unknown live price rejects G10"
    );
}

#[test]
fn clock_is_read_once_and_the_override_clock_is_the_same_instant() {
    let rig = Rig::new(TEXT);
    let clock = CountingClock(AtomicU32::new(0));
    let e = rig
        .run(
            &Store(Ok(Baseline::none())),
            &clock,
            &intent(Side::Buy, 10),
            &snap(),
        )
        .unwrap();
    assert_eq!(clock.0.load(Ordering::SeqCst), 1);
    assert_eq!(e.ctx().now, e.clock().now_ist());
    assert_eq!(e.clock().now_ist(), e.clock().now_ist());
}

#[test]
fn entry_blocked_at_day_loss_but_exit_and_tighter_stop_pass() {
    let rig = Rig::new(TEXT);
    let mut s = snap();
    s.positions = Tri::Known(vec![held_long(10)]);
    s.marks = Tri::Known(vec![Mark {
        symbol: "ABC".into(),
        price: d("500"),
    }]);
    s.day_pnl = Tri::Known(d("-9000"));
    let entry = rig.eval(&intent(Side::Buy, 5), &s);
    assert!(failed(&entry).contains(&RuleId::G03));
    // Sell more than held is an entry for the excess: still blocked.
    let flip = rig.eval(&intent(Side::Sell, 15), &s);
    assert!(failed(&flip).contains(&RuleId::G03));
    let exit = rig.eval(&intent(Side::Sell, 10), &s);
    assert!(!failed(&exit).contains(&RuleId::G03));
    // G09 sees the raw intent on a stop move: widening fails, tightening passes.
    let mut mv = intent(Side::Sell, 0);
    mv.stop = None;
    mv.new_stop = Some(d("480"));
    assert!(failed(&rig.eval(&mv, &s)).contains(&RuleId::G09));
    mv.new_stop = Some(d("495"));
    assert!(!failed(&rig.eval(&mv, &s)).contains(&RuleId::G09));
}

#[test]
fn entry_without_a_stop_is_rejected() {
    let rig = Rig::new(TEXT);
    let mut i = intent(Side::Buy, 10);
    i.stop = None;
    let e = rig.eval(&i, &snap());
    assert!(failed(&e).contains(&RuleId::G02));
}

#[test]
fn option_kind_comes_from_the_instrument_only() {
    let mut rig = Rig::new(TEXT);
    rig.inst.instrument_type = InstrumentType::Call;
    rig.inst.expiry = NaiveDate::from_ymd_opt(2026, 11, 26);
    rig.inst.strike = Some(d("500"));
    let e = rig.eval(&intent(Side::Sell, 10), &snap());
    assert!(
        failed(&e).contains(&RuleId::G13),
        "a sold option is a naked short"
    );
    rig.inst.instrument_type = InstrumentType::Other("XX".into());
    assert!(failed(&rig.eval(&intent(Side::Buy, 10), &snap())).contains(&RuleId::G13));
}

#[test]
fn stale_or_future_g05_series_rejects_entries() {
    let mut rig = Rig::new(TEXT);
    let s = snap();
    for end in [
        now().date_naive() - Duration::days(6),
        now().date_naive() + Duration::days(1),
    ] {
        rig.exposure = exposure(end);
        let e = rig.eval(&intent(Side::Buy, 10), &s);
        let g05 = e.verdict().failures().find(|r| r.rule() == RuleId::G05);
        assert!(
            g05.is_some_and(bazaar_risk::verdict::RuleResult::is_input_missing),
            "{end}"
        );
    }
    rig.exposure = exposure(now().date_naive() - Duration::days(5));
    assert!(!failed(&rig.eval(&intent(Side::Buy, 10), &s)).contains(&RuleId::G05));
}

fn semi_auto_text() -> String {
    TEXT.replace(
        "execution_mode = \"paper\"",
        "execution_mode = \"semi_auto\"",
    )
}

#[test]
fn approval_is_bound_to_the_intent_digest() {
    let i = intent(Side::Buy, 10);
    let mut rig = Rig::new(&semi_auto_text());
    let s = snap();
    assert!(
        failed(&rig.eval(&i, &s)).contains(&RuleId::G16),
        "no approval"
    );
    rig.mode = mode_evidence(Some(IntentDigest::of(&intent(Side::Buy, 11))));
    assert!(
        failed(&rig.eval(&i, &s)).contains(&RuleId::G16),
        "other order"
    );
    rig.mode = mode_evidence(Some(IntentDigest::of(&i)));
    assert!(!failed(&rig.eval(&i, &s)).contains(&RuleId::G16));
}

#[test]
fn live_needs_the_build_feature_which_the_caller_cannot_claim() {
    let i = intent(Side::Buy, 10);
    let mut rig = Rig::new(&semi_auto_text());
    rig.mode = mode_evidence(Some(IntentDigest::of(&i)));
    rig.mode.live = Tri::Known(true);
    let e = rig.eval(&i, &snap());
    assert_eq!(failed(&e).contains(&RuleId::G16), !live_trading_compiled());
    // Paper never goes live.
    let mut paper = Rig::new(TEXT);
    paper.mode.live = Tri::Known(true);
    assert!(failed(&paper.eval(&i, &snap())).contains(&RuleId::G16));
}

fn override_for(
    e: &Evaluation,
    i: &OrderIntent,
    s: &PortfolioSnapshot,
) -> Result<(), OverrideError> {
    let wait = i64::from(e.ctx().limits.override_confirm_delay_secs);
    let ch = OverrideChallenge::open(&At(now() - Duration::seconds(wait)), i, e.verdict()).unwrap();
    let req = OverrideRequest {
        challenge: ch,
        reason: ReasonType::NewInformation,
        note: "Fresh results changed my view of the evidence here.".into(),
    };
    let a = evaluate(req, i, e.verdict(), s, e.ctx(), e.clock())?;
    assert_eq!(a.accepted_at(), e.ctx().now);
    Ok(())
}

#[test]
fn g01_override_is_accepted_in_paper_only() {
    let i = intent(Side::Buy, 10);
    let s = snap();
    let mut paper = Rig::new(TEXT);
    paper.data = data(5);
    let e = paper.eval(&i, &s);
    assert_eq!(failed(&e), vec![RuleId::G01]);
    assert_eq!(override_for(&e, &i, &s), Ok(()));

    let mut semi = Rig::new(&semi_auto_text());
    semi.data = data(5);
    semi.mode = mode_evidence(Some(IntentDigest::of(&i)));
    let e = semi.eval(&i, &s);
    assert_eq!(failed(&e), vec![RuleId::G01]);
    assert_eq!(
        override_for(&e, &i, &s),
        Err(OverrideError::PaperOnly(RuleId::G01))
    );
}
