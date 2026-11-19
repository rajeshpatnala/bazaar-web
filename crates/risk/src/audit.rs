//! Audit record for an accepted override and the acknowledged-write API.
//!
//! The record is written through an [`AuditSink`] and must be acknowledged before the gate may
//! mint a `GatedIntent` (the ordering is enforced in the gate, slice 11b, which must take the
//! receipt by value, test that a failing sink mints nothing, and compare the receipt's digest
//! and config hash with the verdict being minted). [`AuditReceipt`] has a private constructor:
//! the only way to get one is a sink that returned `Ok`, so a mint step that demands a receipt
//! cannot run after a failed or skipped write. A sink error is returned, never swallowed or
//! retried here.

use crate::config::ConfigHash;
use crate::override_::{AcceptedOverride, IntentDigest, ReasonType};
use crate::verdict::{RuleId, RuleResult};
use bazaar_core::clock::Clock;
use bazaar_core::mode::ExecutionMode;
use chrono::DateTime;
use chrono_tz::Tz;

/// Observed and limit values of one overridden rule, taken from its `RuleResult`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditedRule {
    rule: RuleId,
    observed: String,
    limit: String,
}

impl AuditedRule {
    /// The overridden rule.
    #[must_use]
    pub const fn rule(&self) -> RuleId {
        self.rule
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
}

/// What is persisted for one accepted override. The intent digest serves as the intent id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRecord {
    rule_ids: Vec<RuleId>,
    rules: Vec<AuditedRule>,
    mode: ExecutionMode,
    reason: ReasonType,
    note: String,
    config_hash: ConfigHash,
    intent_digest: IntentDigest,
    at: DateTime<Tz>,
}

impl AuditRecord {
    /// Build the record from an accepted override; `at` is read from the injected clock.
    /// `results` are the verdict's rule results; each overridden rule needs one.
    ///
    /// # Errors
    /// Returns `AuditError` when an overridden rule has no result in `results`, because its
    /// observed and limit values could not be recorded.
    pub fn new(
        accepted: &AcceptedOverride,
        mode: ExecutionMode,
        results: &[RuleResult],
        clock: &dyn Clock,
    ) -> Result<Self, AuditError> {
        let mut rules = Vec::with_capacity(accepted.rule_ids().len());
        for &id in accepted.rule_ids() {
            let r = results
                .iter()
                .find(|r| r.rule() == id)
                .ok_or_else(|| AuditError::new(format!("no rule result for {id} to record")))?;
            rules.push(AuditedRule {
                rule: id,
                observed: r.observed().to_owned(),
                limit: r.limit().to_owned(),
            });
        }
        Ok(Self {
            rule_ids: accepted.rule_ids().to_vec(),
            rules,
            mode,
            reason: accepted.reason(),
            note: accepted.note().to_owned(),
            config_hash: accepted.config_hash().clone(),
            intent_digest: accepted.intent_digest().clone(),
            at: clock.now_ist(),
        })
    }

    /// Overridden rule ids.
    #[must_use]
    pub fn rule_ids(&self) -> &[RuleId] {
        &self.rule_ids
    }
    /// Observed and limit per overridden rule.
    #[must_use]
    pub fn rules(&self) -> &[AuditedRule] {
        &self.rules
    }
    /// Execution mode at override time.
    #[must_use]
    pub const fn mode(&self) -> ExecutionMode {
        self.mode
    }
    /// The reason type chosen.
    #[must_use]
    pub const fn reason(&self) -> ReasonType {
        self.reason
    }
    /// The free-text note.
    #[must_use]
    pub fn note(&self) -> &str {
        &self.note
    }
    /// Config hash the verdict was computed under.
    #[must_use]
    pub const fn config_hash(&self) -> &ConfigHash {
        &self.config_hash
    }
    /// Digest of the intent (serves as the intent id).
    #[must_use]
    pub const fn intent_digest(&self) -> &IntentDigest {
        &self.intent_digest
    }
    /// Time of the write, IST.
    #[must_use]
    pub const fn at(&self) -> DateTime<Tz> {
        self.at
    }
}

/// The sink could not durably store the record. The override fails; nothing is minted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditError(String);

impl AuditError {
    /// Wrap a failure. The detail is for the log only; `Display` never shows it.
    #[must_use]
    pub fn new(detail: impl Into<String>) -> Self {
        Self(detail.into())
    }
    /// Internal detail for the log; do not show it to callers.
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AuditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "The override could not be recorded, so it is refused.")
    }
}

impl std::error::Error for AuditError {}

/// Durable store for override records. `Ok(())` means the record is stored and acknowledged;
/// anything else must be an `Err`. The real sink arrives with F10.
pub trait AuditSink {
    /// Store `record`, returning only after it is durable.
    ///
    /// # Errors
    /// Returns `AuditError` when the sink did not store the record.
    fn write(&self, record: &AuditRecord) -> Result<(), AuditError>;
}

/// Proof that a record was written and acknowledged. Private constructor, not `Clone`.
#[derive(Debug, PartialEq, Eq)]
pub struct AuditReceipt {
    intent_digest: IntentDigest,
    config_hash: ConfigHash,
    at: DateTime<Tz>,
}

impl AuditReceipt {
    /// Digest of the intent whose override was recorded.
    #[must_use]
    pub const fn intent_digest(&self) -> &IntentDigest {
        &self.intent_digest
    }
    /// Config hash recorded.
    #[must_use]
    pub const fn config_hash(&self) -> &ConfigHash {
        &self.config_hash
    }
    /// Time recorded.
    #[must_use]
    pub const fn at(&self) -> DateTime<Tz> {
        self.at
    }
}

/// Write the override's record and return a receipt only when the sink acknowledged it.
/// A sink error is returned as is; the caller must treat it as a failed override.
/// Takes `accepted` by reference because the gate (slice 11b) still needs it to mint; single
/// use is enforced there (11b must test that a second write for the same override is refused).
///
/// # Errors
/// Returns `AuditError` when the record could not be built or the sink did not store it.
pub fn write_acknowledged(
    sink: &dyn AuditSink,
    accepted: &AcceptedOverride,
    mode: ExecutionMode,
    results: &[RuleResult],
    clock: &dyn Clock,
) -> Result<AuditReceipt, AuditError> {
    let record = AuditRecord::new(accepted, mode, results, clock)?;
    sink.write(&record)?;
    Ok(AuditReceipt {
        intent_digest: record.intent_digest,
        config_hash: record.config_hash,
        at: record.at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_text_hides_the_detail_and_refuses() {
        let e = AuditError::new("secret path /x");
        assert_eq!(
            e.to_string(),
            "The override could not be recorded, so it is refused."
        );
        assert_eq!(e.detail(), "secret path /x");
    }
}
