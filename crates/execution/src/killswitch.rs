//! Persisted, fail-closed kill switch.
//!
//! `KillSwitch::admits` is the single decision function: the pipeline and `GuardedExecutor`
//! both call it. `read` is the only place in `execution` that calls `KillSwitchRepo::is_engaged`;
//! the repo is a private field with no getter. An unreadable state counts as engaged, and the
//! reduce-only exemption does not apply then (state unknown: refuse everything).

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bazaar_core::clock::Clock;
use bazaar_core::ports::{AuditDraft, KillSwitchRepo, RepoError};

use crate::audit::chain_hash;
use crate::executor::ExecError;

/// The store refuses an empty reason; an engage without one is recorded as "unspecified".
/// The reason is trimmed before it is stored (CR-130-4); the length cap is in bytes at the
/// pipeline edge and again in the store.
fn reason_or_default(reason: &str) -> &str {
    let r = reason.trim();
    if r.is_empty() { "unspecified" } else { r }
}

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

/// Shortest release phrase accepted; mirrors the config validation in `core::config::risk`.
const MIN_PHRASE_LEN: usize = 8;

/// The configured release phrase is too short to be a real confirmation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("kill switch release phrase is too short")]
pub struct PhraseTooShort;

/// Kill switch over a persistent repo. An in-process latch is set by every `engage` call before
/// the repo write and cleared only by a successful `release`, so a failed engage write still
/// refuses orders. The latch is lost on restart (the persisted state is what survives).
pub struct KillSwitch<R: KillSwitchRepo> {
    repo: R,
    clock: Arc<dyn Clock>,
    release_phrase: String,
    allow_reduce_only: bool,
    latch: Arc<AtomicBool>,
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
    ///
    /// # Errors
    /// `PhraseTooShort` when the phrase (trimmed) is shorter than the config minimum.
    pub fn new(
        repo: R,
        clock: Arc<dyn Clock>,
        release_phrase: String,
        allow_reduce_only: bool,
    ) -> Result<Self, PhraseTooShort> {
        if release_phrase.trim().len() < MIN_PHRASE_LEN {
            return Err(PhraseTooShort);
        }
        Ok(Self {
            repo,
            clock,
            release_phrase,
            allow_reduce_only,
            latch: Arc::new(AtomicBool::new(false)),
        })
    }

    /// The in-process latch, shared so a read-only view can show it without a command.
    #[must_use]
    pub fn latch_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.latch)
    }

    async fn read(&self) -> Result<bool, RepoError> {
        let r = self.repo.is_engaged().await;
        if let Err(e) = &r {
            tracing::warn!(category = e.safe_reason(), "kill switch state unreadable");
        }
        r
    }

    /// True when engaged or when the state cannot be read (fail closed).
    pub async fn is_engaged(&self) -> bool {
        self.latch.load(Ordering::SeqCst) || self.read().await.unwrap_or(true)
    }

    /// `Ok` when an order may be sent. Engaged: only a reduce-only order, and only when the
    /// config allows it. Unreadable state refuses everything.
    ///
    /// # Errors
    /// `ExecError::KillSwitch` when engaged (and not exempt) or the state is unreadable.
    pub async fn admits(&self, reduce_only: bool) -> Result<(), ExecError> {
        let latched = self.latch.load(Ordering::SeqCst);
        match self.read().await {
            Ok(false) if !latched => Ok(()),
            Ok(_) if reduce_only && self.allow_reduce_only => Ok(()),
            Ok(_) | Err(_) => Err(ExecError::KillSwitch),
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

    /// Engage; state and audit row are written together. The latch is set first, so a failed
    /// write still leaves orders refused. Already engaged: no second row.
    ///
    /// # Errors
    /// `ExecError::Storage` when the write fails (the latch stays set).
    pub async fn engage(&self, actor: &str, reason: &str) -> Result<(), ExecError> {
        self.latch.store(true, Ordering::SeqCst);
        if matches!(self.read().await, Ok(true)) {
            return Ok(());
        }
        // One normalised reason (trimmed, "unspecified" when blank) for both records (RISK-130-1).
        let reason = reason_or_default(reason);
        let d = self.draft(actor, "kill_switch_engaged", reason);
        self.repo
            .set(true, reason, d, chain_hash)
            .await
            .map_err(|e| {
                tracing::warn!(
                    category = e.safe_reason(),
                    "kill switch engage not persisted"
                );
                ExecError::Storage
            })
    }

    /// Release with the exact typed phrase.
    ///
    /// # Errors
    /// `WrongPhrase` (nothing written) or `Storage` (still engaged).
    pub async fn release(&self, typed: &str, actor: &str) -> Result<(), ReleaseError> {
        if typed != self.release_phrase {
            return Err(ReleaseError::WrongPhrase);
        }
        // Unreadable state: do not release (fail closed). Already released: no row.
        if !self.read().await.map_err(|_| ReleaseError::Storage)? {
            // Latched but never persisted: record that, so the log shows why orders were refused.
            if self.latch.load(Ordering::SeqCst) {
                let d = self.draft(actor, "kill_switch_released", "engage_never_persisted");
                self.repo
                    .set(false, "engage_never_persisted", d, chain_hash)
                    .await
                    .map_err(|e| {
                        tracing::warn!(
                            category = e.safe_reason(),
                            "kill switch release not persisted"
                        );
                        ReleaseError::Storage
                    })?;
            }
            self.latch.store(false, Ordering::SeqCst);
            return Ok(());
        }
        let d = self.draft(actor, "kill_switch_released", "");
        self.repo
            .set(false, "released", d, chain_hash)
            .await
            .map_err(|e| {
                tracing::warn!(
                    category = e.safe_reason(),
                    "kill switch release not persisted"
                );
                ReleaseError::Storage
            })?;
        self.latch.store(false, Ordering::SeqCst);
        Ok(())
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
        pub reasons: Mutex<Vec<String>>,
        pub fail_read: AtomicBool,
        pub fail_write: AtomicBool,
        pub bad_hash: AtomicBool,
        pub fail_state_write: AtomicBool,
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
        async fn set(
            &self,
            e: bool,
            reason: &str,
            a: AuditDraft,
            c: ChainFn,
        ) -> Result<(), RepoError> {
            self.0.push(a, c)?; // write-ahead: a failed audit leaves the state unchanged
            if self.0.fail_state_write.load(Ordering::SeqCst) {
                return Err(RepoError::Other); // row appended, state unchanged
            }
            self.0.engaged.store(e, Ordering::SeqCst);
            self.0.reasons.lock().unwrap().push(reason.to_owned());
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
        KillSwitch::new(Shared(r.clone()), Arc::new(At), PHRASE.into(), reduce).unwrap()
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
    async fn typed_reason_reaches_the_repo_not_the_audit_kind() {
        let r = Arc::new(MemRepo::default());
        let k = ks(&r, true);
        k.engage("user", "daily loss").await.unwrap();
        k.release(PHRASE, "user").await.unwrap();
        let got = r.reasons.lock().unwrap().clone();
        assert_eq!(got, vec!["daily loss".to_owned(), "released".to_owned()]);
        let r2 = Arc::new(MemRepo::default());
        ks(&r2, true).engage("user", "  ").await.unwrap();
        assert_eq!(
            r2.reasons.lock().unwrap().clone(),
            vec!["unspecified".to_owned()]
        );
        let audit_reason = |r: &MemRepo| {
            let rows = r.rows.lock().unwrap();
            let v: serde_json::Value = serde_json::from_str(&rows[0].draft.payload).unwrap();
            v["reason"].as_str().unwrap().to_owned()
        };
        assert_eq!(audit_reason(&r2), "unspecified", "audit and row agree");
        let r3 = Arc::new(MemRepo::default());
        ks(&r3, true).engage("user", "  loss \n").await.unwrap();
        assert_eq!(r3.reasons.lock().unwrap().clone(), vec!["loss".to_owned()]);
        assert_eq!(audit_reason(&r3), "loss");
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
    }

    #[tokio::test]
    async fn failed_engage_write_still_refuses_until_a_real_release() {
        let r = Arc::new(MemRepo::default());
        let k = ks(&r, false);
        r.fail_write.store(true, SeqCst);
        assert_eq!(k.engage("user", "y").await, Err(ExecError::Storage));
        assert!(!r.engaged.load(SeqCst));
        assert_eq!(k.admits(false).await, Err(ExecError::KillSwitch));
        assert!(k.is_engaged().await);
        r.fail_write.store(false, SeqCst);
        k.release(PHRASE, "user").await.unwrap();
        assert_eq!(k.admits(false).await, Ok(()));
    }

    #[tokio::test]
    async fn engage_and_release_are_idempotent_and_unreadable_blocks_release() {
        let r = Arc::new(MemRepo::default());
        let k = ks(&r, true);
        k.release(PHRASE, "user").await.unwrap();
        assert_eq!(r.rows.lock().unwrap().len(), 0);
        k.engage("user", "x").await.unwrap();
        k.engage("user", "x").await.unwrap();
        assert_eq!(r.rows.lock().unwrap().len(), 1);
        r.fail_read.store(true, SeqCst);
        assert_eq!(k.release(PHRASE, "user").await, Err(ReleaseError::Storage));
        assert!(k.is_engaged().await);
    }

    #[tokio::test]
    async fn releasing_a_never_persisted_engage_is_recorded() {
        let r = Arc::new(MemRepo::default());
        let k = ks(&r, true);
        r.fail_write.store(true, SeqCst);
        assert!(k.engage("user", "y").await.is_err());
        r.fail_write.store(false, SeqCst);
        k.release(PHRASE, "user").await.unwrap();
        {
            let rows = r.rows.lock().unwrap();
            assert_eq!(rows.len(), 1);
            assert!(rows[0].draft.payload.contains("engage_never_persisted"));
        }
        assert_eq!(k.admits(false).await, Ok(()));
    }

    #[tokio::test]
    async fn failed_release_write_of_a_never_persisted_engage_keeps_the_latch() {
        let r = Arc::new(MemRepo::default());
        let k = ks(&r, true);
        r.fail_write.store(true, SeqCst);
        assert!(k.engage("user", "y").await.is_err());
        assert_eq!(k.release(PHRASE, "user").await, Err(ReleaseError::Storage));
        assert_eq!(k.admits(false).await, Err(ExecError::KillSwitch));
        assert!(r.rows.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn state_write_failure_after_the_row_still_refuses_and_exempt_flag_matters() {
        let r = Arc::new(MemRepo::default());
        let k = ks(&r, false);
        assert_eq!(k.admits(true).await, Ok(())); // not engaged: reduce-only needs no exemption
        r.fail_state_write.store(true, SeqCst);
        assert!(k.engage("user", "y").await.is_err());
        assert_eq!(r.rows.lock().unwrap().len(), 1);
        assert!(!r.engaged.load(SeqCst));
        assert_eq!(k.admits(true).await, Err(ExecError::KillSwitch));
    }

    #[test]
    fn short_or_empty_phrase_is_rejected() {
        let r = Arc::new(MemRepo::default());
        for p in ["", "   ", "short"] {
            let k = KillSwitch::new(Shared(r.clone()), Arc::new(At), p.into(), true);
            assert_eq!(k.err(), Some(PhraseTooShort));
        }
    }

    #[test]
    fn debug_never_shows_the_phrase() {
        let r = Arc::new(MemRepo::default());
        assert!(!format!("{:?}", ks(&r, true)).contains("RELEASE"));
    }
}
