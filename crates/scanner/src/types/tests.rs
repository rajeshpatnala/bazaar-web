use bazaar_regime::{Fact, RegimeState};
use bazaar_risk::config::ConfigHash;
use bazaar_risk::verdict::{GateVerdict, RuleId, RuleResult};
use chrono::TimeZone;
use chrono_tz::Asia::Kolkata;
use rust_decimal::Decimal;

use super::*;

fn t(h: u32) -> DateTime<Tz> {
    Kolkata.with_ymd_and_hms(2026, 11, 20, h, 0, 0).unwrap()
}

fn d(n: i64) -> Decimal {
    Decimal::from(n)
}

fn verdict(failing: &[RuleId]) -> GateVerdict {
    let rs: Vec<RuleResult> = RuleId::EVALUATION_ORDER
        .iter()
        .map(|r| {
            if failing.contains(r) {
                RuleResult::fail(*r, "x", "y", "no")
            } else {
                RuleResult::pass(*r, "x", "y")
            }
        })
        .collect();
    GateVerdict::new(ConfigHash::of_text("cfg"), &rs).unwrap()
}

fn parts() -> RecommendationParts {
    RecommendationParts {
        id: 7,
        generated_at: t(18),
        data_as_of: t(15),
        instrument: InstrumentRef {
            instrument_token: 1,
            tradingsymbol: "ABC".into(),
            name: "Abc Ltd".into(),
        },
        action: Action::Buy,
        entry_zone: (d(100), d(102)),
        stop_loss: d(96),
        targets: vec![d(110)],
        horizon: Horizon::Swing,
        size: PositionSize { qty: 10, lots: 1 },
        risk_rupees: d(60),
        reward_risk: reward_risk(d(102), d(96), d(110)).unwrap(),
        agreeing_strategies: vec!["donchian_breakout".into()],
        confidence: 80,
        regime: RegimeState::unknown(t(15), Fact::new("a", "b", "c")),
        backtest: None,
        why: vec![WhyFactor::Risk {
            risk_rupees: d(60),
            limit_rupees: d(1000),
            qty: 10,
        }],
        warnings: vec![],
        gate: GateSummary::from_verdict(&verdict(&[]), t(18)),
    }
}

fn good_evidence(trades: u32) -> Evidence {
    Evidence {
        trades,
        costs_applied: true,
        out_of_sample: true,
        win_rate: 0.5,
        profit_factor: 1.4,
        avg_r: 0.2,
        max_drawdown_pct: 8.0,
        period: "2021 to 2025".into(),
    }
}

fn low_with(f: impl FnOnce(&mut Evidence), min: u32) -> Recommendation {
    let mut p = parts();
    let mut e = good_evidence(30);
    f(&mut e);
    p.backtest = Some(e);
    Recommendation::new(p, min, 55).unwrap()
}

#[test]
fn each_low_evidence_branch_caps_confidence() {
    assert!(low_with(|e| e.trades = 29, 30).low_evidence());
    assert!(!low_with(|_| {}, 30).low_evidence());
    let no_costs = low_with(|e| e.costs_applied = false, 30);
    assert!(no_costs.low_evidence() && no_costs.confidence() == 55);
    let in_sample = low_with(|e| e.out_of_sample = false, 30);
    assert!(in_sample.low_evidence() && in_sample.confidence() == 55);
    assert!(in_sample.warnings().contains(&Warning::LowEvidence));
}

#[test]
fn zero_min_trades_cannot_clear_no_evidence() {
    let r = Recommendation::new(parts(), 0, 55).unwrap();
    assert!(r.low_evidence());
    assert_eq!(r.confidence(), 55);
}

#[test]
fn risk_below_stop_distance_is_rejected() {
    let mut p = parts();
    p.risk_rupees = d(59);
    assert_eq!(
        Recommendation::new(p.clone(), 30, 55).unwrap_err(),
        RecError::RiskBelowStopDistance
    );
    p.action = Action::Sell;
    p.stop_loss = d(106);
    p.targets = vec![d(90)];
    p.reward_risk = reward_risk(d(100), d(106), d(90)).unwrap();
    assert_eq!(
        Recommendation::new(p, 30, 55).unwrap_err(),
        RecError::RiskBelowStopDistance
    );
}

#[test]
fn targets_must_move_strictly_away_from_entry() {
    let mut p = parts();
    p.targets = vec![d(110), d(110)];
    assert_eq!(
        Recommendation::new(p.clone(), 30, 55).unwrap_err(),
        RecError::TargetsNotMonotonic
    );
    p.targets = vec![d(110), d(105)];
    assert_eq!(
        Recommendation::new(p.clone(), 30, 55).unwrap_err(),
        RecError::TargetsNotMonotonic
    );
    p.targets = vec![d(105), d(110)];
    p.reward_risk = reward_risk(d(102), d(96), d(105)).unwrap();
    assert!(Recommendation::new(p, 30, 55).is_ok());
}

#[test]
fn no_evidence_is_low_evidence_and_caps_confidence() {
    let r = Recommendation::new(parts(), 30, 55).unwrap();
    assert!(r.low_evidence());
    assert_eq!(r.confidence(), 55);
    assert!(r.backtest().is_none());
    assert!(r.warnings().contains(&Warning::LowEvidence));
}

#[test]
fn enough_trades_keeps_confidence() {
    let mut p = parts();
    p.backtest = Some(good_evidence(30));
    let r = Recommendation::new(p, 30, 55).unwrap();
    assert!(!r.low_evidence());
    assert_eq!(r.confidence(), 80);
    assert_eq!(r.warnings(), &[] as &[Warning]);
}

#[test]
fn low_evidence_below_cap_is_not_raised() {
    let mut p = parts();
    p.confidence = 40;
    assert_eq!(Recommendation::new(p, 30, 55).unwrap().confidence(), 40);
}

#[test]
fn each_missing_or_bad_field_is_an_error() {
    let bad = |f: fn(&mut RecommendationParts)| {
        let mut p = parts();
        f(&mut p);
        Recommendation::new(p, 30, 55).unwrap_err()
    };
    assert_eq!(
        bad(|p| p.entry_zone = (d(103), d(102))),
        RecError::BadEntryZone
    );
    assert_eq!(bad(|p| p.stop_loss = d(100)), RecError::BadStop);
    assert_eq!(bad(|p| p.targets.clear()), RecError::NoTargets);
    assert_eq!(bad(|p| p.targets = vec![d(101)]), RecError::BadTarget);
    assert_eq!(bad(|p| p.risk_rupees = d(0)), RecError::NoRisk);
    assert_eq!(bad(|p| p.reward_risk = d(0)), RecError::BadRewardRisk);
    assert_eq!(bad(|p| p.size.qty = 0), RecError::NoQuantity);
    assert_eq!(
        bad(|p| p.agreeing_strategies.clear()),
        RecError::NoStrategies
    );
    assert_eq!(bad(|p| p.confidence = 101), RecError::BadConfidence);
    assert_eq!(bad(|p| p.why.clear()), RecError::NoWhy);
    assert_eq!(bad(|p| p.data_as_of = t(19)), RecError::DataFromFuture);
    assert_eq!(bad(|p| p.action = Action::NoTrade), RecError::NoAction);
}

#[test]
fn sell_checks_sides_the_other_way() {
    let mut p = parts();
    p.action = Action::Sell;
    assert_eq!(
        Recommendation::new(p.clone(), 30, 55).unwrap_err(),
        RecError::BadStop
    );
    p.stop_loss = d(106);
    p.targets = vec![d(90)];
    p.reward_risk = reward_risk(d(100), d(106), d(90)).unwrap();
    assert!(Recommendation::new(p, 30, 55).is_ok());
}

#[test]
fn gate_summary_splits_hard_and_overridable() {
    let s = GateSummary::from_verdict(&verdict(&[RuleId::G02, RuleId::G05]), t(18));
    assert!(!s.passed);
    assert_eq!(s.hard_failures, vec![RuleId::G02]);
    assert_eq!(s.overridable_failures, vec![RuleId::G05]);
    assert_eq!(s.config_hash, ConfigHash::of_text("cfg").as_str());
    let ok = GateSummary::from_verdict(&verdict(&[]), t(18));
    assert!(ok.passed);
    assert_eq!(ok.hard_failures, Vec::<RuleId>::new());
}

#[test]
fn bad_evidence_numbers_are_rejected() {
    for f in [
        (|e: &mut Evidence| e.win_rate = f64::NAN) as fn(&mut Evidence),
        |e| e.win_rate = 1.7,
        |e| e.profit_factor = f64::INFINITY,
        |e| e.avg_r = f64::NAN,
        |e| e.max_drawdown_pct = -1.0,
    ] {
        let mut p = parts();
        let mut e = good_evidence(30);
        f(&mut e);
        p.backtest = Some(e);
        assert_eq!(
            Recommendation::new(p, 30, 55).unwrap_err(),
            RecError::BadEvidence
        );
    }
}

#[test]
fn gate_summary_must_agree_with_its_lists() {
    let mut p = parts();
    p.gate.hard_failures = vec![RuleId::G02];
    assert_eq!(
        Recommendation::new(p.clone(), 30, 55).unwrap_err(),
        RecError::GateInconsistent
    );
    p.gate.passed = false;
    assert!(Recommendation::new(p, 30, 55).is_ok());
}

#[test]
fn existing_low_evidence_warning_is_not_duplicated() {
    let mut p = parts();
    p.warnings = vec![Warning::LowEvidence];
    let r = Recommendation::new(p, 30, 55).unwrap();
    assert_eq!(r.warnings(), &[Warning::LowEvidence]);
}

#[test]
fn horizon_is_part_of_equality() {
    let a = parts();
    assert_eq!(a, a.clone());
}

#[test]
fn missing_input_failure_counts_as_hard() {
    let rs: Vec<RuleResult> = RuleId::EVALUATION_ORDER
        .iter()
        .map(|r| {
            if *r == RuleId::G05 {
                RuleResult::input_missing(*r, "?", "y", "supply it")
            } else {
                RuleResult::pass(*r, "x", "y")
            }
        })
        .collect();
    let v = GateVerdict::new(ConfigHash::of_text("cfg"), &rs).unwrap();
    let s = GateSummary::from_verdict(&v, t(18));
    assert_eq!(s.hard_failures, vec![RuleId::G05]);
    assert_eq!(s.overridable_failures, Vec::<RuleId>::new());
}

#[test]
fn paper_only_failures_are_the_paper_level_subset_of_the_overridable_ones() {
    // G05 is a soft override, G01 overridable in paper mode only, G02 never.
    let s = GateSummary::from_verdict(&verdict(&[RuleId::G01, RuleId::G02, RuleId::G05]), t(18));
    assert_eq!(s.hard_failures, vec![RuleId::G02]);
    assert_eq!(s.overridable_failures, vec![RuleId::G01, RuleId::G05]);
    assert_eq!(s.paper_only, vec![RuleId::G01]);
    let ok = GateSummary::from_verdict(&verdict(&[]), t(18));
    assert_eq!(ok.paper_only, Vec::<RuleId>::new());
}

#[test]
fn paper_only_outside_the_overridable_list_is_rejected_naming_the_rule() {
    let mut p = parts();
    p.gate = GateSummary::from_verdict(&verdict(&[RuleId::G05]), t(18));
    p.gate.paper_only = vec![RuleId::G01];
    assert_eq!(
        Recommendation::new(p.clone(), 30, 55).unwrap_err(),
        RecError::PaperOnlyNotOverridable
    );
    p.gate.paper_only = vec![];
    assert!(Recommendation::new(p, 30, 55).is_ok());
}

#[test]
fn a_stated_reward_risk_that_does_not_match_the_levels_is_rejected() {
    // (110 - 102) / (102 - 96) = 1.3333...; the stated 2 is a copy of a different calculation.
    let mut p = parts();
    p.reward_risk = d(2);
    assert_eq!(
        Recommendation::new(p.clone(), 30, 55).unwrap_err(),
        RecError::RewardRiskMismatch
    );
    // The best-case fill (entry low) would give 10 / 4 = 2.5: also refused.
    p.reward_risk = reward_risk(d(100), d(96), d(110)).unwrap();
    assert_eq!(
        Recommendation::new(p, 30, 55).unwrap_err(),
        RecError::RewardRiskMismatch
    );
}

#[test]
fn reward_risk_is_none_when_stop_equals_entry() {
    assert_eq!(reward_risk(d(100), d(100), d(110)), None);
    assert_eq!(reward_risk(d(102), d(96), d(110)), Some(d(8) / d(6)));
}

#[test]
fn a_sell_reward_risk_on_the_best_fill_is_refused() {
    let mut p = parts();
    p.action = Action::Sell;
    p.stop_loss = d(106);
    p.targets = vec![d(90)];
    // Worst sell fill is the zone low (100): 10 / 6. The zone high (102) would give 12 / 4 = 3.
    p.reward_risk = reward_risk(d(102), d(106), d(90)).unwrap();
    assert_eq!(
        Recommendation::new(p.clone(), 30, 55).unwrap_err(),
        RecError::RewardRiskMismatch
    );
    p.reward_risk = reward_risk(d(100), d(106), d(90)).unwrap();
    assert!(Recommendation::new(p, 30, 55).is_ok());
}

#[test]
fn paper_only_must_be_exactly_the_paper_level_failures() {
    // G01 is Paper level: leaving it out of `paper_only` would make it look like a Soft override.
    let mut p = parts();
    p.gate = GateSummary::from_verdict(&verdict(&[RuleId::G01, RuleId::G05]), t(18));
    assert_eq!(p.gate.paper_only, vec![RuleId::G01]);
    assert!(Recommendation::new(p.clone(), 30, 55).is_ok());
    p.gate.paper_only = vec![];
    assert_eq!(
        Recommendation::new(p, 30, 55).unwrap_err(),
        RecError::PaperOnlyNotOverridable
    );
}

#[test]
fn approval_pending_flag_is_accepted_only_for_exactly_g16() {
    let mut p = parts();
    p.gate = GateSummary::from_verdict(&verdict(&[RuleId::G16]), t(18));
    p.gate.approval_pending = true; // hand-set: the verdict never marked G16 pending
    assert!(Recommendation::new(p.clone(), 30, 55).is_ok());
    // Flag with a passed gate, a different rule, or extra failures is refused.
    let mut q = parts();
    q.gate.approval_pending = true;
    assert_eq!(
        Recommendation::new(q, 30, 55).unwrap_err(),
        RecError::ApprovalPendingInconsistent
    );
    for failing in [
        &[RuleId::G02][..],
        &[RuleId::G16, RuleId::G02][..],
        &[RuleId::G16, RuleId::G05][..],
    ] {
        let mut q = parts();
        q.gate = GateSummary::from_verdict(&verdict(failing), t(18));
        q.gate.approval_pending = true;
        assert_eq!(
            Recommendation::new(q, 30, 55).unwrap_err(),
            RecError::ApprovalPendingInconsistent,
            "{failing:?}"
        );
    }
}
