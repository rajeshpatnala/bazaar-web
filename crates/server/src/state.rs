//! `AppState` and its builder (slice F1/server-app).

use std::{path::PathBuf, sync::Arc};

use axum::extract::FromRef;
use bazaar_core::{clock::Clock, mode::ExecutionMode};

use crate::security::Security;

/// Shared handler state (cheap to clone). Ports arrive in later features; tests pass fakes.
#[derive(Clone)]
pub struct AppState {
    /// Launch token, sessions and guard settings.
    pub security: Security,
    /// The only time source handlers may use.
    pub clock: Arc<dyn Clock>,
    /// Mode from `config/risk.toml`; never changed over HTTP.
    pub mode: ExecutionMode,
    /// True only in a build with the `live-trading` feature.
    pub live_enabled: bool,
    /// Built web UI directory (`web/dist`); may be missing.
    pub web_dist: PathBuf,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("mode", &self.mode)
            .field("live_enabled", &self.live_enabled)
            .finish_non_exhaustive()
    }
}

impl AppState {
    /// Build state; `live_enabled` follows the compile-time feature, never a runtime setting.
    #[must_use]
    pub fn new(
        security: Security,
        clock: Arc<dyn Clock>,
        mode: ExecutionMode,
        web_dist: PathBuf,
    ) -> Self {
        Self {
            security,
            clock,
            mode,
            live_enabled: cfg!(feature = "live-trading"),
            web_dist,
        }
    }
}

impl FromRef<AppState> for Security {
    fn from_ref(s: &AppState) -> Self {
        s.security.clone()
    }
}
