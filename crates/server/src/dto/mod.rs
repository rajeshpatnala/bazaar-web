//! Response DTOs: wire shapes kept apart from domain types (slice F1/server-app).

use bazaar_core::mode::ExecutionMode;
use serde::Serialize;
use utoipa::ToSchema;

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
}
