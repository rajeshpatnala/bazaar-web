//! `audit-anchor adopt|status`: the audit anchor beside the database (ADR 0003).
//!
//! `adopt` is HUMAN-ONLY (owner decision 2026-11-25): agents never run it. It verifies the audit
//! chain, then writes the anchor for the current tail. An existing anchor file of any kind is
//! replaced only with `accept_rollback`. Neither command creates a database that is missing.
//!
//! Stop the server first: `adopt` takes no lock, and a running server's anchor writer could
//! race it (single writer per database, ADR 0003). `adopt` leaves no audit row; the printed
//! report (old seq, fault, new seq, tail time) is the owner's only trail (ADR 0003).

use std::path::{Path, PathBuf};

use bazaar_core::ports::AuditRepo;
use bazaar_execution::audit::chain_hash;
use bazaar_store::{AdoptReport, SqliteStore};

use super::{CommandError, DB_FILE, repo_label};

/// What `adopt` changed, plus the tail it anchored so the owner can check it against their own
/// records (a missing anchor adopts the current tail on trust, ADR 0003).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdoptOutcome {
    /// Old seq, fault found and new seq.
    pub report: AdoptReport,
    /// IST timestamp (RFC 3339) of the last audit row now anchored; `None` for an empty log.
    pub tail_ts: Option<String>,
}

/// What `audit-anchor status` found (the store opened, so the chain and the anchor both passed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnchorReport {
    /// Seq the anchor file holds.
    pub seq: u64,
    /// The last anchor write failed and has not been repaired.
    pub lag: bool,
}

fn db_path(data_dir: &Path) -> Result<PathBuf, CommandError> {
    // Canonicalised like the server does, so a relative or symlinked directory is resolved once.
    let dir = std::fs::canonicalize(data_dir).map_err(|_| CommandError::NoDatabase)?;
    let db = dir.join(DB_FILE);
    if db.is_file() {
        Ok(db)
    } else {
        Err(CommandError::NoDatabase)
    }
}

/// Verify the chain and write the anchor for the tail. `accept_rollback` allows replacing an
/// existing anchor (ahead of the log, different hash, unparsable, or consistent). A MISSING
/// anchor adopts without it and trusts the current tail: check the printed tail seq and time
/// against your own records. Stop the server first (no lock is taken).
///
/// # Errors
/// `NoDatabase`; `Store("corrupt")` on a broken chain (nothing written); `Store("conflict")` when
/// an anchor exists and `accept_rollback` is false (nothing written).
pub async fn audit_anchor_adopt(
    data_dir: &Path,
    accept_rollback: bool,
) -> Result<AdoptOutcome, CommandError> {
    let db = db_path(data_dir)?;
    let report = SqliteStore::anchor_adopt(&db, chain_hash, accept_rollback)
        .await
        .map_err(repo_label)?;
    // The adopt verified the chain and wrote the anchor, so this open passes; it only reads the
    // tail time for the report.
    let store = crate::compose::open_store(&db).await.map_err(repo_label)?;
    let rows = AuditRepo::read_all(&store).await.map_err(repo_label)?;
    let tail_ts = rows.last().map(|r| r.draft.ts_ist_rfc3339.clone());
    Ok(AdoptOutcome { report, tail_ts })
}

/// Open the store the way the app does (chain AND anchor verified) and report the anchor. This
/// opens the store, so it may run a migration or write the crash-window catch-up: it is not
/// strictly read-only.
///
/// # Errors
/// `NoDatabase`; `Store("corrupt")` when the chain or the anchor check refuses.
pub async fn audit_anchor_status(data_dir: &Path) -> Result<AnchorReport, CommandError> {
    let db = db_path(data_dir)?;
    let store = crate::compose::open_store(&db).await.map_err(repo_label)?;
    // `open_anchored` always sets an anchor; a store without one cannot be reported on.
    let s = store
        .anchor_status()
        .await
        .ok_or(CommandError::Store("other"))?;
    Ok(AnchorReport {
        seq: s.last_seq,
        lag: s.lag,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use bazaar_core::ports::{AuditDraft, AuditRepo};
    use bazaar_store::AnchorFault;

    use super::*;

    fn draft(kind: &str) -> AuditDraft {
        AuditDraft {
            ts_ist_rfc3339: "2026-11-25T09:15:00+05:30".to_owned(),
            actor: "tester".to_owned(),
            kind: kind.to_owned(),
            payload: "{}".to_owned(),
        }
    }

    /// A data dir with `n` audit rows and a settled anchor.
    async fn seeded(n: usize) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let s = crate::compose::open_store(&dir.path().join(DB_FILE))
            .await
            .unwrap();
        for i in 0..n {
            AuditRepo::append(&s, draft(&format!("k{i}")), chain_hash)
                .await
                .unwrap();
        }
        s.anchor_flush().await.unwrap();
        dir
    }

    fn anchor_file(dir: &Path) -> PathBuf {
        dir.join(format!("{DB_FILE}.anchor"))
    }

    #[tokio::test]
    async fn status_reports_the_anchored_seq() {
        let dir = seeded(3).await;
        let r = audit_anchor_status(dir.path()).await.unwrap();
        assert_eq!(r, AnchorReport { seq: 3, lag: false });
    }

    #[tokio::test]
    async fn status_names_the_corrupt_category_for_a_missing_anchor() {
        let dir = seeded(2).await;
        std::fs::remove_file(anchor_file(dir.path())).unwrap();
        assert_eq!(
            audit_anchor_status(dir.path()).await,
            Err(CommandError::Store("corrupt"))
        );
    }

    #[tokio::test]
    async fn both_commands_refuse_a_missing_database_and_create_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            audit_anchor_status(dir.path()).await,
            Err(CommandError::NoDatabase)
        );
        assert_eq!(
            audit_anchor_adopt(dir.path(), true).await.unwrap_err(),
            CommandError::NoDatabase
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn adopt_on_a_missing_anchor_writes_the_tail_and_status_then_passes() {
        let dir = seeded(3).await;
        std::fs::remove_file(anchor_file(dir.path())).unwrap();
        let r = audit_anchor_adopt(dir.path(), false).await.unwrap();
        assert_eq!(
            (r.report.old_seq, r.report.old_fault, r.report.new_seq),
            (None, None, 3)
        );
        assert_eq!(r.tail_ts.as_deref(), Some("2026-11-25T09:15:00+05:30"));
        assert_eq!(audit_anchor_status(dir.path()).await.unwrap().seq, 3);
    }

    /// An existing anchor is never overwritten silently: refused, file byte-identical.
    #[tokio::test]
    async fn adopt_over_an_existing_anchor_is_refused_without_the_flag_and_changes_nothing() {
        let dir = seeded(3).await;
        let before = std::fs::read(anchor_file(dir.path())).unwrap();
        let e = audit_anchor_adopt(dir.path(), false).await.unwrap_err();
        assert_eq!(e, CommandError::Store("conflict"));
        assert_eq!(std::fs::read(anchor_file(dir.path())).unwrap(), before);
    }

    /// A log shorter than the anchor: needs the flag, and the report carries both seqs.
    #[tokio::test]
    async fn adopt_over_an_anchor_ahead_of_the_log_needs_the_flag_and_reports_both_seqs() {
        let dir = seeded(2).await;
        let ahead = std::fs::read_to_string(anchor_file(dir.path()))
            .unwrap()
            .replace("seq=2", "seq=9");
        std::fs::write(anchor_file(dir.path()), &ahead).unwrap();
        assert_eq!(
            audit_anchor_adopt(dir.path(), false).await.unwrap_err(),
            CommandError::Store("conflict")
        );
        assert_eq!(
            std::fs::read_to_string(anchor_file(dir.path())).unwrap(),
            ahead
        );
        let r = audit_anchor_adopt(dir.path(), true).await.unwrap();
        assert_eq!(
            (r.report.old_seq, r.report.old_fault, r.report.new_seq),
            (Some(9), Some(AnchorFault::Ahead), 2)
        );
        assert_eq!(audit_anchor_status(dir.path()).await.unwrap().seq, 2);
    }

    /// A broken chain is refused even with the flag, and the anchor stays as it was.
    #[tokio::test]
    async fn adopt_on_a_corrupt_chain_is_refused_and_writes_nothing() {
        use sqlx::sqlite::SqliteConnectOptions;
        use sqlx::{ConnectOptions, Connection};
        let dir = seeded(3).await;
        let before = std::fs::read(anchor_file(dir.path())).unwrap();
        let mut c = SqliteConnectOptions::new()
            .filename(dir.path().join(DB_FILE))
            .connect()
            .await
            .unwrap();
        sqlx::query("DROP TRIGGER audit_log_no_update")
            .execute(&mut c)
            .await
            .unwrap();
        sqlx::query("UPDATE audit_log SET actor = 'evil' WHERE seq = 2")
            .execute(&mut c)
            .await
            .unwrap();
        c.close().await.unwrap();
        let e = audit_anchor_adopt(dir.path(), true).await.unwrap_err();
        assert_eq!(e, CommandError::Store("corrupt"));
        assert_eq!(std::fs::read(anchor_file(dir.path())).unwrap(), before);
    }

    /// An unparsable anchor is evidence: refused without the flag, replaced with it.
    #[tokio::test]
    async fn adopt_over_an_unparsable_anchor_needs_the_flag_and_names_the_fault() {
        let dir = seeded(2).await;
        std::fs::write(anchor_file(dir.path()), "garbage").unwrap();
        assert_eq!(
            audit_anchor_adopt(dir.path(), false).await.unwrap_err(),
            CommandError::Store("conflict")
        );
        assert_eq!(std::fs::read(anchor_file(dir.path())).unwrap(), b"garbage");
        let r = audit_anchor_adopt(dir.path(), true).await.unwrap();
        assert_eq!(
            (r.report.old_seq, r.report.old_fault, r.report.new_seq),
            (None, Some(AnchorFault::Unparsable), 2)
        );
        assert_eq!(audit_anchor_status(dir.path()).await.unwrap().seq, 2);
    }

    /// An anchor naming a different hash at a real seq: needs the flag, fault is `HashMismatch`.
    #[tokio::test]
    async fn adopt_over_a_different_hash_needs_the_flag_and_names_the_fault() {
        let dir = seeded(2).await;
        let text = std::fs::read_to_string(anchor_file(dir.path())).unwrap();
        let hash_line = text.lines().find(|l| l.starts_with("hash=")).unwrap();
        let other = format!("hash={}", "0".repeat(64));
        let forged = text.replace(hash_line, &other);
        std::fs::write(anchor_file(dir.path()), &forged).unwrap();
        assert_eq!(
            audit_anchor_adopt(dir.path(), false).await.unwrap_err(),
            CommandError::Store("conflict")
        );
        let r = audit_anchor_adopt(dir.path(), true).await.unwrap();
        assert_eq!(
            (r.report.old_seq, r.report.old_fault, r.report.new_seq),
            (Some(2), Some(AnchorFault::HashMismatch), 2)
        );
    }
}
