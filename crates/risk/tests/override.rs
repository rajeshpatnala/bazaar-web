//! Override engine tests: acceptance, refusals, binding, caps, delay, single use.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use bazaar_core::clock::Clock;
use bazaar_core::mode::ExecutionMode;
use bazaar_core::money::Qty;
use bazaar_risk::config::{Baseline, ConfigHash, GateConfig, RuleCtx};
use bazaar_risk::override_::{
    AcceptedOverride, IntentDigest, OverrideError, OverrideRequest, ReasonType, evaluate,
};
use bazaar_risk::snapshot::PortfolioSnapshot;
use bazaar_risk::types::{OrderIntent, Side, Tri};
use bazaar_risk::verdict::{GateVerdict, RuleId, RuleResult};
use chrono::{DateTime, Duration, TimeZone};
use chrono_tz::{Asia::Kolkata, Tz};
use rust_decimal::Decimal;

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

fn ctx(mode: ExecutionMode) -> RuleCtx {
    let cfg = GateConfig::from_toml_str(TEXT).unwrap();
    RuleCtx {
        limits: cfg.effective(&Baseline::none()),
        mode,
        config_hash: cfg.hash().clone(),
        now: t0(),
    }
}

fn verdict(c: &RuleCtx, failing: &[RuleId]) -> GateVerdict {
    let results: Vec<RuleResult> = RuleId::EVALUATION_ORDER
        .iter()
        .map(|&r| {
            if failing.contains(&r) {
                RuleResult::fail(r, "o", "l", "Too much.")
            } else {
                RuleResult::pass(r, "o", "l")
            }
        })
        .collect();
    GateVerdict::new(c.config_hash.clone(), &results).unwrap()
}

fn snap(day: u32, week: u32) -> PortfolioSnapshot {
    let mut s = PortfolioSnapshot::unknown();
    s.overrides_today = Tri::Known(day);
    s.overrides_week = Tri::Known(week);
    s
}

fn req(c: &RuleCtx, ids: &[RuleId]) -> OverrideRequest {
    OverrideRequest {
        rule_ids: ids.to_vec(),
        reason: ReasonType::NewInformation,
        note: NOTE.into(),
        intent_digest: IntentDigest::of(&intent()),
        config_hash: c.config_hash.clone(),
        opened_at: t0() - Duration::seconds(i64::from(c.limits.override_confirm_delay_secs)),
    }
}

fn run(
    mode: ExecutionMode,
    failing: &[RuleId],
    tweak: impl FnOnce(&mut OverrideRequest),
    s: &PortfolioSnapshot,
) -> Result<AcceptedOverride, OverrideError> {
    let c = ctx(mode);
    let v = verdict(&c, failing);
    let mut r = req(&c, failing);
    tweak(&mut r);
    evaluate(r, &intent(), &v, s, &c, &At(t0()))
}

#[test]
fn soft_row_accepts_in_paper_and_semi_auto_with_binding() {
    for mode in [ExecutionMode::Paper, ExecutionMode::SemiAuto] {
        let a = run(mode, &[RuleId::G14], |_| {}, &snap(0, 0)).unwrap();
        assert_eq!(a.rule_ids(), &[RuleId::G14]);
        assert_eq!(a.reason(), ReasonType::NewInformation);
        assert_eq!(a.note(), NOTE);
        assert_eq!(a.intent_digest(), &IntentDigest::of(&intent()));
        assert_eq!(a.config_hash(), &ctx(mode).config_hash);
        assert_eq!(a.accepted_at(), t0());
        assert_eq!(a.size_factor(), ctx(mode).limits.override_size_factor);
    }
}

#[test]
fn several_soft_failures_need_all_named() {
    let both = [RuleId::G04a, RuleId::G14];
    assert!(run(ExecutionMode::Paper, &both, |_| {}, &snap(0, 0)).is_ok());
    let r = run(
        ExecutionMode::Paper,
        &both,
        |r| r.rule_ids = vec![RuleId::G14],
        &snap(0, 0),
    );
    assert_eq!(r.unwrap_err(), OverrideError::RuleMismatch);
}

#[test]
fn paper_row_only_in_paper_mode() {
    assert!(run(ExecutionMode::Paper, &[RuleId::G01], |_| {}, &snap(0, 0)).is_ok());
    assert_eq!(
        run(ExecutionMode::SemiAuto, &[RuleId::G01], |_| {}, &snap(0, 0)).unwrap_err(),
        OverrideError::PaperOnly(RuleId::G01)
    );
    assert_eq!(
        run(ExecutionMode::Auto, &[RuleId::G01], |_| {}, &snap(0, 0)).unwrap_err(),
        OverrideError::AutoMode
    );
}

#[test]
fn any_hard_or_no_failure_rejects_the_whole_override() {
    for hard in [RuleId::G03, RuleId::G09, RuleId::G08, RuleId::X02] {
        let e = run(
            ExecutionMode::Paper,
            &[RuleId::G14, hard],
            |r| r.rule_ids = vec![RuleId::G14],
            &snap(0, 0),
        )
        .unwrap_err();
        assert_eq!(e, OverrideError::NotOverridable(hard));
    }
}

#[test]
fn auto_mode_never_accepts() {
    let e = run(ExecutionMode::Auto, &[RuleId::G14], |_| {}, &snap(0, 0)).unwrap_err();
    assert_eq!(e, OverrideError::AutoMode);
}

#[test]
fn passing_verdict_has_nothing_to_override() {
    let e = run(ExecutionMode::Paper, &[], |_| {}, &snap(0, 0)).unwrap_err();
    assert_eq!(e, OverrideError::NothingToOverride);
}

#[test]
fn extra_or_duplicate_rule_ids_are_refused() {
    let e = run(
        ExecutionMode::Paper,
        &[RuleId::G14],
        |r| r.rule_ids.push(RuleId::G05),
        &snap(0, 0),
    );
    assert_eq!(e.unwrap_err(), OverrideError::RuleMismatch);
    let e = run(
        ExecutionMode::Paper,
        &[RuleId::G14],
        |r| r.rule_ids.push(RuleId::G14),
        &snap(0, 0),
    );
    assert_eq!(e.unwrap_err(), OverrideError::RuleMismatch);
}

#[test]
fn bound_to_intent_digest_and_config_hash() {
    let mut other = intent();
    other.qty = Qty(11);
    let e = run(
        ExecutionMode::Paper,
        &[RuleId::G14],
        |r| r.intent_digest = IntentDigest::of(&other),
        &snap(0, 0),
    );
    assert_eq!(e.unwrap_err(), OverrideError::IntentMismatch);
    let e = run(
        ExecutionMode::Paper,
        &[RuleId::G14],
        |r| r.config_hash = ConfigHash::of_text("other"),
        &snap(0, 0),
    );
    assert_eq!(e.unwrap_err(), OverrideError::ConfigMismatch);
}

#[test]
fn digest_changes_with_every_field() {
    let base = IntentDigest::of(&intent());
    assert_eq!(base, IntentDigest::of(&intent()));
    assert_eq!(base.as_str().len(), 64);
    let edits: [fn(&mut OrderIntent); 7] = [
        |i| i.symbol = "XYZ".into(),
        |i| i.side = Side::Sell,
        |i| i.price = Decimal::from(101),
        |i| i.stop = None,
        |i| i.new_stop = Some(Decimal::from(96)),
        |i| i.lot_size = 5,
        |i| i.tick_size = Decimal::new(1, 1),
    ];
    for edit in edits {
        let mut i = intent();
        edit(&mut i);
        assert_ne!(base, IntentDigest::of(&i));
    }
}

#[test]
fn note_needs_30_trimmed_characters_for_every_reason() {
    for reason in [ReasonType::Other, ReasonType::PlanChanged] {
        let e = run(
            ExecutionMode::Paper,
            &[RuleId::G14],
            |r| {
                r.reason = reason;
                r.note = format!("  {}  ", "x".repeat(29));
            },
            &snap(0, 0),
        );
        assert_eq!(
            e.unwrap_err(),
            OverrideError::NoteTooShort { have: 29, need: 30 }
        );
    }
    let ok = run(
        ExecutionMode::Paper,
        &[RuleId::G14],
        |r| r.note = format!(" {} ", "x".repeat(30)),
        &snap(0, 0),
    );
    assert_eq!(ok.unwrap().note().len(), 30);
}

#[test]
fn caps_come_from_the_snapshot_and_effective_limits() {
    let c = ctx(ExecutionMode::Paper);
    let (md, mw) = (
        c.limits.override_max_per_day,
        c.limits.override_max_per_week,
    );
    let day = run(ExecutionMode::Paper, &[RuleId::G14], |_| {}, &snap(md, 0)).unwrap_err();
    assert_eq!(day, OverrideError::DayCapReached { used: md, max: md });
    let week = run(ExecutionMode::Paper, &[RuleId::G14], |_| {}, &snap(0, mw)).unwrap_err();
    assert_eq!(week, OverrideError::WeekCapReached { used: mw, max: mw });
    let under = snap(md - 1, mw - 1);
    assert!(run(ExecutionMode::Paper, &[RuleId::G14], |_| {}, &under).is_ok());
}

#[test]
fn unknown_counts_refuse() {
    let mut s = snap(0, 0);
    s.overrides_week = Tri::Unknown;
    let e = run(ExecutionMode::Paper, &[RuleId::G14], |_| {}, &s).unwrap_err();
    assert_eq!(e, OverrideError::CountUnknown);
    let u = PortfolioSnapshot::unknown();
    let e = run(ExecutionMode::Paper, &[RuleId::G14], |_| {}, &u);
    assert_eq!(e.unwrap_err(), OverrideError::CountUnknown);
}

#[test]
fn confirm_delay_runs_on_the_clock() {
    let c = ctx(ExecutionMode::Paper);
    let delay = i64::from(c.limits.override_confirm_delay_secs);
    let v = verdict(&c, &[RuleId::G14]);
    let try_at = |waited: i64| {
        let mut r = req(&c, &[RuleId::G14]);
        r.opened_at = t0() - Duration::seconds(waited);
        evaluate(r, &intent(), &v, &snap(0, 0), &c, &At(t0()))
    };
    assert_eq!(
        try_at(delay - 1).unwrap_err(),
        OverrideError::TooSoon { wait_secs: 1 }
    );
    assert!(try_at(delay).is_ok());
    // Opened in the future: clock skew never shortens the wait (full delay applies).
    assert_eq!(
        try_at(-5).unwrap_err(),
        OverrideError::TooSoon {
            wait_secs: u32::try_from(delay).unwrap()
        }
    );
}

#[test]
fn accepted_override_is_moved_not_copied() {
    // `evaluate` takes the request by value and neither type is Clone, so a second use of
    // the same request or accepted override does not compile.
    let a = run(ExecutionMode::Paper, &[RuleId::G14], |_| {}, &snap(0, 0)).unwrap();
    let moved = a;
    assert_eq!(moved.rule_ids().len(), 1);
}

#[test]
fn errors_read_in_plain_english() {
    let e = OverrideError::NotOverridable(RuleId::G03);
    assert!(e.to_string().contains("G03"));
    assert!(
        OverrideError::TooSoon { wait_secs: 3 }
            .to_string()
            .contains("3 more seconds")
    );
}
