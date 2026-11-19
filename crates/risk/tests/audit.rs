//! Audit record and acknowledged-write tests with fake sinks.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use bazaar_core::clock::Clock;
use bazaar_core::mode::ExecutionMode;
use bazaar_core::money::Qty;
use bazaar_risk::audit::{AuditError, AuditRecord, AuditSink, write_acknowledged};
use bazaar_risk::config::{Baseline, GateConfig, RuleCtx};
use bazaar_risk::override_::{
    AcceptedOverride, IntentDigest, OverrideChallenge, OverrideRequest, ReasonType, evaluate,
};
use bazaar_risk::snapshot::PortfolioSnapshot;
use bazaar_risk::types::{OrderIntent, Side, Tri};
use bazaar_risk::verdict::{GateVerdict, RuleId, RuleResult};
use chrono::{DateTime, Duration, TimeZone};
use chrono_tz::{Asia::Kolkata, Tz};
use rust_decimal::Decimal;
use std::cell::RefCell;

const TEXT: &str = include_str!("../../../fixtures/config/risk.toml");
const NOTE: &str = "Earnings read changed my plan after the close.";

struct At(DateTime<Tz>);
impl Clock for At {
    fn now_ist(&self) -> DateTime<Tz> {
        self.0
    }
}

fn t0() -> DateTime<Tz> {
    Kolkata.with_ymd_and_hms(2026, 11, 19, 10, 0, 0).unwrap()
}

#[derive(Default)]
struct MemSink(RefCell<Vec<AuditRecord>>);
impl AuditSink for MemSink {
    fn write(&self, record: &AuditRecord) -> Result<(), AuditError> {
        self.0.borrow_mut().push(record.clone());
        Ok(())
    }
}

struct FailingSink;
impl AuditSink for FailingSink {
    fn write(&self, _: &AuditRecord) -> Result<(), AuditError> {
        Err(AuditError("disk full".into()))
    }
}

fn intent() -> OrderIntent {
    OrderIntent {
        symbol: "ABC".into(),
        side: Side::Buy,
        qty: Qty(10),
        price: Decimal::from(100),
        stop: Some(Decimal::from(95)),
        new_stop: None,
        lot_size: 1,
        tick_size: Decimal::new(5, 2),
    }
}

fn accepted() -> (AcceptedOverride, RuleCtx) {
    let cfg = GateConfig::from_toml_str(TEXT).unwrap();
    let ctx = RuleCtx {
        limits: cfg.effective(&Baseline::none()),
        mode: ExecutionMode::Paper,
        config_hash: cfg.hash().clone(),
        now: t0(),
    };
    let results: Vec<RuleResult> = RuleId::EVALUATION_ORDER
        .iter()
        .map(|&r| {
            if r == RuleId::G14 {
                RuleResult::fail(r, "o", "l", "Too much.")
            } else {
                RuleResult::pass(r, "o", "l")
            }
        })
        .collect();
    let v = GateVerdict::new(ctx.config_hash.clone(), &results)
        .unwrap()
        .bound_to(IntentDigest::of(&intent()));
    let wait = Duration::seconds(i64::from(ctx.limits.override_confirm_delay_secs));
    let ch = OverrideChallenge::open(&At(t0() - wait), &intent(), &v).unwrap();
    let req = OverrideRequest {
        challenge: ch,
        reason: ReasonType::PlanChanged,
        note: NOTE.into(),
    };
    let mut s = PortfolioSnapshot::unknown();
    s.overrides_today = Tri::Known(0);
    s.overrides_week = Tri::Known(0);
    let a = evaluate(req, &intent(), &v, &s, &ctx, &At(t0())).unwrap();
    (a, ctx)
}

#[test]
fn record_carries_every_field_with_time_from_the_clock() {
    let (a, ctx) = accepted();
    let later = t0() + Duration::seconds(3);
    let r = AuditRecord::new(&a, &At(later));
    assert_eq!(r.rule_ids(), &[RuleId::G14]);
    assert_eq!(r.reason(), ReasonType::PlanChanged);
    assert_eq!(r.note(), NOTE);
    assert_eq!(r.config_hash(), &ctx.config_hash);
    assert_eq!(r.intent_digest(), &IntentDigest::of(&intent()));
    assert_eq!(r.at(), later);
}

#[test]
fn acknowledged_write_stores_the_record_and_returns_a_receipt() {
    let (a, ctx) = accepted();
    let sink = MemSink::default();
    let rc = write_acknowledged(&sink, &a, &At(t0())).unwrap();
    assert_eq!(sink.0.borrow().len(), 1);
    assert_eq!(sink.0.borrow()[0], AuditRecord::new(&a, &At(t0())));
    assert_eq!(rc.intent_digest(), &IntentDigest::of(&intent()));
    assert_eq!(rc.config_hash(), &ctx.config_hash);
    assert_eq!(rc.at(), t0());
}

#[test]
fn sink_error_is_returned_and_no_receipt_exists() {
    let (a, _) = accepted();
    let e = write_acknowledged(&FailingSink, &a, &At(t0())).unwrap_err();
    assert_eq!(e, AuditError("disk full".into()));
    assert!(e.to_string().contains("disk full"));
    assert!(e.to_string().contains("refused"));
}
