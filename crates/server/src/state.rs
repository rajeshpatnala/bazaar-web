//! `AppState` and its builder (slice F1/server-app; BZ-511 S0 adds pipeline, kill-switch read
//! port and alarm board).

use std::{
    collections::{BTreeMap, VecDeque},
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::{Arc, Mutex, PoisonError},
};

use axum::extract::FromRef;
use bazaar_core::{clock::Clock, mode::ExecutionMode};
use bazaar_execution::pipeline::PipelineHandle;

use crate::{error::ApiError, security::Security};

/// What the kill-switch read port reports. An unreadable state is reported as engaged (fail closed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KillSwitchReading {
    /// True when engaged, or when the state could not be read.
    pub engaged: bool,
    /// Last engage reason, raw text (escape on output with `dto::escape_control`).
    pub reason: Option<String>,
    /// False when the state could not be read (then `engaged` is true).
    pub readable: bool,
    /// True when the pipeline's in-process latch is set (shown apart from the stored state).
    pub latched: bool,
}

/// Boxed future so the port is object safe without an extra crate.
pub type ReadFuture<'a> = Pin<Box<dyn Future<Output = KillSwitchReading> + Send + 'a>>;

/// Read-only view of the persisted kill switch. Writes go only through the pipeline.
pub trait KillSwitchRead: Send + Sync {
    /// Current state; never an error (see [`KillSwitchReading`]).
    fn read(&self) -> ReadFuture<'_>;
}

/// One order waiting for the user's decision (read model; wire shape is `dto::ApprovalDto`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingApproval {
    /// Intent id.
    pub id: u128,
    /// Symbol, raw text.
    pub symbol: String,
    /// `buy` or `sell`.
    pub side: &'static str,
    /// Quantity.
    pub qty: u32,
    /// When it was proposed, RFC 3339 IST.
    pub created_at: String,
    /// Last moment an approve is honoured (created + approval TTL), RFC 3339 IST.
    pub expires_at: String,
}

/// `None` means the store could not be read (reported as 503, never as an empty list).
pub type PendingFuture<'a> =
    Pin<Box<dyn Future<Output = Option<Vec<PendingApproval>>> + Send + 'a>>;

/// Read-only view of orders awaiting approval. Decisions go only through the pipeline.
pub trait PendingRead: Send + Sync {
    /// Pending orders, oldest first.
    fn list(&self) -> PendingFuture<'_>;
}

/// Cap on remembered `Idempotency-Key` replies; the oldest is dropped first.
pub const IDEMPOTENCY_CAP: usize = 256;

type Remembered = (String, bool, crate::dto::DecisionOutcomeDto);

/// First reply per (actor, order, key), so a retried POST returns the first outcome. In memory
/// only: lost on restart, and a key evicted at the cap replays as 409 (BZ-512 persists keys).
#[derive(Debug, Clone, Default)]
pub struct IdempotencyCache(Arc<Mutex<VecDeque<Remembered>>>);

impl IdempotencyCache {
    /// The remembered reply for `key` with the decision that produced it (`true` = approve).
    #[must_use]
    pub fn get(&self, key: &str) -> Option<(bool, crate::dto::DecisionOutcomeDto)> {
        let q = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        q.iter().find(|e| e.0 == key).map(|e| (e.1, e.2.clone()))
    }

    /// Remember the first reply for `key`; a later `put` of the same key is ignored.
    pub fn put(&self, key: String, approve: bool, reply: crate::dto::DecisionOutcomeDto) {
        let mut q = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if q.iter().any(|e| e.0 == key) {
            return;
        }
        if q.len() >= IDEMPOTENCY_CAP {
            q.pop_front();
        }
        q.push_back((key, approve, reply));
    }
}

/// One raised alarm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alarm {
    /// Stable machine key, for example `square_off_blocked`.
    pub code: String,
    /// User-safe text.
    pub message: String,
    /// When it was first raised, RFC 3339 IST.
    pub since: String,
}

/// Byte caps for alarm fields (SEC-137-2).
pub const MAX_CODE: usize = 64;
/// Byte cap for an alarm message.
pub const MAX_MESSAGE: usize = 512;
/// Byte cap for an alarm `since` stamp.
pub const MAX_SINCE: usize = 64;

fn cap(s: &str, max: usize) -> String {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_owned()
}

/// Alarms raised by background drivers and shown on `GET /status`. Keyed by code: raising an
/// alarm that is already up keeps its first `since`.
#[derive(Debug, Clone, Default)]
pub struct AlarmBoard(Arc<Mutex<BTreeMap<String, Alarm>>>);

impl AlarmBoard {
    /// Empty board.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn map(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Alarm>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Raise (or keep) the alarm `code`. Fields are cut to [`MAX_CODE`] / [`MAX_MESSAGE`] /
    /// [`MAX_SINCE`] bytes (on a char boundary) so a caller cannot grow the board without bound.
    pub fn raise(&self, code: &str, message: &str, now_rfc3339: &str) {
        let code = cap(code, MAX_CODE);
        self.map().entry(code.clone()).or_insert_with(|| Alarm {
            code,
            message: cap(message, MAX_MESSAGE),
            since: cap(now_rfc3339, MAX_SINCE),
        });
    }

    /// Clear the alarm `code`; true if it was up.
    pub fn clear(&self, code: &str) -> bool {
        self.map().remove(code).is_some()
    }

    /// All alarms, ordered by code.
    #[must_use]
    pub fn snapshot(&self) -> Vec<Alarm> {
        self.map().values().cloned().collect()
    }
}

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
    /// The pipeline front door; `None` until the runtime wires it (routes then answer 503).
    /// Private: only `with_pipeline` sets it, together with `kill_switch` (RISK-137-3).
    pipeline: Option<PipelineHandle>,
    /// Read-only kill-switch view; `None` until wired; set only with `pipeline`.
    kill_switch: Option<Arc<dyn KillSwitchRead>>,
    /// Pending-approval read port; `None` until wired (the list route answers 503).
    pending: Option<Arc<dyn PendingRead>>,
    /// Replies of the decision route by `Idempotency-Key`.
    idempotency: IdempotencyCache,
    /// Alarms for `GET /status`.
    pub alarms: AlarmBoard,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("mode", &self.mode)
            .field("live_enabled", &self.live_enabled)
            .field("pipeline", &self.pipeline.is_some())
            .finish_non_exhaustive()
    }
}

impl AppState {
    /// Build state; `live_enabled` follows the compile-time feature, never a runtime setting.
    /// The pipeline and kill-switch port start unwired (see [`Self::with_pipeline`]).
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
            pipeline: None,
            kill_switch: None,
            pending: None,
            idempotency: IdempotencyCache::default(),
            alarms: AlarmBoard::new(),
        }
    }

    /// Replies of the decision route by `Idempotency-Key`.
    #[must_use]
    pub fn idempotency(&self) -> &IdempotencyCache {
        &self.idempotency
    }

    /// Wire the running pipeline and the kill-switch read port.
    #[must_use]
    pub fn with_pipeline(
        mut self,
        pipeline: PipelineHandle,
        kill_switch: Arc<dyn KillSwitchRead>,
    ) -> Self {
        self.pipeline = Some(pipeline);
        self.kill_switch = Some(kill_switch);
        self
    }

    /// Wire the pending-approval read port.
    #[must_use]
    pub fn with_pending(mut self, pending: Arc<dyn PendingRead>) -> Self {
        self.pending = Some(pending);
        self
    }

    /// The pending-approval read port, or 503 `pipeline_closed` when it is not wired.
    pub fn pending(&self) -> Result<&Arc<dyn PendingRead>, ApiError> {
        self.pending.as_ref().ok_or(ApiError::PipelineClosed)
    }

    /// The pipeline, or 503 `pipeline_closed` when it is not wired.
    pub fn pipeline(&self) -> Result<&PipelineHandle, ApiError> {
        self.pipeline.as_ref().ok_or(ApiError::PipelineClosed)
    }

    /// The kill-switch read port, or 503 `pipeline_closed` when it is not wired. Acceptable for
    /// S0; the kill-switch routes (S3/S4) document and test the unwired answer.
    pub fn kill_switch(&self) -> Result<&Arc<dyn KillSwitchRead>, ApiError> {
        self.kill_switch.as_ref().ok_or(ApiError::PipelineClosed)
    }
}

impl FromRef<AppState> for Security {
    fn from_ref(s: &AppState) -> Self {
        s.security.clone()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(crate) mod tests_support {
    use bazaar_core::testutil::FakeClock;
    use chrono::TimeZone;

    use super::*;

    pub(crate) fn state() -> AppState {
        let t = chrono_tz::Asia::Kolkata
            .with_ymd_and_hms(2026, 1, 5, 10, 0, 0)
            .unwrap();
        AppState::new(
            Security::new(8787, false).unwrap(),
            Arc::new(FakeClock::at(t)),
            ExecutionMode::Paper,
            PathBuf::new(),
        )
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn raise_caps_field_lengths_on_char_boundaries() {
        let b = AlarmBoard::new();
        b.raise(&"c".repeat(100), &"é".repeat(600), &"s".repeat(100));
        let a = &b.snapshot()[0];
        assert_eq!(a.code.len(), MAX_CODE);
        assert!(a.message.len() <= MAX_MESSAGE && a.message.chars().all(|c| c == 'é'));
        assert_eq!(a.since.len(), MAX_SINCE);
    }

    #[test]
    fn alarm_board_raises_once_keeps_first_since_and_clears() {
        let b = AlarmBoard::new();
        b.raise("square_off_blocked", "blocked", "t1");
        b.raise("square_off_blocked", "blocked again", "t2");
        let s = b.snapshot();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].since, "t1");
        assert!(b.clear("square_off_blocked"));
        assert!(!b.clear("square_off_blocked"));
        assert_eq!(b.snapshot(), Vec::new());
    }

    #[test]
    fn unwired_pipeline_and_kill_switch_are_503() {
        let st = crate::state::tests_support::state();
        assert_eq!(st.pipeline().unwrap_err(), ApiError::PipelineClosed);
        assert!(st.kill_switch().is_err());
    }
}
