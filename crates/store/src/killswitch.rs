//! Kill-switch state: a single durable row. Callers treat any error as engaged (fail closed).
//! Writes bump an optimistic version, so a stale writer loses with `RepoError::Conflict`.

use bazaar_core::ports::{AuditDraft, ChainFn, RepoError};
use sqlx::Row;

use crate::audit::append_in_tx;
use crate::sqlite::{SqliteStore, map_err};

const MAX_TEXT: usize = 500;

/// The stored kill-switch state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KillSwitchRow {
    pub engaged: bool,
    pub version: u64,
    pub actor: String,
    pub reason: String,
    pub changed_ns: i64,
}

impl SqliteStore {
    /// Current state. A missing or undecodable row is an error, never "not engaged".
    pub async fn kill_switch_get(&self) -> Result<KillSwitchRow, RepoError> {
        let r = sqlx::query(
            "SELECT engaged, version, actor, reason, changed_ns FROM kill_switch WHERE id = 1",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| map_err(&e))?
        .ok_or(RepoError::Corrupt)?;
        let g = |e: sqlx::Error| map_err(&e);
        let engaged: i64 = r.try_get("engaged").map_err(g)?;
        let version: i64 = r.try_get("version").map_err(g)?;
        Ok(KillSwitchRow {
            engaged: engaged != 0,
            version: u64::try_from(version).map_err(|_| RepoError::Corrupt)?,
            actor: r.try_get("actor").map_err(g)?,
            reason: r.try_get("reason").map_err(g)?,
            changed_ns: r.try_get("changed_ns").map_err(g)?,
        })
    }

    /// Set the state if the stored version is still `expected_version`; returns the new version.
    /// The change and its append-only event row commit together or not at all.
    pub async fn kill_switch_set(
        &self,
        expected_version: u64,
        engaged: bool,
        actor: &str,
        reason: &str,
        at_ns: i64,
    ) -> Result<u64, RepoError> {
        if actor.is_empty() || reason.is_empty() {
            return Err(RepoError::Other);
        }
        if actor.len() > MAX_TEXT || reason.len() > MAX_TEXT {
            return Err(RepoError::TooLarge);
        }
        let expected = i64::try_from(expected_version).map_err(|_| RepoError::Other)?;
        let next = expected.checked_add(1).ok_or(RepoError::Other)?;
        let mut tx = self.pool.begin().await.map_err(|e| map_err(&e))?;
        let done = sqlx::query(
            "UPDATE kill_switch SET engaged = ?, version = ?, actor = ?, reason = ?, \
             changed_ns = ? WHERE id = 1 AND version = ?",
        )
        .bind(i64::from(engaged))
        .bind(next)
        .bind(actor)
        .bind(reason)
        .bind(at_ns)
        .bind(expected)
        .execute(&mut *tx)
        .await
        .map_err(|e| map_err(&e))?;
        if done.rows_affected() != 1 {
            return Err(RepoError::Conflict);
        }
        sqlx::query(
            "INSERT INTO kill_switch_events (version, engaged, actor, reason, changed_ns) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(next)
        .bind(i64::from(engaged))
        .bind(actor)
        .bind(reason)
        .bind(at_ns)
        .execute(&mut *tx)
        .await
        .map_err(|e| map_err(&e))?;
        tx.commit().await.map_err(|e| map_err(&e))?;
        u64::try_from(next).map_err(|_| RepoError::Other)
    }

    /// Set the state from the current version, with the audit row in the same transaction
    /// (port `set`). A concurrent writer that moved the version first is `Conflict`.
    pub(crate) async fn kill_switch_set_audited(
        &self,
        engaged: bool,
        actor: &str,
        reason: &str,
        at_ns: i64,
        audit: &AuditDraft,
        chain: ChainFn,
    ) -> Result<(), RepoError> {
        if actor.is_empty() || reason.is_empty() {
            return Err(RepoError::Other);
        }
        if actor.len() > MAX_TEXT || reason.len() > MAX_TEXT {
            return Err(RepoError::TooLarge);
        }
        let mut tx = self.pool.begin().await.map_err(|e| map_err(&e))?;
        let expected: i64 = sqlx::query_scalar("SELECT version FROM kill_switch WHERE id = 1")
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| map_err(&e))?
            .ok_or(RepoError::Corrupt)?;
        let next = expected.checked_add(1).ok_or(RepoError::Other)?;
        let done = sqlx::query(
            "UPDATE kill_switch SET engaged = ?, version = ?, actor = ?, reason = ?, \
             changed_ns = ? WHERE id = 1 AND version = ?",
        )
        .bind(i64::from(engaged))
        .bind(next)
        .bind(actor)
        .bind(reason)
        .bind(at_ns)
        .bind(expected)
        .execute(&mut *tx)
        .await
        .map_err(|e| map_err(&e))?;
        if done.rows_affected() != 1 {
            return Err(RepoError::Conflict);
        }
        sqlx::query(
            "INSERT INTO kill_switch_events (version, engaged, actor, reason, changed_ns) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(next)
        .bind(i64::from(engaged))
        .bind(actor)
        .bind(reason)
        .bind(at_ns)
        .execute(&mut *tx)
        .await
        .map_err(|e| map_err(&e))?;
        append_in_tx(&mut tx, audit, chain).await?;
        tx.commit().await.map_err(|e| map_err(&e))?;
        self.anchor_after_commit().await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn starts_released_and_sets_with_version() {
        let s = SqliteStore::open_in_memory().await.unwrap();
        let k = s.kill_switch_get().await.unwrap();
        assert!(!k.engaged);
        assert_eq!(
            s.kill_switch_set(k.version, true, "me", "test", 9).await,
            Ok(2)
        );
        let k2 = s.kill_switch_get().await.unwrap();
        assert_eq!(
            (k2.engaged, k2.version, k2.reason.as_str()),
            (true, 2, "test")
        );
        assert_eq!(
            s.kill_switch_set(1, false, "me", "stale", 10).await,
            Err(RepoError::Conflict)
        );
        assert!(s.kill_switch_get().await.unwrap().engaged);
    }

    #[tokio::test]
    async fn survives_reopen_and_second_reader_sees_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let a = SqliteStore::open(&path).await.unwrap();
        a.kill_switch_set(1, true, "me", "halt", 5).await.unwrap();
        let b = SqliteStore::open(&path).await.unwrap();
        assert!(b.kill_switch_get().await.unwrap().engaged);
        drop(a);
        drop(b);
        let c = SqliteStore::open(&path).await.unwrap();
        let k = c.kill_switch_get().await.unwrap();
        assert!(k.engaged);
        assert_eq!(k.actor, "me");
    }

    async fn event_count(s: &SqliteStore) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM kill_switch_events")
            .fetch_one(&s.pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn oversize_and_empty_text_are_errors() {
        let s = SqliteStore::open_in_memory().await.unwrap();
        let long = "x".repeat(MAX_TEXT + 1);
        assert_eq!(
            s.kill_switch_set(1, true, &long, "r", 1).await,
            Err(RepoError::TooLarge)
        );
        assert_eq!(
            s.kill_switch_set(1, true, "a", &long, 1).await,
            Err(RepoError::TooLarge)
        );
        assert_eq!(
            s.kill_switch_set(1, true, "", "r", 1).await,
            Err(RepoError::Other)
        );
        assert_eq!(
            s.kill_switch_set(1, true, "a", "", 1).await,
            Err(RepoError::Other)
        );
        assert_eq!(event_count(&s).await, 0);
    }

    #[tokio::test]
    async fn missing_table_is_an_error() {
        let s = SqliteStore::open_in_memory().await.unwrap();
        sqlx::query("DROP TABLE kill_switch")
            .execute(&s.pool)
            .await
            .unwrap();
        assert!(s.kill_switch_get().await.is_err());
    }

    #[tokio::test]
    async fn missing_row_is_corrupt_not_released() {
        let s = SqliteStore::open_in_memory().await.unwrap();
        sqlx::query("DROP TRIGGER kill_switch_no_delete")
            .execute(&s.pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM kill_switch")
            .execute(&s.pool)
            .await
            .unwrap();
        assert_eq!(s.kill_switch_get().await, Err(RepoError::Corrupt));
    }

    #[tokio::test]
    async fn each_set_writes_an_event_row() {
        let s = SqliteStore::open_in_memory().await.unwrap();
        s.kill_switch_set(1, true, "me", "halt", 5).await.unwrap();
        s.kill_switch_set(2, false, "me", "resume", 6)
            .await
            .unwrap();
        assert_eq!(event_count(&s).await, 2);
        let last: (i64, i64, String) = sqlx::query_as(
            "SELECT version, engaged, reason FROM kill_switch_events ORDER BY version DESC LIMIT 1",
        )
        .fetch_one(&s.pool)
        .await
        .unwrap();
        assert_eq!(last, (3, 0, "resume".to_owned()));
    }

    #[tokio::test]
    async fn stale_set_writes_no_event() {
        let s = SqliteStore::open_in_memory().await.unwrap();
        assert_eq!(
            s.kill_switch_set(7, true, "me", "x", 1).await,
            Err(RepoError::Conflict)
        );
        assert_eq!(event_count(&s).await, 0);
        assert!(!s.kill_switch_get().await.unwrap().engaged);
    }

    #[tokio::test]
    async fn failed_event_insert_rolls_back_the_state_change() {
        let s = SqliteStore::open_in_memory().await.unwrap();
        sqlx::query(
            "CREATE TRIGGER boom BEFORE INSERT ON kill_switch_events \
             BEGIN SELECT RAISE(ABORT, 'boom'); END",
        )
        .execute(&s.pool)
        .await
        .unwrap();
        assert!(s.kill_switch_set(1, true, "me", "halt", 1).await.is_err());
        let k = s.kill_switch_get().await.unwrap();
        assert_eq!((k.engaged, k.version), (false, 1));
    }

    #[tokio::test]
    async fn concurrent_sets_have_one_winner() {
        let s = SqliteStore::open_in_memory().await.unwrap();
        let (a, b) = tokio::join!(
            s.kill_switch_set(1, true, "a", "r", 1),
            s.kill_switch_set(1, true, "b", "r", 2)
        );
        let mut r = [a, b];
        r.sort_by_key(Result::is_err);
        assert_eq!(r[0], Ok(2));
        assert_eq!(r[1], Err(RepoError::Conflict));
        assert_eq!(event_count(&s).await, 1);
    }

    #[tokio::test]
    async fn events_are_append_only() {
        let s = SqliteStore::open_in_memory().await.unwrap();
        s.kill_switch_set(1, true, "me", "halt", 5).await.unwrap();
        assert!(
            sqlx::query("UPDATE kill_switch_events SET reason='x'")
                .execute(&s.pool)
                .await
                .is_err()
        );
        assert!(
            sqlx::query("DELETE FROM kill_switch_events")
                .execute(&s.pool)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn row_cannot_be_deleted() {
        let s = SqliteStore::open_in_memory().await.unwrap();
        assert!(
            sqlx::query("DELETE FROM kill_switch")
                .execute(&s.pool)
                .await
                .is_err()
        );
    }
}
