//! Response DTOs: wire shapes kept apart from domain types (slice F1/server-app).

use bazaar_core::{mode::ExecutionMode, ports::ApprovalDecision};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::state::{Alarm, KillSwitchReading, PendingApproval};

/// Execution mode on the wire (`snake_case`, same spelling as `risk.toml`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModeDto {
    /// Virtual cash; every order still needs approval.
    Paper,
    /// Live orders, each approved by the user.
    SemiAuto,
    /// Live orders under an auto policy.
    Auto,
}

impl From<ExecutionMode> for ModeDto {
    fn from(m: ExecutionMode) -> Self {
        match m {
            ExecutionMode::Paper => Self::Paper,
            ExecutionMode::SemiAuto => Self::SemiAuto,
            ExecutionMode::Auto => Self::Auto,
        }
    }
}

/// `GET /api/v1/auth/me`: proof of a live session plus what the UI needs for non-GET calls.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AuthMeDto {
    /// Echo this in `X-CSRF-Token` on every non-GET request.
    pub csrf_token: String,
    /// Execution mode from `risk.toml`.
    pub mode: ModeDto,
    /// True only in a build with the `live-trading` feature.
    pub live_enabled: bool,
}

/// `GET /api/v1/status` (F1 subset: no feed, broker or calendar fields yet).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct StatusDto {
    /// Current time in IST, RFC 3339.
    pub server_time: String,
    /// Crate version.
    pub version: String,
    /// Execution mode from `risk.toml`.
    pub mode: ModeDto,
    /// True only in a build with the `live-trading` feature.
    pub live_enabled: bool,
    /// Alarms currently raised by background drivers (empty when all is well).
    pub alarms: Vec<AlarmDto>,
}

/// One raised alarm.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct AlarmDto {
    /// Stable machine key, for example `square_off_blocked`.
    pub code: String,
    /// User-safe text (control characters escaped).
    pub message: String,
    /// When it was first raised, RFC 3339 IST.
    pub since: String,
}

impl From<Alarm> for AlarmDto {
    fn from(a: Alarm) -> Self {
        let Alarm {
            code,
            message,
            since,
        } = a;
        Self {
            code: escape_control(&code),
            message: escape_control(&message),
            since: escape_control(&since),
        }
    }
}

/// The user's verdict on a pending approval. The actor is never in the body: the server takes it
/// from the session (SEC-132-1), so an `actor` field is rejected as unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DecisionRequestDto {
    /// Approve or decline.
    pub decision: DecisionDto,
}

/// Verdict wire values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DecisionDto {
    /// Approve (TTL and the gate are re-checked with a fresh quote before any send).
    Approve,
    /// Decline.
    Decline,
}

impl From<DecisionDto> for ApprovalDecision {
    fn from(d: DecisionDto) -> Self {
        match d {
            DecisionDto::Approve => Self::Approve,
            DecisionDto::Decline => Self::Decline,
        }
    }
}

/// One order waiting for the user's decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct ApprovalDto {
    /// Intent id (32 lowercase hex digits).
    pub id: String,
    /// Symbol (control characters escaped).
    pub symbol: String,
    /// `buy` or `sell`.
    pub side: String,
    /// Quantity.
    pub qty: u32,
    /// Proposed at, RFC 3339 IST.
    pub created_at: String,
    /// Approve is refused after this instant (RFC 3339 IST).
    pub expires_at: String,
}

impl From<PendingApproval> for ApprovalDto {
    fn from(p: PendingApproval) -> Self {
        let PendingApproval {
            id,
            symbol,
            side,
            qty,
            created_at,
            expires_at,
        } = p;
        Self {
            id: format!("{id:032x}"),
            symbol: escape_control(&symbol),
            side: side.to_owned(),
            qty,
            created_at,
            expires_at,
        }
    }
}

/// `GET /api/v1/approvals`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct ApprovalListDto {
    /// Pending orders, oldest first.
    pub approvals: Vec<ApprovalDto>,
}

/// How a decision ended. Only `placed` and `placed_unrecorded` mean an order was sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeKind {
    /// Still waiting.
    Queued,
    /// The user declined.
    Declined,
    /// Past the approval TTL; nothing sent.
    Expired,
    /// The re-gate (fresh quote) refused it; nothing sent.
    Blocked,
    /// The quote moved past the drift limit; nothing sent.
    QuoteDrifted,
    /// Kill switch, breaker or rate limit refused the send.
    GuardBlocked,
    /// Sent and recorded.
    Placed,
    /// Sent, but the fill could not be recorded.
    PlacedUnrecorded,
    /// The executor refused or failed.
    ExecFailed,
    /// Storage failed; see the audit log.
    Error,
}

/// Reply of `POST /api/v1/approvals/{id}/decision`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct DecisionOutcomeDto {
    /// How it ended.
    pub outcome: OutcomeKind,
    /// Gate reason or error category (control characters escaped); null when there is none.
    pub detail: Option<String>,
}

/// `GET /api/v1/kill-switch`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct KillSwitchDto {
    /// True when engaged, or when the state could not be read (fail closed).
    pub engaged: bool,
    /// False when the stored state could not be read.
    pub readable: bool,
    /// Last engage reason with control characters escaped (SEC-130-2); `null` if none.
    pub reason: Option<String>,
}

impl From<KillSwitchReading> for KillSwitchDto {
    fn from(r: KillSwitchReading) -> Self {
        let KillSwitchReading {
            engaged,
            reason,
            readable,
        } = r;
        Self {
            // Unreadable counts as engaged, whatever the port said (fail closed, RISK-137-2).
            engaged: engaged || !readable,
            readable,
            reason: reason.as_deref().map(escape_control),
        }
    }
}

/// Unicode format characters (category Cf: soft hyphen, zero-width, bidi marks and overrides,
/// word joiner, invisible operators, BOM, tags) plus the line and paragraph separators. They can
/// reorder or hide text, so they are escaped too (SEC-137-1, RISK-137-4).
fn is_format_or_separator(c: char) -> bool {
    matches!(u32::from(c),
        0x00AD | 0x0600..=0x0605 | 0x061C | 0x06DD | 0x070F | 0x180E
        | 0x200B..=0x200F | 0x2028..=0x202E | 0x2060..=0x206F | 0xFEFF
        | 0xFFF9..=0xFFFB | 0x1_D173..=0x1_D17A | 0xE_0001 | 0xE_0020..=0xE_007F)
}

/// Escape control and format characters (ESC, BEL, newlines, NUL, C1, bidi overrides, U+2028/9 ...)
/// as `\xNN` / `\u{N}` so stored text cannot rewrite a terminal or forge a log line. A literal
/// backslash is kept as is.
#[must_use]
pub fn escape_control(s: &str) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_control() || is_format_or_separator(c) {
            let n = u32::from(c);
            let _ = if n <= 0xff {
                write!(out, "\\x{n:02x}")
            } else {
                write!(out, "\\u{{{n:x}}}")
            };
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn mode_maps_and_serialises_snake_case() {
        assert_eq!(ModeDto::from(ExecutionMode::Paper), ModeDto::Paper);
        let v = serde_json::to_value(ModeDto::from(ExecutionMode::SemiAuto)).unwrap();
        assert_eq!(v, "semi_auto");
        assert_eq!(serde_json::to_value(ModeDto::Auto).unwrap(), "auto");
    }

    #[test]
    fn escape_control_escapes_ansi_bel_newline_and_keeps_text() {
        assert_eq!(escape_control("\x1b[31mx\x07"), "\\x1b[31mx\\x07");
        assert_eq!(escape_control("a\nb\r\0"), "a\\x0ab\\x0d\\x00");
        assert_eq!(escape_control("\u{85}\u{2028}x"), "\\x85\\u{2028}x");
        assert_eq!(
            escape_control("a\u{202e}b\u{2066}\u{200b}\u{feff}\u{2029}"),
            "a\\u{202e}b\\u{2066}\\u{200b}\\u{feff}\\u{2029}"
        );
        assert_eq!(escape_control("हिन्दी"), "हिन्दी");
        assert_eq!(escape_control("plain ₹ text"), "plain ₹ text");
    }

    #[test]
    fn kill_switch_dto_escapes_reason() {
        let d = KillSwitchDto::from(KillSwitchReading {
            engaged: true,
            reason: Some("\x1b[31mx\x07".into()),
            readable: true,
        });
        assert_eq!(d.reason.as_deref(), Some("\\x1b[31mx\\x07"));
        assert!(!serde_json::to_string(&d).unwrap().contains('\x1b'));
    }

    #[test]
    fn unreadable_kill_switch_is_engaged_even_if_port_said_not() {
        let d = KillSwitchDto::from(KillSwitchReading {
            engaged: false,
            reason: None,
            readable: false,
        });
        assert!(d.engaged && !d.readable);
    }

    #[test]
    fn alarm_dto_escapes_code_message_and_since() {
        let d = AlarmDto::from(Alarm {
            code: "c\x1b".into(),
            message: "m\x1b".into(),
            since: "t\x07".into(),
        });
        assert_eq!(
            (d.code.as_str(), d.message.as_str(), d.since.as_str()),
            ("c\\x1b", "m\\x1b", "t\\x07")
        );
    }

    #[test]
    fn decision_request_rejects_actor_and_unknown_fields() {
        let ok: DecisionRequestDto = serde_json::from_str(r#"{"decision":"approve"}"#).unwrap();
        assert_eq!(
            ApprovalDecision::from(ok.decision),
            ApprovalDecision::Approve
        );
        assert!(
            serde_json::from_str::<DecisionRequestDto>(
                r#"{"decision":"approve","actor":"mallory"}"#
            )
            .is_err()
        );
        assert!(serde_json::from_str::<DecisionRequestDto>(r#"{"decision":"place"}"#).is_err());
        assert!(serde_json::from_str::<DecisionRequestDto>("{}").is_err());
    }
}
