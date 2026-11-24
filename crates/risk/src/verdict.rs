//! Rule ids, the single override-level table, rule results and the gate verdict.

use crate::config::ConfigHash;
use crate::override_::IntentDigest;

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
/// Fields are private: build with `pass` or `fail`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleResult {
    rule: RuleId,
    passed: bool,
    observed: String,
    limit: String,
    message: String,
    /// True when the rule failed because an input was missing or unknown, not because a limit
    /// was breached. Such a failure can never be overridden.
    input_missing: bool,
    /// True only when G16 failed and the sole reason is the missing `semi_auto` approval. Set
    /// inside the crate by `g16`; no public setter and never an input anywhere.
    approval_pending: bool,
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
            input_missing: false,
            approval_pending: false,
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
            input_missing: false,
            approval_pending: false,
        }
    }

    /// The G16 failure whose only reason is the missing approval. Crate-private: only `g16`
    /// calls it, so a caller cannot mark a result.
    pub(crate) fn g16_approval_pending(
        observed: impl Into<String>,
        limit: impl Into<String>,
        message: &str,
    ) -> Self {
        let mut r = Self::fail(RuleId::G16, observed, limit, message);
        r.approval_pending = true;
        r
    }

    /// True for a G16 failure caused only by the missing `semi_auto` approval.
    #[must_use]
    pub const fn is_approval_pending(&self) -> bool {
        self.approval_pending && !self.passed && matches!(self.rule, RuleId::G16)
    }

    /// A failing result because an input is missing or unknown (fail closed). `message` says
    /// what would make it pass. Never overridable, whatever the rule's override level.
    #[must_use]
    pub fn input_missing(
        rule: RuleId,
        observed: impl Into<String>,
        limit: impl Into<String>,
        message: &str,
    ) -> Self {
        let mut r = Self::fail(rule, observed, limit, message);
        r.input_missing = true;
        r
    }

    /// True when the failure is a missing or unknown input.
    #[must_use]
    pub const fn is_input_missing(&self) -> bool {
        self.input_missing
    }

    /// The rule this result belongs to.
    #[must_use]
    pub const fn rule(&self) -> RuleId {
        self.rule
    }

    /// True when the rule passed.
    #[must_use]
    pub const fn passed(&self) -> bool {
        self.passed
    }

    /// The observed value, formatted.
    #[must_use]
    pub fn observed(&self) -> &str {
        &self.observed
    }

    /// The limit, formatted.
    #[must_use]
    pub fn limit(&self) -> &str {
        &self.limit
    }

    /// Plain-English message; failures end with the rule id in brackets.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The override level of this rule (from the single table).
    #[must_use]
    pub const fn level(&self) -> OverrideLevel {
        self.rule.override_level()
    }
}

/// A verdict that does not hold exactly one result per rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictError {
    /// No results at all.
    Empty,
    /// A rule in `EVALUATION_ORDER` has no result.
    Missing(RuleId),
    /// A rule has more than one result.
    Duplicate(RuleId),
}

impl std::fmt::Display for VerdictError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("the gate produced no rule results"),
            Self::Missing(r) => write!(f, "the gate produced no result for rule {r}"),
            Self::Duplicate(r) => write!(f, "the gate produced two results for rule {r}"),
        }
    }
}

impl std::error::Error for VerdictError {}

/// All rule results for one intent, with the hash of the config they ran under. Built only by
/// [`GateVerdict::new`], which requires every rule exactly once, so a partial verdict cannot pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateVerdict {
    config_hash: ConfigHash,
    results: Vec<RuleResult>,
    intent_digest: Option<IntentDigest>,
}

impl GateVerdict {
    /// Check that every id in `EVALUATION_ORDER` appears exactly once, then keep the results in
    /// evaluation order.
    pub fn new(config_hash: ConfigHash, results: &[RuleResult]) -> Result<Self, VerdictError> {
        if results.is_empty() {
            return Err(VerdictError::Empty);
        }
        let mut ordered = Vec::with_capacity(RuleId::EVALUATION_ORDER.len());
        for rule in RuleId::EVALUATION_ORDER {
            let mut found = results.iter().filter(|r| r.rule == rule);
            let first = found.next().ok_or(VerdictError::Missing(rule))?;
            if found.next().is_some() {
                return Err(VerdictError::Duplicate(rule));
            }
            ordered.push(first.clone());
        }
        Ok(Self {
            config_hash,
            results: ordered,
            intent_digest: None,
        })
    }

    /// Record which intent this verdict was computed for. The gate calls this; an override is
    /// refused for a verdict that is not bound to the intent being overridden.
    #[must_use]
    pub fn bound_to(mut self, digest: IntentDigest) -> Self {
        self.intent_digest = Some(digest);
        self
    }

    /// The digest of the intent this verdict was computed for, if bound.
    #[must_use]
    pub const fn intent_digest(&self) -> Option<&IntentDigest> {
        self.intent_digest.as_ref()
    }

    /// The hash of the config the rules ran under.
    #[must_use]
    pub const fn config_hash(&self) -> &ConfigHash {
        &self.config_hash
    }

    /// Every rule result, in evaluation order.
    #[must_use]
    pub fn results(&self) -> &[RuleResult] {
        &self.results
    }

    /// True when every rule passed (a verdict always holds every rule).
    #[must_use]
    pub fn passed(&self) -> bool {
        self.results.iter().all(|r| r.passed)
    }

    /// Failed results, in evaluation order.
    pub fn failures(&self) -> impl Iterator<Item = &RuleResult> {
        self.results.iter().filter(|r| !r.passed)
    }

    /// True when something failed and every failure is G16 carrying the approval-pending flag:
    /// the only gap is the missing approval. Never makes `passed()` true.
    #[must_use]
    pub fn approval_pending_only(&self) -> bool {
        let mut any = false;
        for r in self.failures() {
            if !r.is_approval_pending() {
                return false;
            }
            any = true;
        }
        any
    }

    /// True when there is at least one failure and every failure is Soft or Paper and a real breach. Any Hard, No or missing-input failure
    /// makes the whole verdict non-overridable.
    #[must_use]
    pub fn overridable(&self) -> bool {
        let mut any = false;
        for r in self.failures() {
            if r.is_input_missing() || !r.level().is_overridable() {
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

    /// Compile-time exhaustiveness: adding a `RuleId` variant breaks the match in `check` until
    /// `ALL`, `GOLDEN` and `EVALUATION_ORDER` are updated too.
    fn all_rules() -> Vec<RuleId> {
        const fn check(r: RuleId) {
            match r {
                RuleId::G01
                | RuleId::G02
                | RuleId::G03
                | RuleId::G04a
                | RuleId::G04b
                | RuleId::G05
                | RuleId::G06
                | RuleId::G07
                | RuleId::G08
                | RuleId::G09
                | RuleId::G10
                | RuleId::G11
                | RuleId::G13
                | RuleId::G14
                | RuleId::G15
                | RuleId::G16
                | RuleId::G17
                | RuleId::G18
                | RuleId::G19
                | RuleId::X01
                | RuleId::X02
                | RuleId::X03 => {}
            }
        }
        let all = vec![
            RuleId::G01,
            RuleId::G02,
            RuleId::G03,
            RuleId::G04a,
            RuleId::G04b,
            RuleId::G05,
            RuleId::G06,
            RuleId::G07,
            RuleId::G08,
            RuleId::G09,
            RuleId::G10,
            RuleId::G11,
            RuleId::G13,
            RuleId::G14,
            RuleId::G15,
            RuleId::G16,
            RuleId::G17,
            RuleId::G18,
            RuleId::G19,
            RuleId::X01,
            RuleId::X02,
            RuleId::X03,
        ];
        for r in &all {
            check(*r);
        }
        all
    }

    #[test]
    fn all_list_equals_order_and_golden() {
        let mut all = all_rules();
        let mut order = RuleId::EVALUATION_ORDER.to_vec();
        let mut golden: Vec<RuleId> = GOLDEN.iter().map(|(r, _)| *r).collect();
        all.sort();
        order.sort();
        golden.sort();
        assert_eq!(all, order);
        assert_eq!(all, golden);
        all.dedup();
        assert_eq!(all.len(), RuleId::EVALUATION_ORDER.len(), "duplicate id");
    }

    #[test]
    fn override_levels_match_the_golden_table() {
        for (rule, level) in GOLDEN {
            assert_eq!(rule.override_level(), level, "{rule}");
        }
    }

    #[test]
    fn evaluation_order_is_pinned() {
        use RuleId::*;
        let want = [
            G16, G15, G03, G08, G19, G18, G17, X01, X02, X03, G06, G07, G13, G09, G10, G11, G02,
            G14, G04a, G04b, G01, G05,
        ];
        assert_eq!(RuleId::EVALUATION_ORDER, want);
        let n = RuleId::EVALUATION_ORDER.len();
        assert_eq!(n, GOLDEN.len());
        assert_eq!(RuleId::EVALUATION_ORDER.get(n - 2..), Some(&[G01, G05][..]));
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
        assert!(!r.passed());
        assert!(r.message().ends_with("(G03)"), "{}", r.message());
        assert_eq!(r.level(), HardCeiling);
        assert_eq!(
            (r.rule(), r.observed(), r.limit()),
            (RuleId::G03, "-Rs 3,050", "Rs 3,000")
        );
        assert!(RuleResult::pass(RuleId::G02, "a", "b").passed());
    }

    fn hash() -> ConfigHash {
        ConfigHash::of_text("x")
    }

    fn pass_all() -> Vec<RuleResult> {
        all_rules()
            .into_iter()
            .map(|r| RuleResult::pass(r, "a", "b"))
            .collect()
    }

    fn with_fail(rules: &[RuleId]) -> GateVerdict {
        let results: Vec<RuleResult> = pass_all()
            .into_iter()
            .map(|r| {
                if rules.contains(&r.rule()) {
                    RuleResult::fail(r.rule(), "o", "l", "no")
                } else {
                    r
                }
            })
            .collect();
        GateVerdict::new(hash(), &results).unwrap()
    }

    #[test]
    fn full_pass_is_a_pass_in_evaluation_order() {
        let mut shuffled = pass_all();
        shuffled.reverse();
        let v = GateVerdict::new(hash(), &shuffled).unwrap();
        assert!(v.passed() && !v.overridable());
        assert_eq!(v.failures().count(), 0);
        assert_eq!(v.results().len(), 22);
        let got: Vec<RuleId> = v.results().iter().map(RuleResult::rule).collect();
        assert_eq!(got, RuleId::EVALUATION_ORDER.to_vec());
        assert_eq!(v.config_hash(), &hash());
    }

    #[test]
    fn partial_empty_duplicate_and_missing_verdicts_are_errors() {
        assert_eq!(GateVerdict::new(hash(), &[]), Err(VerdictError::Empty));
        let partial = vec![RuleResult::pass(RuleId::G02, "a", "b")];
        assert!(matches!(
            GateVerdict::new(hash(), &partial),
            Err(VerdictError::Missing(_))
        ));

        let mut dup = pass_all();
        dup.push(RuleResult::fail(RuleId::G03, "o", "l", "no"));
        assert_eq!(
            GateVerdict::new(hash(), &dup),
            Err(VerdictError::Duplicate(RuleId::G03))
        );

        let mut missing = pass_all();
        missing.retain(|r| r.rule() != RuleId::G10);
        assert_eq!(
            GateVerdict::new(hash(), &missing),
            Err(VerdictError::Missing(RuleId::G10))
        );
        for e in [
            VerdictError::Empty,
            VerdictError::Missing(RuleId::G10),
            VerdictError::Duplicate(RuleId::G03),
        ] {
            assert!(e.to_string().contains("rule") || e.to_string().contains("results"));
        }
    }

    #[test]
    fn input_missing_failure_is_never_overridable_even_on_a_soft_rule() {
        let results: Vec<RuleResult> = pass_all()
            .into_iter()
            .map(|r| match r.rule() {
                RuleId::G05 => RuleResult::input_missing(RuleId::G05, "o", "l", "no"),
                RuleId::G14 => RuleResult::fail(RuleId::G14, "o", "l", "no"),
                _ => r,
            })
            .collect();
        assert!(!GateVerdict::new(hash(), &results).unwrap().overridable());
    }

    fn pending(rule_fail: &[RuleId], pend: bool) -> GateVerdict {
        let results: Vec<RuleResult> = pass_all()
            .into_iter()
            .map(|r| match r.rule() {
                RuleId::G16 if pend => RuleResult::g16_approval_pending("o", "l", "no"),
                x if rule_fail.contains(&x) => RuleResult::fail(x, "o", "l", "no"),
                _ => r,
            })
            .collect();
        GateVerdict::new(hash(), &results).unwrap()
    }

    #[test]
    fn pending_only_when_g16_flagged_is_the_sole_failure() {
        let v = pending(&[], true);
        assert!(v.approval_pending_only() && !v.passed() && !v.overridable());
        assert_eq!(v.failures().count(), 1);
        // Same G16 failure without the flag, another failure beside it, or nothing failing.
        assert!(!with_fail(&[RuleId::G16]).approval_pending_only());
        assert!(!pending(&[RuleId::G01], true).approval_pending_only());
        assert!(!pending(&[RuleId::G03], true).approval_pending_only());
        assert!(!with_fail(&[RuleId::G01]).approval_pending_only());
        assert!(
            !GateVerdict::new(hash(), &pass_all())
                .unwrap()
                .approval_pending_only()
        );
        // The flag changes no level, message or equality of the failing rule's facts.
        let (a, b) = (
            RuleResult::g16_approval_pending("o", "l", "no"),
            RuleResult::fail(RuleId::G16, "o", "l", "no"),
        );
        assert_eq!(
            (a.level(), a.message(), a.passed()),
            (b.level(), b.message(), b.passed())
        );
        assert!(a.is_approval_pending() && !b.is_approval_pending());
    }

    #[test]
    fn hard_or_no_failure_blocks_the_whole_override() {
        assert!(with_fail(&[RuleId::G14, RuleId::G05]).overridable());
        assert!(!with_fail(&[RuleId::G14, RuleId::G03]).overridable());
        assert!(!with_fail(&[RuleId::G05, RuleId::G17]).overridable());
        assert!(with_fail(&[RuleId::G01]).overridable());
        assert!(!with_fail(&[RuleId::G01]).passed());
    }
}
