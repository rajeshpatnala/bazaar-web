//! Override engine (AC7, minus the audit sink). One typed, single-use override can accept Soft
//! rows, and Paper rows in `paper` mode only. A Hard or No failure in the same verdict rejects
//! the whole override. An [`AcceptedOverride`] is the only source of `overridden`; the request
//! never carries it. The gate (slice 11b) writes the audit record before minting.

use crate::config::{ConfigHash, RuleCtx};
use crate::snapshot::PortfolioSnapshot;
use crate::types::{OrderIntent, Side};
use crate::verdict::{GateVerdict, OverrideLevel, RuleId, RuleResult};
use bazaar_core::clock::Clock;
use bazaar_core::mode::ExecutionMode;
use chrono::DateTime;
use chrono_tz::Tz;
use rust_decimal::Decimal;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;

/// Shortest accepted note, in characters (trimmed).
pub const MIN_NOTE_CHARS: usize = 30;
/// Fewest whitespace-separated words in a note (thirty repeated letters are not a reason).
pub const MIN_NOTE_WORDS: usize = 5;
/// Longest accepted note, in characters.
pub const MAX_NOTE_CHARS: usize = 1000;

/// SHA-256 of the intent's fields, lower-case hex. An override is bound to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntentDigest(String);

impl IntentDigest {
    /// Digest of every intent field, each length-prefixed so fields cannot run together.
    #[must_use]
    pub fn of(intent: &OrderIntent) -> Self {
        let opt =
            |v: Option<Decimal>| v.map_or_else(|| "-".to_owned(), |d| d.normalize().to_string());
        let parts = [
            intent.symbol.clone(),
            match intent.side {
                Side::Buy => "BUY",
                Side::Sell => "SELL",
            }
            .to_owned(),
            intent.qty.0.to_string(),
            intent.price.normalize().to_string(),
            opt(intent.stop),
            opt(intent.new_stop),
            intent.lot_size.to_string(),
            intent.tick_size.normalize().to_string(),
        ];
        let mut hasher = Sha256::new();
        for p in &parts {
            hasher.update(p.len().to_string().as_bytes());
            hasher.update(b":");
            hasher.update(p.as_bytes());
        }
        let mut hex = String::with_capacity(64);
        for byte in &hasher.finalize() {
            let _ = write!(hex, "{byte:02x}");
        }
        Self(hex)
    }

    /// The hex string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Why the user is overriding. `Other` needs the note like every type, and is never a default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasonType {
    NewInformation,
    PlanChanged,
    PaperExperiment,
    Other,
}

/// Opened by the crate when the confirm dialog is shown: it stamps `opened_at` from the injected
/// `Clock` (never from the client) and captures the failed rule ids, the intent digest and the
/// config hash of the verdict. Not `Clone`; consumed with the request.
#[derive(Debug)]
pub struct OverrideChallenge {
    rule_ids: Vec<RuleId>,
    intent_digest: IntentDigest,
    config_hash: ConfigHash,
    opened_at: DateTime<Tz>,
}

impl OverrideChallenge {
    /// Open a challenge for `verdict`, which must be bound to `intent` (the gate binds it).
    pub fn open(
        clock: &dyn Clock,
        intent: &OrderIntent,
        verdict: &GateVerdict,
    ) -> Result<Self, OverrideError> {
        let digest = IntentDigest::of(intent);
        if verdict.intent_digest() != Some(&digest) {
            return Err(OverrideError::IntentMismatch);
        }
        let mut rule_ids: Vec<RuleId> = verdict.failures().map(RuleResult::rule).collect();
        rule_ids.sort();
        Ok(Self {
            rule_ids,
            intent_digest: digest,
            config_hash: verdict.config_hash().clone(),
            opened_at: clock.now_ist(),
        })
    }
}

/// A user's override request: the challenge plus the typed reason and note. Not `Clone` and
/// consumed by [`evaluate`]: one request, one use. There is no field a caller could set to claim
/// the order is already overridden, and no caller-set time.
#[derive(Debug)]
pub struct OverrideRequest {
    pub challenge: OverrideChallenge,
    pub reason: ReasonType,
    pub note: String,
}

/// Why an override was refused. No variant leaves a half-accepted override.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverrideError {
    NothingToOverride,
    /// A Hard or No rule failed in the same verdict.
    NotOverridable(RuleId),
    /// A Paper row failed while the mode is not `paper`.
    PaperOnly(RuleId),
    /// Request ids differ from the failed rules (missing, extra or repeated).
    RuleMismatch,
    IntentMismatch,
    ConfigMismatch,
    NoteTooShort {
        have: usize,
        need: usize,
    },
    NoteTooLong {
        have: usize,
        max: usize,
    },
    NoteTooFewWords {
        have: usize,
        need: usize,
    },
    NoteControlChars,
    /// The rule failed because an input is missing or unknown; fix the input, do not override.
    InputMissing(RuleId),
    /// `PaperExperiment` is valid only in paper mode.
    ReasonNotAllowed,
    AutoMode,
    CountUnknown,
    DayCapReached {
        used: u32,
        max: u32,
    },
    WeekCapReached {
        used: u32,
        max: u32,
    },
    /// The confirm delay has not passed (or the clock is before `opened_at`).
    TooSoon {
        wait_secs: u32,
    },
}

impl std::fmt::Display for OverrideError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NothingToOverride => {
                write!(f, "No rule failed, so there is nothing to override.")
            }
            Self::NotOverridable(r) => {
                write!(
                    f,
                    "{r} cannot be overridden, so the whole override is refused."
                )
            }
            Self::PaperOnly(r) => write!(f, "{r} can be overridden in paper mode only."),
            Self::RuleMismatch => write!(f, "The override must name exactly the failed rules."),
            Self::IntentMismatch => write!(f, "The override was made for a different order."),
            Self::ConfigMismatch => {
                write!(f, "The risk config changed since this override was made.")
            }
            Self::NoteTooShort { have, need } => {
                write!(
                    f,
                    "The note has {have} characters; at least {need} are needed."
                )
            }
            Self::NoteTooLong { have, max } => {
                write!(
                    f,
                    "The note has {have} characters; at most {max} are allowed."
                )
            }
            Self::NoteTooFewWords { have, need } => {
                write!(f, "The note has {have} words; at least {need} are needed.")
            }
            Self::NoteControlChars => write!(f, "The note must not contain control characters."),
            Self::InputMissing(r) => write!(
                f,
                "{r} failed because an input is missing, so it cannot be overridden. Fix the input."
            ),
            Self::ReasonNotAllowed => write!(
                f,
                "The paper experiment reason is valid in paper mode only."
            ),
            Self::AutoMode => write!(f, "Overrides are not allowed in auto mode."),
            Self::CountUnknown => {
                write!(f, "Override counts are unknown, so no override is allowed.")
            }
            Self::DayCapReached { used, max } => write!(f, "{used} of {max} overrides used today."),
            Self::WeekCapReached { used, max } => {
                write!(f, "{used} of {max} overrides used this week.")
            }
            Self::TooSoon { wait_secs } => {
                write!(f, "Wait {wait_secs} more seconds before confirming.")
            }
        }
    }
}

impl std::error::Error for OverrideError {}

/// A verified override. Fields are private and only [`evaluate`] builds it; it is not `Clone`,
/// so the gate consumes it once. Its existence is the only way `overridden` becomes true.
#[derive(Debug, PartialEq, Eq)]
pub struct AcceptedOverride {
    rule_ids: Vec<RuleId>,
    reason: ReasonType,
    note: String,
    intent_digest: IntentDigest,
    config_hash: ConfigHash,
    accepted_at: DateTime<Tz>,
    size_factor: Decimal,
}

impl AcceptedOverride {
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
    pub const fn intent_digest(&self) -> &IntentDigest {
        &self.intent_digest
    }
    #[must_use]
    pub const fn config_hash(&self) -> &ConfigHash {
        &self.config_hash
    }
    #[must_use]
    pub const fn accepted_at(&self) -> DateTime<Tz> {
        self.accepted_at
    }
    /// Fraction of the sized quantity an overridden order may use (`override.size_factor`).
    #[must_use]
    pub const fn size_factor(&self) -> Decimal {
        self.size_factor
    }
}

/// Note rules: length bounds, no control characters, enough distinct words.
fn check_note(note: &str) -> Result<(), OverrideError> {
    let have = note.chars().count();
    if have < MIN_NOTE_CHARS {
        return Err(OverrideError::NoteTooShort {
            have,
            need: MIN_NOTE_CHARS,
        });
    }
    if have > MAX_NOTE_CHARS {
        return Err(OverrideError::NoteTooLong {
            have,
            max: MAX_NOTE_CHARS,
        });
    }
    if note.chars().any(char::is_control) {
        return Err(OverrideError::NoteControlChars);
    }
    let words = note.split_whitespace().count();
    if words < MIN_NOTE_WORDS {
        return Err(OverrideError::NoteTooFewWords {
            have: words,
            need: MIN_NOTE_WORDS,
        });
    }
    Ok(())
}

/// Accept or refuse `req`. The one time source is `clock.now_ist()` (also stamps `accepted_at`);
/// `ctx.now` is not read, so the delay and the accepted time can never disagree. Single use is by
/// move; the day and week counts are read from `snapshot`, so the gate (11b) must write and
/// acknowledge the audit record and bump the journal count before the next call.
///
/// Accept or refuse `req` for `intent` against the same-run `verdict`. Every refusal is a typed
/// error; nothing is accepted partially.
pub fn evaluate(
    req: OverrideRequest,
    intent: &OrderIntent,
    verdict: &GateVerdict,
    snapshot: &PortfolioSnapshot,
    ctx: &RuleCtx,
    clock: &dyn Clock,
) -> Result<AcceptedOverride, OverrideError> {
    if ctx.mode == ExecutionMode::Auto {
        return Err(OverrideError::AutoMode);
    }
    if req.reason == ReasonType::PaperExperiment && ctx.mode != ExecutionMode::Paper {
        return Err(OverrideError::ReasonNotAllowed);
    }
    let OverrideRequest {
        challenge,
        reason,
        note,
    } = req;
    // The verdict must have been computed for this very intent.
    let digest = IntentDigest::of(intent);
    if verdict.intent_digest() != Some(&digest) || challenge.intent_digest != digest {
        return Err(OverrideError::IntentMismatch);
    }
    let mut failed: Vec<RuleId> = Vec::new();
    for r in verdict.failures() {
        if r.is_input_missing() {
            return Err(OverrideError::InputMissing(r.rule()));
        }
        match r.level() {
            OverrideLevel::HardCeiling | OverrideLevel::No => {
                return Err(OverrideError::NotOverridable(r.rule()));
            }
            OverrideLevel::Paper if ctx.mode != ExecutionMode::Paper => {
                return Err(OverrideError::PaperOnly(r.rule()));
            }
            OverrideLevel::Paper | OverrideLevel::Soft => failed.push(r.rule()),
        }
    }
    if failed.is_empty() {
        return Err(OverrideError::NothingToOverride);
    }
    failed.sort();
    if challenge.rule_ids != failed {
        return Err(OverrideError::RuleMismatch);
    }
    if &challenge.config_hash != verdict.config_hash() || challenge.config_hash != ctx.config_hash {
        return Err(OverrideError::ConfigMismatch);
    }
    let note = note.trim().to_owned();
    check_note(&note)?;
    let (Some(&day), Some(&week)) = (
        snapshot.overrides_today.known(),
        snapshot.overrides_week.known(),
    ) else {
        return Err(OverrideError::CountUnknown);
    };
    let (max_day, max_week) = (
        ctx.limits.override_max_per_day,
        ctx.limits.override_max_per_week,
    );
    if day >= max_day {
        return Err(OverrideError::DayCapReached {
            used: day,
            max: max_day,
        });
    }
    if week >= max_week {
        return Err(OverrideError::WeekCapReached {
            used: week,
            max: max_week,
        });
    }
    let now = clock.now_ist();
    let waited = now.signed_duration_since(challenge.opened_at).num_seconds();
    let need = i64::from(ctx.limits.override_confirm_delay_secs);
    if waited < need {
        let wait = need
            .saturating_sub(waited.max(0))
            .clamp(0, i64::from(u32::MAX));
        return Err(OverrideError::TooSoon {
            wait_secs: u32::try_from(wait).unwrap_or(u32::MAX),
        });
    }
    Ok(AcceptedOverride {
        rule_ids: failed,
        reason,
        note,
        intent_digest: digest,
        config_hash: challenge.config_hash,
        accepted_at: now,
        size_factor: ctx.limits.override_size_factor,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_text_names_the_numbers() {
        let e = OverrideError::DayCapReached { used: 1, max: 1 };
        assert_eq!(e.to_string(), "1 of 1 overrides used today.");
        assert_eq!(MIN_NOTE_CHARS, 30);
    }
}
