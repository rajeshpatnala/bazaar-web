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
    AcceptedOverride, IntentDigest, OverrideChallenge, OverrideError, OverrideRequest, ReasonType,
    evaluate,
};
use bazaar_risk::snapshot::PortfolioSnapshot;
use bazaar_risk::types::{OrderIntent, Side, Tri};
use bazaar_risk::verdict::{GateVerdict, OverrideLevel, RuleId, RuleResult};
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

fn verdict_for(
    hash: &ConfigHash,
    i: &OrderIntent,
    failing: &[RuleId],
    missing: &[RuleId],
) -> GateVerdict {
    let results: Vec<RuleResult> = RuleId::EVALUATION_ORDER
        .iter()
        .map(|&r| {
            if missing.contains(&r) {
                RuleResult::input_missing(r, "o", "l", "Input missing.")
            } else if failing.contains(&r) {
                RuleResult::fail(r, "o", "l", "Too much.")
            } else {
                RuleResult::pass(r, "o", "l")
            }
        })
        .collect();
    GateVerdict::new(hash.clone(), &results)
        .unwrap()
        .bound_to(IntentDigest::of(i))
}

fn verdict(c: &RuleCtx, failing: &[RuleId]) -> GateVerdict {
    verdict_for(&c.config_hash, &intent(), failing, &[])
}

fn snap(day: u32, week: u32) -> PortfolioSnapshot {
    let mut s = PortfolioSnapshot::unknown();
    s.overrides_today = Tri::Known(day);
    s.overrides_week = Tri::Known(week);
    s
}

fn delay(c: &RuleCtx) -> Duration {
    Duration::seconds(i64::from(c.limits.override_confirm_delay_secs))
}

/// Open a challenge `waited` before `t0()`.
fn challenge(v: &GateVerdict, waited: Duration) -> OverrideChallenge {
    OverrideChallenge::open(&At(t0() - waited), &intent(), v).unwrap()
}

fn request(ch: OverrideChallenge) -> OverrideRequest {
    OverrideRequest {
        challenge: ch,
        reason: ReasonType::NewInformation,
        note: NOTE.into(),
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
    let mut r = request(challenge(&v, delay(&c)));
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
fn level_table_drives_acceptance_for_every_rule() {
    for rule in RuleId::EVALUATION_ORDER {
        for failing in [vec![rule], vec![rule, RuleId::G14]] {
            if rule == RuleId::G14 && failing.len() == 2 {
                continue;
            }
            let ok = |mode| run(mode, &failing, |_| {}, &snap(0, 0));
            match rule.override_level() {
                OverrideLevel::HardCeiling | OverrideLevel::No => {
                    for mode in [ExecutionMode::Paper, ExecutionMode::SemiAuto] {
                        let e = ok(mode).unwrap_err();
                        assert_eq!(e, OverrideError::NotOverridable(rule), "{rule}");
                    }
                }
                OverrideLevel::Soft => assert!(ok(ExecutionMode::SemiAuto).is_ok(), "{rule}"),
                OverrideLevel::Paper => {
                    assert!(ok(ExecutionMode::Paper).is_ok(), "{rule}");
                    assert_eq!(
                        ok(ExecutionMode::SemiAuto).unwrap_err(),
                        OverrideError::PaperOnly(rule)
                    );
                }
            }
        }
    }
}

#[test]
fn missing_input_is_never_overridable_even_on_soft_rows() {
    for rule in [RuleId::G05, RuleId::G07, RuleId::G04a, RuleId::G14] {
        let c = ctx(ExecutionMode::Paper);
        let v = verdict_for(&c.config_hash, &intent(), &[], &[rule]);
        let r = request(challenge(&v, delay(&c)));
        let e = evaluate(r, &intent(), &v, &snap(0, 0), &c, &At(t0())).unwrap_err();
        assert_eq!(e, OverrideError::InputMissing(rule));
    }
    // A real breach on the same rule is accepted.
    assert!(run(ExecutionMode::Paper, &[RuleId::G05], |_| {}, &snap(0, 0)).is_ok());
}

#[test]
fn paper_experiment_reason_only_in_paper_mode() {
    let set = |r: &mut OverrideRequest| r.reason = ReasonType::PaperExperiment;
    assert!(run(ExecutionMode::Paper, &[RuleId::G14], set, &snap(0, 0)).is_ok());
    let e = run(ExecutionMode::SemiAuto, &[RuleId::G14], set, &snap(0, 0)).unwrap_err();
    assert_eq!(e, OverrideError::ReasonNotAllowed);
    let e = run(ExecutionMode::Auto, &[RuleId::G14], set, &snap(0, 0)).unwrap_err();
    assert_eq!(e, OverrideError::AutoMode);
}

#[test]
fn several_soft_failures_must_match_the_challenge() {
    let both = [RuleId::G04a, RuleId::G14];
    let a = run(ExecutionMode::Paper, &both, |_| {}, &snap(0, 0)).unwrap();
    assert_eq!(a.rule_ids(), &[RuleId::G04a, RuleId::G14]);
    // Challenge opened for one rule, evaluated against a verdict with two.
    let c = ctx(ExecutionMode::Paper);
    let one = challenge(&verdict(&c, &[RuleId::G14]), delay(&c));
    let v2 = verdict(&c, &both);
    let e = evaluate(request(one), &intent(), &v2, &snap(0, 0), &c, &At(t0()));
    assert_eq!(e.unwrap_err(), OverrideError::RuleMismatch);
}

#[test]
fn auto_mode_never_accepts() {
    let e = run(ExecutionMode::Auto, &[RuleId::G14], |_| {}, &snap(0, 0)).unwrap_err();
    assert_eq!(e, OverrideError::AutoMode);
    let e = run(ExecutionMode::Auto, &[RuleId::G01], |_| {}, &snap(0, 0)).unwrap_err();
    assert_eq!(e, OverrideError::AutoMode);
}

#[test]
fn passing_verdict_has_nothing_to_override() {
    let e = run(ExecutionMode::Paper, &[], |_| {}, &snap(0, 0)).unwrap_err();
    assert_eq!(e, OverrideError::NothingToOverride);
}

#[test]
fn verdict_must_be_bound_to_the_intent() {
    let c = ctx(ExecutionMode::Paper);
    let mut other = intent();
    other.qty = Qty(400);
    // A G14-only verdict computed for a small order, reused for a big one.
    let v_small = verdict(&c, &[RuleId::G14]);
    let ch = challenge(&v_small, delay(&c));
    let e = evaluate(request(ch), &other, &v_small, &snap(0, 0), &c, &At(t0()));
    assert_eq!(e.unwrap_err(), OverrideError::IntentMismatch);
    // Opening a challenge for an intent the verdict was not computed for is refused too.
    let e = OverrideChallenge::open(&At(t0()), &other, &v_small);
    assert_eq!(e.unwrap_err(), OverrideError::IntentMismatch);
    // An unbound verdict is refused.
    let results: Vec<RuleResult> = RuleId::EVALUATION_ORDER
        .iter()
        .map(|&r| RuleResult::pass(r, "o", "l"))
        .collect();
    let unbound = GateVerdict::new(c.config_hash.clone(), &results).unwrap();
    let e = OverrideChallenge::open(&At(t0()), &intent(), &unbound);
    assert_eq!(e.unwrap_err(), OverrideError::IntentMismatch);
}

#[test]
fn bound_to_config_hash_of_verdict_and_ctx() {
    let c = ctx(ExecutionMode::Paper);
    let other = ConfigHash::of_text("other");
    // Verdict, challenge and request all carry the other hash; ctx differs.
    let v = verdict_for(&other, &intent(), &[RuleId::G14], &[]);
    let r = request(challenge(&v, delay(&c)));
    let e = evaluate(r, &intent(), &v, &snap(0, 0), &c, &At(t0()));
    assert_eq!(e.unwrap_err(), OverrideError::ConfigMismatch);
    // Challenge from a verdict under the right hash, evaluated against another-hash verdict.
    let v_ok = verdict(&c, &[RuleId::G14]);
    let r = request(challenge(&v_ok, delay(&c)));
    let e = evaluate(r, &intent(), &v, &snap(0, 0), &c, &At(t0()));
    assert_eq!(e.unwrap_err(), OverrideError::ConfigMismatch);
}

#[test]
fn challenge_for_another_intent_is_refused_at_evaluate() {
    let c = ctx(ExecutionMode::Paper);
    let mut other = intent();
    other.qty = Qty(11);
    let v_other = verdict_for(&c.config_hash, &other, &[RuleId::G14], &[]);
    let ch = OverrideChallenge::open(&At(t0() - delay(&c)), &other, &v_other).unwrap();
    let v = verdict(&c, &[RuleId::G14]);
    let e = evaluate(request(ch), &intent(), &v, &snap(0, 0), &c, &At(t0()));
    assert_eq!(e.unwrap_err(), OverrideError::IntentMismatch);
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
fn note_rules_apply_for_every_reason() {
    let note = |n: String| move |r: &mut OverrideRequest| r.note = n;
    for reason in [ReasonType::Other, ReasonType::PlanChanged] {
        let set = |n: String| {
            move |r: &mut OverrideRequest| {
                r.reason = reason;
                r.note = n;
            }
        };
        let e = run(
            ExecutionMode::Paper,
            &[RuleId::G14],
            set(format!("  {}  ", "x".repeat(29))),
            &snap(0, 0),
        );
        assert_eq!(
            e.unwrap_err(),
            OverrideError::NoteTooShort { have: 29, need: 30 }
        );
    }
    let p = ExecutionMode::Paper;
    // Thirty repeated letters are one word.
    let e = run(p, &[RuleId::G14], note("x".repeat(30)), &snap(0, 0));
    assert_eq!(
        e.unwrap_err(),
        OverrideError::NoteTooFewWords { have: 1, need: 5 }
    );
    let four = "aaaaaaa bbbbbbb ccccccc ddddddd eeeee".replace(" eeeee", "");
    let e = run(p, &[RuleId::G14], note(four), &snap(0, 0));
    assert!(matches!(
        e.unwrap_err(),
        OverrideError::NoteTooFewWords { have: 4, .. }
    ));
    let long = format!("{} end", "word ".repeat(250));
    let e = run(p, &[RuleId::G14], note(long), &snap(0, 0));
    assert!(matches!(
        e.unwrap_err(),
        OverrideError::NoteTooLong { max: 1000, .. }
    ));
    let ctl = format!("{NOTE}\u{7}");
    let e = run(p, &[RuleId::G14], note(ctl), &snap(0, 0));
    assert_eq!(e.unwrap_err(), OverrideError::NoteControlChars);
    let ok = run(p, &[RuleId::G14], note(format!(" {NOTE} ")), &snap(0, 0));
    assert_eq!(ok.unwrap().note(), NOTE);
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
fn confirm_delay_runs_from_the_crate_stamped_clock_time() {
    let c = ctx(ExecutionMode::Paper);
    let d = delay(&c);
    let v = verdict(&c, &[RuleId::G14]);
    let try_after = |waited: Duration| {
        let r = request(challenge(&v, waited));
        evaluate(r, &intent(), &v, &snap(0, 0), &c, &At(t0()))
    };
    assert_eq!(
        try_after(d - Duration::seconds(1)).unwrap_err(),
        OverrideError::TooSoon { wait_secs: 1 }
    );
    assert!(try_after(d).is_ok());
    // A challenge stamped in the future (clock skew) never shortens the wait.
    assert_eq!(
        try_after(Duration::seconds(-5)).unwrap_err(),
        OverrideError::TooSoon {
            wait_secs: u32::try_from(d.num_seconds()).unwrap()
        }
    );
}

/// Compiles only when `T` is not `Clone` (two blanket impls make the call ambiguous otherwise).
trait AmbiguousIfClone<A> {
    fn check() {}
}
impl<T> AmbiguousIfClone<()> for T {}
impl<T: Clone> AmbiguousIfClone<u8> for T {}

#[test]
fn single_use_types_are_not_clone() {
    <OverrideRequest as AmbiguousIfClone<_>>::check();
    <OverrideChallenge as AmbiguousIfClone<_>>::check();
    <AcceptedOverride as AmbiguousIfClone<_>>::check();
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
    assert!(
        OverrideError::InputMissing(RuleId::G07)
            .to_string()
            .contains("Fix the input")
    );
}
