//! Rule ids, the single override-level table, rule results and the gate verdict.

use crate::config::ConfigHash;

/// Every rule the gate evaluates. G12 (liquidity) is not in F6 scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RuleId {
    G01,
    G02,
    G03,
    G04a,
    G04b,
    G05,
    G06,
    G07,
    G08,
    G09,
    G10,
    G11,
    G13,
    G14,
    G15,
    G16,
    G17,
    G18,
    G19,
    X01,
    X02,
    X03,
}

/// Override column of the discipline skill's G-table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverrideLevel {
    /// Never overridable; the loader also enforces a `[hard_limits]` ceiling.
    HardCeiling,
    /// Never overridable (behaviour guard, data or mode rule).
    No,
    /// One typed override with a reason.
    Soft,
    /// Overridable in `paper` mode only.
    Paper,
}

impl OverrideLevel {
    /// True when an override can ever accept this failure (mode checks happen in the override engine).
    #[must_use]
    pub const fn is_overridable(self) -> bool {
        matches!(self, Self::Soft | Self::Paper)
    }
}

impl RuleId {
    /// Evaluation order from the skill: G16, G15, G03, G08, structural rules, then G01 and exposure.
    /// Every rule runs even after a failure.
    pub const EVALUATION_ORDER: [Self; 22] = [
        Self::G16,
        Self::G15,
        Self::G03,
        Self::G08,
        Self::G19,
        Self::G18,
        Self::G17,
        Self::X01,
        Self::X02,
        Self::X03,
        Self::G06,
        Self::G07,
        Self::G13,
        Self::G09,
        Self::G10,
        Self::G11,
        Self::G02,
        Self::G14,
        Self::G04a,
        Self::G04b,
        Self::G01,
        Self::G05,
    ];

    /// The id as printed in messages.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::G01 => "G01",
            Self::G02 => "G02",
            Self::G03 => "G03",
            Self::G04a => "G04a",
            Self::G04b => "G04b",
            Self::G05 => "G05",
            Self::G06 => "G06",
            Self::G07 => "G07",
            Self::G08 => "G08",
            Self::G09 => "G09",
            Self::G10 => "G10",
            Self::G11 => "G11",
            Self::G13 => "G13",
            Self::G14 => "G14",
            Self::G15 => "G15",
            Self::G16 => "G16",
            Self::G17 => "G17",
            Self::G18 => "G18",
            Self::G19 => "G19",
            Self::X01 => "X01",
            Self::X02 => "X02",
            Self::X03 => "X03",
        }
    }

    /// The one override table. Rule files cannot set a level. X01-X03 are not in the skill
    /// table; they are data/mode/structure checks and are `No` (owner to confirm).
    #[must_use]
    pub const fn override_level(self) -> OverrideLevel {
        match self {
            Self::G02 | Self::G03 | Self::G04b | Self::G09 | Self::G11 | Self::G16 | Self::G19 => {
                OverrideLevel::HardCeiling
            }
            Self::G04a | Self::G05 | Self::G07 | Self::G14 => OverrideLevel::Soft,
            Self::G01 => OverrideLevel::Paper,
            Self::G06
            | Self::G08
            | Self::G10
            | Self::G13
            | Self::G15
            | Self::G17
            | Self::G18
            | Self::X01
            | Self::X02
            | Self::X03 => OverrideLevel::No,
        }
    }
}

impl std::fmt::Display for RuleId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Outcome of one rule: observed value, limit, pass or fail, and a plain-English message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleResult {
    pub rule: RuleId,
    pub passed: bool,
    pub observed: String,
    pub limit: String,
    /// Failures end with the rule id in brackets, e.g. `... (G03)`.
    pub message: String,
}

impl RuleResult {
    /// A passing result.
    #[must_use]
    pub fn pass(rule: RuleId, observed: impl Into<String>, limit: impl Into<String>) -> Self {
        let (observed, limit) = (observed.into(), limit.into());
        Self {
            rule,
            passed: true,
            message: format!("{observed} is within {limit} ({rule})"),
            observed,
            limit,
        }
    }

    /// A failing result; `message` is the plain-English reason and the rule id is appended.
    #[must_use]
    pub fn fail(
        rule: RuleId,
        observed: impl Into<String>,
        limit: impl Into<String>,
        message: &str,
    ) -> Self {
        Self {
            rule,
            passed: false,
            observed: observed.into(),
            limit: limit.into(),
            message: format!("{} ({rule})", message.trim_end()),
        }
    }

    /// The override level of this rule (from the single table).
    #[must_use]
    pub const fn level(&self) -> OverrideLevel {
        self.rule.override_level()
    }
}

/// All rule results for one intent, with the hash of the config they ran under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateVerdict {
    pub config_hash: ConfigHash,
    pub results: Vec<RuleResult>,
}

impl GateVerdict {
    /// An empty verdict is not a pass: no rules ran.
    #[must_use]
    pub fn passed(&self) -> bool {
        !self.results.is_empty() && self.results.iter().all(|r| r.passed)
    }

    /// Failed results, in evaluation order.
    pub fn failures(&self) -> impl Iterator<Item = &RuleResult> {
        self.results.iter().filter(|r| !r.passed)
    }

    /// True when there is at least one failure and every failure is Soft or Paper. Any Hard or
    /// No failure makes the whole verdict non-overridable.
    #[must_use]
    pub fn overridable(&self) -> bool {
        let mut any = false;
        for r in self.failures() {
            if !r.level().is_overridable() {
                return false;
            }
            any = true;
        }
        any
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use OverrideLevel::{HardCeiling, No, Paper, Soft};

    /// Expected levels as literals (the skill G-table, owner decisions 2026-11-19). Never parsed
    /// from SKILL.md: a change to the table must be a deliberate change here too.
    const GOLDEN: [(RuleId, OverrideLevel); 22] = [
        (RuleId::G01, Paper),
        (RuleId::G02, HardCeiling),
        (RuleId::G03, HardCeiling),
        (RuleId::G04a, Soft),
        (RuleId::G04b, HardCeiling),
        (RuleId::G05, Soft),
        (RuleId::G06, No),
        (RuleId::G07, Soft),
        (RuleId::G08, No),
        (RuleId::G09, HardCeiling),
        (RuleId::G10, No),
        (RuleId::G11, HardCeiling),
        (RuleId::G13, No),
        (RuleId::G14, Soft),
        (RuleId::G15, No),
        (RuleId::G16, HardCeiling),
        (RuleId::G17, No),
        (RuleId::G18, No),
        (RuleId::G19, HardCeiling),
        (RuleId::X01, No),
        (RuleId::X02, No),
        (RuleId::X03, No),
    ];

    #[test]
    fn override_levels_match_the_golden_table() {
        for (rule, level) in GOLDEN {
            assert_eq!(rule.override_level(), level, "{rule}");
        }
    }

    #[test]
    fn evaluation_order_covers_every_rule_once_in_skill_order() {
        let order = RuleId::EVALUATION_ORDER;
        assert_eq!(
            &order[..4],
            &[RuleId::G16, RuleId::G15, RuleId::G03, RuleId::G08]
        );
        let mut sorted = order.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), GOLDEN.len());
        for (rule, _) in GOLDEN {
            assert!(
                order.contains(&rule),
                "{rule} missing from evaluation order"
            );
        }
        // G01 and the exposure rules run last.
        assert_eq!(&order[20..], &[RuleId::G01, RuleId::G05]);
    }

    #[test]
    fn rule_ids_print_their_name() {
        for (rule, _) in GOLDEN {
            assert_eq!(rule.to_string(), rule.as_str());
            assert_eq!(format!("{rule:?}"), rule.as_str());
        }
    }

    #[test]
    fn only_soft_and_paper_are_overridable() {
        assert!(Soft.is_overridable() && Paper.is_overridable());
        assert!(!HardCeiling.is_overridable() && !No.is_overridable());
    }

    #[test]
    fn failure_message_ends_with_the_rule_id() {
        let r = RuleResult::fail(
            RuleId::G03,
            "-Rs 3,050",
            "Rs 3,000",
            "You are down Rs 3,050. ",
        );
        assert!(!r.passed);
        assert!(r.message.ends_with("(G03)"), "{}", r.message);
        assert_eq!(r.level(), HardCeiling);
        assert!(RuleResult::pass(RuleId::G02, "a", "b").passed);
    }

    fn verdict(results: Vec<RuleResult>) -> GateVerdict {
        GateVerdict {
            config_hash: ConfigHash::of_text("x"),
            results,
        }
    }

    fn fail(rule: RuleId) -> RuleResult {
        RuleResult::fail(rule, "o", "l", "no")
    }

    #[test]
    fn empty_verdict_is_not_a_pass() {
        assert!(!verdict(vec![]).passed());
        assert!(!verdict(vec![]).overridable());
    }

    #[test]
    fn all_pass_is_a_pass_and_not_overridable_input() {
        let v = verdict(vec![RuleResult::pass(RuleId::G02, "a", "b")]);
        assert!(v.passed() && !v.overridable());
        assert_eq!(v.failures().count(), 0);
    }

    #[test]
    fn hard_or_no_failure_blocks_the_whole_override() {
        assert!(verdict(vec![fail(RuleId::G14), fail(RuleId::G05)]).overridable());
        assert!(!verdict(vec![fail(RuleId::G14), fail(RuleId::G03)]).overridable());
        assert!(!verdict(vec![fail(RuleId::G05), fail(RuleId::G17)]).overridable());
        assert!(verdict(vec![fail(RuleId::G01)]).overridable());
    }
}
