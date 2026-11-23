//! What a scan returns: gate-passed ideas, blocked ideas with the reason, and every drop.
//! No `Default`: the verdict is always derived from the lists by [`ScanResult::new`].

use bazaar_risk::verdict::RuleId;
use rust_decimal::Decimal;

use crate::types::{InstrumentRef, Recommendation};
use crate::universe::DropReason;

/// Why an idea is shown as blocked and not as a recommendation. Each names the check that refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum BlockReason {
    #[error("gate inputs unavailable: {0}")]
    GateInputsUnavailable(String),
    #[error("could not be sized: {0}")]
    Sizing(String),
    #[error("tick rounding broke the levels: {0}")]
    TickCollapse(String),
    #[error("risk including charges exceeds the budget of one trade")]
    RiskAboveBudget,
    #[error("limits changed between sizing and the gate")]
    LimitsChanged,
    #[error("the gate could not evaluate: {0}")]
    GateReject(String),
    /// A never-overridable rule failed (the rules are named; `ScanResult::new` reroutes these).
    #[error("hard gate rule failed: {}", rule_names(.0))]
    GateHardFail(Vec<RuleId>),
}

fn rule_names(rules: &[RuleId]) -> String {
    rules
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// An idea that cannot be a `Recommendation` (no valid size or no gate result). The levels are
/// those the strategy gave, not rounded or sized. It is advice to look, never an order.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockedIdea {
    pub instrument: InstrumentRef,
    pub entry_zone: (Decimal, Decimal),
    pub stop_loss: Decimal,
    pub targets: Vec<Decimal>,
    pub reason: BlockReason,
}

/// The one-line outcome of a scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanVerdict {
    /// At least one sized idea passed the gate.
    Setups { count: usize },
    /// "No high-quality setups today", with the reasons (never empty).
    NoHighQualitySetups { reasons: Vec<String> },
}

/// Ranked ideas that passed the gate, blocked ideas (never truncated, same rank order) and drops.
#[derive(Debug, Clone, PartialEq)]
pub struct ScanResult {
    ideas: Vec<Recommendation>,
    blocked: Vec<BlockedIdea>,
    dropped: Vec<(String, DropReason)>,
    verdict: ScanVerdict,
}

impl ScanResult {
    /// `ideas` are ranked and truncated to `max_results`. An idea with a hard (never overridable)
    /// gate failure is moved to `blocked` with the failed rules, so it can never count as a setup.
    /// Failures that an override could accept (soft or paper level) stay in `ideas`, flagged by
    /// their `GateSummary`, because the user may still decide on them.
    #[must_use]
    pub fn new(
        all_ideas: Vec<Recommendation>,
        mut blocked: Vec<BlockedIdea>,
        dropped: Vec<(String, DropReason)>,
    ) -> Self {
        let (mut ideas, mut hard) = (Vec::new(), Vec::new());
        for r in all_ideas {
            if r.gate().hard_failures.is_empty() {
                ideas.push(r);
            } else {
                hard.push(r);
            }
        }
        blocked.extend(hard.into_iter().map(|r| BlockedIdea {
            instrument: r.instrument().clone(),
            entry_zone: r.entry_zone(),
            stop_loss: r.stop_loss(),
            targets: r.targets().to_vec(),
            reason: BlockReason::GateHardFail(r.gate().hard_failures.clone()),
        }));
        let verdict = if ideas.is_empty() {
            let mut reasons: Vec<String> = blocked
                .iter()
                .map(|b| format!("{}: {}", b.instrument.tradingsymbol, b.reason))
                .collect();
            if reasons.is_empty() {
                // A name can be dropped more than once (one expired signal per strategy): count names.
                let names: std::collections::BTreeSet<&str> =
                    dropped.iter().map(|(n, _)| n.as_str()).collect();
                reasons.push(format!(
                    "no strategy fired on a name that passed the filters ({} dropped)",
                    names.len()
                ));
            }
            ScanVerdict::NoHighQualitySetups { reasons }
        } else {
            ScanVerdict::Setups { count: ideas.len() }
        };
        Self {
            ideas,
            blocked,
            dropped,
            verdict,
        }
    }
    #[must_use]
    pub fn ideas(&self) -> &[Recommendation] {
        &self.ideas
    }
    #[must_use]
    pub fn blocked(&self) -> &[BlockedIdea] {
        &self.blocked
    }
    #[must_use]
    pub fn dropped(&self) -> &[(String, DropReason)] {
        &self.dropped
    }
    #[must_use]
    pub const fn verdict(&self) -> &ScanVerdict {
        &self.verdict
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocked(sym: &str, reason: BlockReason) -> BlockedIdea {
        BlockedIdea {
            instrument: InstrumentRef {
                instrument_token: 1,
                tradingsymbol: sym.into(),
                name: sym.into(),
            },
            entry_zone: (Decimal::from(100), Decimal::from(102)),
            stop_loss: Decimal::from(96),
            targets: vec![Decimal::from(110)],
            reason,
        }
    }

    #[test]
    fn no_ideas_gives_no_setups_with_every_blocked_reason() {
        let r = ScanResult::new(
            vec![],
            vec![
                blocked("AAA", BlockReason::LimitsChanged),
                blocked("BBB", BlockReason::RiskAboveBudget),
            ],
            vec![],
        );
        assert_eq!(
            *r.verdict(),
            ScanVerdict::NoHighQualitySetups {
                reasons: vec![
                    "AAA: limits changed between sizing and the gate".into(),
                    "BBB: risk including charges exceeds the budget of one trade".into()
                ]
            }
        );
        assert_eq!(r.blocked().len(), 2);
    }

    fn rec_with_failures(failing: &[RuleId]) -> Recommendation {
        use crate::types::{Action, Horizon, PositionSize, RecommendationParts, WhyFactor};
        use bazaar_regime::{Fact, RegimeState};
        use bazaar_risk::config::ConfigHash;
        use bazaar_risk::verdict::{GateVerdict, RuleResult};
        use chrono::TimeZone;
        let t = chrono_tz::Asia::Kolkata
            .with_ymd_and_hms(2026, 11, 13, 16, 0, 0)
            .unwrap();
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
        let v = GateVerdict::new(ConfigHash::of_text("cfg"), &rs).unwrap();
        let d = Decimal::from;
        Recommendation::new(
            RecommendationParts {
                id: 1,
                generated_at: t,
                data_as_of: t,
                instrument: blocked("ABC", BlockReason::LimitsChanged).instrument,
                action: Action::Buy,
                entry_zone: (d(100), d(102)),
                stop_loss: d(96),
                targets: vec![d(110)],
                horizon: Horizon::Swing,
                size: PositionSize { qty: 10, lots: 1 },
                risk_rupees: d(60),
                reward_risk: crate::types::reward_risk(d(102), d(96), d(110)).unwrap(),
                agreeing_strategies: vec!["a".into()],
                confidence: 70,
                regime: RegimeState::unknown(t, Fact::new("a", "b", "c")),
                backtest: None,
                why: vec![WhyFactor::Liquidity {
                    median_traded_value: d(1),
                }],
                warnings: vec![],
                gate: crate::types::GateSummary::from_verdict(&v, t),
            },
            30,
            55,
        )
        .unwrap()
    }

    #[test]
    fn a_hard_gate_failure_is_blocked_with_the_rule_never_a_setup() {
        let r = ScanResult::new(vec![rec_with_failures(&[RuleId::G02])], vec![], vec![]);
        assert_eq!(r.ideas().len(), 0);
        assert_eq!(
            r.blocked()[0].reason,
            BlockReason::GateHardFail(vec![RuleId::G02])
        );
        assert!(matches!(
            r.verdict(),
            ScanVerdict::NoHighQualitySetups { reasons } if reasons[0] == "ABC: hard gate rule failed: G02"
        ));
        // A paper-level failure alone can still be overridden, so it stays an idea.
        let r = ScanResult::new(vec![rec_with_failures(&[RuleId::G01])], vec![], vec![]);
        assert_eq!(*r.verdict(), ScanVerdict::Setups { count: 1 });
    }

    #[test]
    fn the_dropped_count_is_distinct_names() {
        let d = |n: &str, r| (n.to_owned(), r);
        let r = ScanResult::new(
            vec![],
            vec![],
            vec![
                d("X", DropReason::SignalExpired("a".into())),
                d("X", DropReason::SignalExpired("b".into())),
                d("Y", DropReason::Banned),
            ],
        );
        assert_eq!(
            *r.verdict(),
            ScanVerdict::NoHighQualitySetups {
                reasons: vec![
                    "no strategy fired on a name that passed the filters (2 dropped)".into()
                ]
            }
        );
    }

    #[test]
    fn an_empty_scan_still_explains_itself() {
        let r = ScanResult::new(vec![], vec![], vec![("X".into(), DropReason::Banned)]);
        assert_eq!(
            *r.verdict(),
            ScanVerdict::NoHighQualitySetups {
                reasons: vec![
                    "no strategy fired on a name that passed the filters (1 dropped)".into()
                ]
            }
        );
        assert_eq!(r.dropped().len(), 1);
    }
}
