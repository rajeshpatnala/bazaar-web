//! Persisted, fail-closed kill switch.
//!
//! `KillSwitch::admits` is the single decision function: the pipeline and `GuardedExecutor`
//! both call it. `read` is the only place in `execution` that calls `KillSwitchRepo::is_engaged`;
//! the repo is a private field with no getter. An unreadable state counts as engaged, and the
//! reduce-only exemption does not apply then (state unknown: refuse everything).

use std::fmt;
use std::sync::Arc;

use bazaar_core::clock::Clock;
use bazaar_core::ports::{AuditDraft, KillSwitchRepo, RepoError};

use crate::audit::chain_hash;
use crate::executor::ExecError;

/// Why `KillSwitch::release` refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReleaseError {
    /// The typed phrase did not match; nothing was written.
    #[error("release phrase does not match")]
    WrongPhrase,
    /// The state or audit write failed; the switch stays as it was.
    #[error("storage error")]
    Storage,
}

/// Kill switch over a persistent repo.
pub struct KillSwitch<R: KillSwitchRepo> {
    repo: R,
    clock: Arc<dyn Clock>,
    release_phrase: String,
    allow_reduce_only: bool,
}

// The phrase is secret-like: never printed.
impl<R: KillSwitchRepo> fmt::Debug for KillSwitch<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KillSwitch")
            .field("allow_reduce_only", &self.allow_reduce_only)
            .finish_non_exhaustive()
    }
}

impl<R: KillSwitchRepo> KillSwitch<R> {
    /// `release_phrase` and `allow_reduce_only` come from the typed `[execution]` risk config.
    pub fn new(
        repo: R,
        clock: Arc<dyn Clock>,
        release_phrase: String,
        allow_reduce_only: bool,
    ) -> Self {
        Self {
            repo,
            clock,
            release_phrase,
            allow_reduce_only,
        }
    }

    async fn read(&self) -> Result<bool, RepoError> {
        self.repo.is_engaged().await
    }

    /// True when engaged or when the state cannot be read (fail closed).
    pub async fn is_engaged(&self) -> bool {
        self.read().await.unwrap_or(true)
    }

    /// `Ok` when an order may be sent. Engaged: only a reduce-only order, and only when the
    /// config allows it. Unreadable state refuses everything.
    ///
    /// # Errors
    /// `ExecError::KillSwitch` when engaged (and not exempt) or the state is unreadable.
    pub async fn admits(&self, reduce_only: bool) -> Result<(), ExecError> {
        match self.read().await {
            Ok(false) => Ok(()),
            Ok(true) if reduce_only && self.allow_reduce_only => Ok(()),
            Ok(true) | Err(_) => Err(ExecError::KillSwitch),
        }
    }

    fn draft(&self, actor: &str, kind: &str, reason: &str) -> AuditDraft {
        let payload = serde_json::json!({ "actor": actor, "reason": reason });
        AuditDraft {
            ts_ist_rfc3339: self.clock.now_ist().to_rfc3339(),
            actor: actor.to_owned(),
            kind: kind.to_owned(),
            payload: payload.to_string(),
        }
    }

    /// Engage; state and audit row are written together, and a failure changes nothing.
    ///
    /// # Errors
    /// `ExecError::Storage` when the write fails.
    pub async fn engage(&self, actor: &str, reason: &str) -> Result<(), ExecError> {
        let d = self.draft(actor, "kill_switch_engaged", reason);
        self.repo
            .set(true, d, chain_hash)
            .await
            .map_err(|_| ExecError::Storage)
    }

    /// Release with the exact typed phrase.
    ///
    /// # Errors
    /// `WrongPhrase` (nothing written) or `Storage` (still engaged).
    pub async fn release(&self, typed: &str, actor: &str) -> Result<(), ReleaseError> {
        if typed != self.release_phrase {
            return Err(ReleaseError::WrongPhrase);
        }
        let d = self.draft(actor, "kill_switch_released", "");
        self.repo
            .set(false, d, chain_hash)
            .await
            .map_err(|_| ReleaseError::Storage)
    }
}

#[cfg(test)]
#[allow(clippy::unused_async_trait_impl)]
pub(crate) mod fakes {
    use super::*;
    use crate::audit::GENESIS;
    use bazaar_core::ports::{AuditRepo, AuditRow, ChainFn};
    use chrono::{DateTime, TimeZone};
    use chrono_tz::{Asia::Kolkata, Tz};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    pub(crate) struct At;
    impl Clock for At {
        fn now_ist(&self) -> DateTime<Tz> {
            Kolkata.with_ymd_and_hms(2026, 11, 19, 10, 0, 0).unwrap()
        }
    }

    /// In-memory store that applies the `ChainFn` like the real one.
    #[derive(Default)]
    pub(crate) struct MemRepo {
        pub rows: Mutex<Vec<AuditRow>>,
        pub engaged: AtomicBool,
        pub fail_read: AtomicBool,
        pub fail_write: AtomicBool,
        pub bad_hash: AtomicBool,
        pub delay_ms: AtomicU64,
    }

    impl MemRepo {
        fn push(&self, d: AuditDraft, chain: ChainFn) -> Result<AuditRow, RepoError> {
            if self.fail_write.load(Ordering::SeqCst) {
                return Err(RepoError::Unavailable);
            }
            let mut rows = self.rows.lock().unwrap();
            let prev = rows.last().map_or(GENESIS, |r| r.hash);
            let seq = rows.len() as u64 + 1;
            let mut hash = chain(&prev, seq, &d);
            let bad = self.bad_hash.load(Ordering::SeqCst);
            if bad {
                hash[0] ^= 1;
            }
            let row = AuditRow {
                seq,
                draft: d,
                prev_hash: prev,
                hash,
            };
            rows.push(row.clone());
            if bad {
                Err(RepoError::Corrupt)
            } else {
                Ok(row)
            }
        }
    }

    impl AuditRepo for MemRepo {
        async fn append(&self, d: AuditDraft, chain: ChainFn) -> Result<AuditRow, RepoError> {
            let ms = self.delay_ms.load(Ordering::SeqCst);
            if ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
            }
            self.push(d, chain)
        }
        async fn read_all(&self) -> Result<Vec<AuditRow>, RepoError> {
            Ok(self.rows.lock().unwrap().clone())
        }
    }

    /// Shared handle so a "restarted" `KillSwitch` sees the same store.
    pub(crate) struct Shared(pub Arc<MemRepo>);
    impl KillSwitchRepo for Shared {
        async fn is_engaged(&self) -> Result<bool, RepoError> {
            if self.0.fail_read.load(Ordering::SeqCst) {
                return Err(RepoError::Unavailable);
            }
            Ok(self.0.engaged.load(Ordering::SeqCst))
        }
        async fn set(&self, e: bool, a: AuditDraft, c: ChainFn) -> Result<(), RepoError> {
            self.0.push(a, c)?; // write-ahead: a failed audit leaves the state unchanged
            self.0.engaged.store(e, Ordering::SeqCst);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fakes::*;
    use super::*;
    use crate::audit::verify_chain;
    use bazaar_core::ports::AuditRepo;
    use std::sync::atomic::Ordering::SeqCst;

    const PHRASE: &str = "RELEASE KILL SWITCH";

    fn ks(r: &Arc<MemRepo>, reduce: bool) -> KillSwitch<Shared> {
        KillSwitch::new(Shared(r.clone()), Arc::new(At), PHRASE.into(), reduce)
    }

    #[tokio::test]
    async fn unreadable_state_refuses_everything_including_reduce_only() {
        let r = Arc::new(MemRepo::default());
        r.fail_read.store(true, SeqCst);
        let k = ks(&r, true);
        assert!(k.is_engaged().await);
        assert_eq!(k.admits(false).await, Err(ExecError::KillSwitch));
        assert_eq!(k.admits(true).await, Err(ExecError::KillSwitch));
    }

    #[tokio::test]
    async fn not_engaged_admits_and_engaged_follows_reduce_only_config() {
        let r = Arc::new(MemRepo::default());
        let k = ks(&r, true);
        assert_eq!(k.admits(false).await, Ok(()));
        k.engage("user", "test").await.unwrap();
        assert_eq!(k.admits(false).await, Err(ExecError::KillSwitch));
        assert_eq!(k.admits(true).await, Ok(()));
        assert_eq!(ks(&r, false).admits(true).await, Err(ExecError::KillSwitch));
    }

    #[tokio::test]
    async fn state_survives_a_restart() {
        let r = Arc::new(MemRepo::default());
        ks(&r, true).engage("user", "x").await.unwrap();
        assert!(ks(&r, true).is_engaged().await);
    }

    #[tokio::test]
    async fn wrong_or_empty_phrase_writes_nothing() {
        let r = Arc::new(MemRepo::default());
        let k = ks(&r, true);
        k.engage("user", "x").await.unwrap();
        for typed in ["", "release kill switch", "RELEASE KILL SWITCH "] {
            assert_eq!(
                k.release(typed, "user").await,
                Err(ReleaseError::WrongPhrase)
            );
        }
        assert_eq!(r.rows.lock().unwrap().len(), 1);
        assert!(k.is_engaged().await);
    }

    #[tokio::test]
    async fn right_phrase_releases_with_one_audit_row_and_no_phrase_in_it() {
        let r = Arc::new(MemRepo::default());
        let k = ks(&r, true);
        k.engage("user", "x").await.unwrap();
        k.release(PHRASE, "user").await.unwrap();
        let rows = r.read_all().await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].draft.kind, "kill_switch_released");
        assert!(!rows[1].draft.payload.contains(PHRASE));
        assert!(!k.is_engaged().await);
        assert_eq!(verify_chain(&rows), Ok(()));
    }

    #[tokio::test]
    async fn failed_audit_append_blocks_release_and_engage() {
        let r = Arc::new(MemRepo::default());
        let k = ks(&r, true);
        k.engage("user", "x").await.unwrap();
        r.fail_write.store(true, SeqCst);
        assert_eq!(k.release(PHRASE, "user").await, Err(ReleaseError::Storage));
        assert!(k.is_engaged().await);
        r.engaged.store(false, SeqCst);
        assert_eq!(k.engage("user", "y").await, Err(ExecError::Storage));
        assert!(!r.engaged.load(SeqCst));
    }

    #[test]
    fn debug_never_shows_the_phrase() {
        let r = Arc::new(MemRepo::default());
        assert!(!format!("{:?}", ks(&r, true)).contains("RELEASE"));
    }
}
