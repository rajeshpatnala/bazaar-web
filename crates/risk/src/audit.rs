//! Audit record for an accepted override and the acknowledged-write API.
//!
//! The record is written through an [`AuditSink`] and must be acknowledged before the gate may
//! mint a `GatedIntent` (the ordering is enforced in the gate, slice 11b). [`AuditReceipt`] has a
//! private constructor: the only way to get one is a sink that returned `Ok`, so a mint step that
//! demands a receipt cannot run after a failed or skipped write. A sink error is returned, never
//! swallowed or retried here.

use crate::config::ConfigHash;
use crate::override_::{AcceptedOverride, IntentDigest, ReasonType};
use crate::verdict::RuleId;
use bazaar_core::clock::Clock;
use chrono::DateTime;
use chrono_tz::Tz;

/// What is persisted for one accepted override.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRecord {
    rule_ids: Vec<RuleId>,
    reason: ReasonType,
    note: String,
    config_hash: ConfigHash,
    intent_digest: IntentDigest,
    at: DateTime<Tz>,
}

impl AuditRecord {
    /// Build the record from an accepted override; `at` is read from the injected clock.
    #[must_use]
    pub fn new(accepted: &AcceptedOverride, clock: &dyn Clock) -> Self {
        Self {
            rule_ids: accepted.rule_ids().to_vec(),
            reason: accepted.reason(),
            note: accepted.note().to_owned(),
            config_hash: accepted.config_hash().clone(),
            intent_digest: accepted.intent_digest().clone(),
            at: clock.now_ist(),
        }
    }

    #[must_use]
    pub fn rule_ids(&self) -> &[RuleId] {
        &self.rule_ids
    }
    #[must_use]
    pub const fn reason(&self) -> ReasonType {
        self.reason
    }
    #[must_use]
    pub fn note(&self) -> &str {
        &self.note
    }
    #[must_use]
    pub const fn config_hash(&self) -> &ConfigHash {
        &self.config_hash
    }
    #[must_use]
    pub const fn intent_digest(&self) -> &IntentDigest {
        &self.intent_digest
    }
    #[must_use]
    pub const fn at(&self) -> DateTime<Tz> {
        self.at
    }
}

/// The sink could not durably store the record. The override fails; nothing is minted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditError(pub String);

impl std::fmt::Display for AuditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "The override could not be recorded ({}), so it is refused.",
            self.0
        )
    }
}

impl std::error::Error for AuditError {}

/// Durable store for override records. `Ok(())` means the record is stored and acknowledged;
/// anything else must be an `Err`. The real sink arrives with F10.
pub trait AuditSink {
    /// Store `record`, returning only after it is durable.
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
    #[must_use]
    pub const fn intent_digest(&self) -> &IntentDigest {
        &self.intent_digest
    }
    #[must_use]
    pub const fn config_hash(&self) -> &ConfigHash {
        &self.config_hash
    }
    #[must_use]
    pub const fn at(&self) -> DateTime<Tz> {
        self.at
    }
}

/// Write the override's record and return a receipt only when the sink acknowledged it.
/// A sink error is returned as is; the caller must treat it as a failed override.
pub fn write_acknowledged(
    sink: &dyn AuditSink,
    accepted: &AcceptedOverride,
    clock: &dyn Clock,
) -> Result<AuditReceipt, AuditError> {
    let record = AuditRecord::new(accepted, clock);
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
    fn error_text_names_the_cause_and_refuses() {
        let e = AuditError("x".into());
        assert_eq!(
            e.to_string(),
            "The override could not be recorded (x), so it is refused."
        );
    }
}
