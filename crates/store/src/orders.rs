//! Order repo: rows with an optimistic version; every `advance` persists the new state, an
//! event row and an audit row in one transaction, and a stale version loses with
//! `RepoError::Conflict`. States and events are opaque text labels owned by `execution`
//! (store depends on core only).

use bazaar_core::ports::{AuditDraft, ChainFn, RepoError};
use sqlx::sqlite::{Sqlite, SqliteRow};
use sqlx::{Row, Transaction};

use crate::audit::append_in_tx;
use crate::sqlite::{SqliteStore, map_err};

const MAX_LABEL: usize = 64;
const MAX_PAYLOAD: usize = 64 * 1024;
const MAX_LIST: i64 = 10_000;

/// One stored order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderRow {
    pub order_id: String,
    pub state: String,
    pub version: u64,
    pub payload: String,
    pub created_ns: i64,
    pub updated_ns: i64,
}

/// One event: `(version, event, from_state, to_state)`.
pub type OrderEvent = (u64, String, Option<String>, String);

/// A version-checked state change.
#[derive(Debug, Clone, Copy)]
pub struct OrderTransition<'a> {
    pub order_id: &'a str,
    pub expected_version: u64,
    pub event: &'a str,
    pub new_state: &'a str,
    pub at_ns: i64,
}

/// Labels must be non-empty (`Other`) and at most `MAX_LABEL` bytes (`TooLarge`).
pub(crate) fn check_label(s: &str) -> Result<(), RepoError> {
    if s.is_empty() {
        return Err(RepoError::Other);
    }
    if s.len() > MAX_LABEL {
        return Err(RepoError::TooLarge);
    }
    Ok(())
}

fn to_row(r: &SqliteRow) -> Result<OrderRow, RepoError> {
    let g = |e: sqlx::Error| map_err(&e);
    let v: i64 = r.try_get("version").map_err(g)?;
    Ok(OrderRow {
        order_id: r.try_get("order_id").map_err(g)?,
        state: r.try_get("state").map_err(g)?,
        version: u64::try_from(v).map_err(|_| RepoError::Corrupt)?,
        payload: r.try_get("payload").map_err(g)?,
        created_ns: r.try_get("created_ns").map_err(g)?,
        updated_ns: r.try_get("updated_ns").map_err(g)?,
    })
}

const GET: &str = "SELECT order_id, state, version, payload, created_ns, updated_ns \
    FROM orders WHERE order_id = ?";
const BY_STATE: &str = "SELECT order_id, state, version, payload, created_ns, updated_ns \
    FROM orders WHERE state = ? ORDER BY created_ns, order_id LIMIT ?";

/// Unique-violation becomes `Conflict`; everything else keeps its category.
pub(crate) fn conflict_or(e: &sqlx::Error) -> RepoError {
    if let sqlx::Error::Database(d) = e
        && d.is_unique_violation()
    {
        return RepoError::Conflict;
    }
    map_err(e)
}

/// Version-checked state change plus its event row, inside the caller's transaction.
pub(crate) async fn advance_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    t: &OrderTransition<'_>,
) -> Result<u64, RepoError> {
    check_label(t.event)?;
    check_label(t.new_state)?;
    let expected = i64::try_from(t.expected_version).map_err(|_| RepoError::Other)?;
    let next = expected.checked_add(1).ok_or(RepoError::Other)?;
    let from: Option<String> =
        sqlx::query_scalar("SELECT state FROM orders WHERE order_id = ? AND version = ?")
            .bind(t.order_id)
            .bind(expected)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| map_err(&e))?;
    let Some(from) = from else {
        return Err(RepoError::Conflict);
    };
    let done = sqlx::query(
        "UPDATE orders SET state = ?, version = ?, updated_ns = ? \
         WHERE order_id = ? AND version = ?",
    )
    .bind(t.new_state)
    .bind(next)
    .bind(t.at_ns)
    .bind(t.order_id)
    .bind(expected)
    .execute(&mut **tx)
    .await
    .map_err(|e| map_err(&e))?;
    if done.rows_affected() != 1 {
        return Err(RepoError::Conflict);
    }
    sqlx::query(
        "INSERT INTO order_events (order_id, version, event, from_state, to_state, at_ns) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(t.order_id)
    .bind(next)
    .bind(t.event)
    .bind(from)
    .bind(t.new_state)
    .bind(t.at_ns)
    .execute(&mut **tx)
    .await
    .map_err(|e| map_err(&e))?;
    u64::try_from(next).map_err(|_| RepoError::Other)
}

impl SqliteStore {
    /// Insert a new order at version 1 with its first event. A duplicate id is `Conflict`.
    pub async fn order_create(
        &self,
        order_id: &str,
        state: &str,
        payload: &str,
        at_ns: i64,
    ) -> Result<OrderRow, RepoError> {
        check_label(order_id)?;
        check_label(state)?;
        if payload.len() > MAX_PAYLOAD {
            return Err(RepoError::TooLarge);
        }
        let mut tx = self.pool.begin().await.map_err(|e| map_err(&e))?;
        sqlx::query(
            "INSERT INTO orders (order_id, state, version, payload, created_ns, updated_ns) \
             VALUES (?, ?, 1, ?, ?, ?)",
        )
        .bind(order_id)
        .bind(state)
        .bind(payload)
        .bind(at_ns)
        .bind(at_ns)
        .execute(&mut *tx)
        .await
        .map_err(|e| conflict_or(&e))?;
        sqlx::query(
            "INSERT INTO order_events (order_id, version, event, from_state, to_state, at_ns) \
             VALUES (?, 1, 'created', NULL, ?, ?)",
        )
        .bind(order_id)
        .bind(state)
        .bind(at_ns)
        .execute(&mut *tx)
        .await
        .map_err(|e| map_err(&e))?;
        tx.commit().await.map_err(|e| map_err(&e))?;
        Ok(OrderRow {
            order_id: order_id.to_owned(),
            state: state.to_owned(),
            version: 1,
            payload: payload.to_owned(),
            created_ns: at_ns,
            updated_ns: at_ns,
        })
    }

    /// One order by id.
    pub async fn order_get(&self, order_id: &str) -> Result<Option<OrderRow>, RepoError> {
        let row = sqlx::query(GET)
            .bind(order_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| map_err(&e))?;
        row.as_ref().map(to_row).transpose()
    }

    /// Orders in `state`, oldest first (startup reload of pending approvals, `expire_stale`).
    /// More than 10 000 rows is `TooLarge`, never a silent truncation.
    pub async fn orders_in_state(&self, state: &str) -> Result<Vec<OrderRow>, RepoError> {
        self.orders_in_state_limit(state, MAX_LIST).await
    }

    pub(crate) async fn orders_in_state_limit(
        &self,
        state: &str,
        limit: i64,
    ) -> Result<Vec<OrderRow>, RepoError> {
        let rows = sqlx::query(BY_STATE)
            .bind(state)
            .bind(limit.saturating_add(1))
            .fetch_all(&self.pool)
            .await
            .map_err(|e| map_err(&e))?;
        if i64::try_from(rows.len()).map_err(|_| RepoError::Other)? > limit {
            return Err(RepoError::TooLarge);
        }
        rows.iter().map(to_row).collect()
    }

    /// Move to `t.new_state` if the stored version is still `t.expected_version`; returns the
    /// new version. State change, event row and audit row commit together or not at all; a
    /// stale version is `Conflict`. Approval decisions must use `approval_decide` instead.
    pub async fn order_advance(
        &self,
        t: &OrderTransition<'_>,
        audit: &AuditDraft,
        chain: ChainFn,
    ) -> Result<u64, RepoError> {
        let mut tx = self.pool.begin().await.map_err(|e| map_err(&e))?;
        let v = advance_in_tx(&mut tx, t).await?;
        append_in_tx(&mut tx, audit, chain).await?;
        tx.commit().await.map_err(|e| map_err(&e))?;
        Ok(v)
    }

    /// Events of one order, in version order.
    pub async fn order_events(&self, order_id: &str) -> Result<Vec<OrderEvent>, RepoError> {
        let rows = sqlx::query(
            "SELECT version, event, from_state, to_state FROM order_events \
             WHERE order_id = ? ORDER BY version",
        )
        .bind(order_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| map_err(&e))?;
        let g = |e: sqlx::Error| map_err(&e);
        rows.iter()
            .map(|r| {
                let v: i64 = r.try_get("version").map_err(g)?;
                Ok((
                    u64::try_from(v).map_err(|_| RepoError::Corrupt)?,
                    r.try_get("event").map_err(g)?,
                    r.try_get("from_state").map_err(g)?,
                    r.try_get("to_state").map_err(g)?,
                ))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::{test_chain, test_draft};

    async fn store() -> SqliteStore {
        SqliteStore::open_in_memory().await.unwrap()
    }

    async fn adv(
        s: &SqliteStore,
        id: &str,
        ver: u64,
        event: &str,
        to: &str,
        at: i64,
    ) -> Result<u64, RepoError> {
        let t = OrderTransition {
            order_id: id,
            expected_version: ver,
            event,
            new_state: to,
            at_ns: at,
        };
        s.order_advance(&t, &test_draft("order"), test_chain).await
    }

    async fn audit_rows(s: &SqliteStore) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM audit_log")
            .fetch_one(&s.pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn create_get_and_duplicate() {
        let s = store().await;
        let r = s
            .order_create("o1", "PendingApproval", "{}", 10)
            .await
            .unwrap();
        assert_eq!(r.version, 1);
        assert_eq!(s.order_get("o1").await.unwrap(), Some(r));
        assert_eq!(s.order_get("nope").await.unwrap(), None);
        assert_eq!(
            s.order_create("o1", "X", "{}", 11).await,
            Err(RepoError::Conflict)
        );
    }

    #[tokio::test]
    async fn advance_bumps_version_logs_events_and_chains_audit() {
        let s = store().await;
        s.order_create("o1", "PendingApproval", "{}", 1)
            .await
            .unwrap();
        assert_eq!(adv(&s, "o1", 1, "approve", "Approved", 2).await, Ok(2));
        assert_eq!(adv(&s, "o1", 2, "send", "Sent", 3).await, Ok(3));
        let ev = s.order_events("o1").await.unwrap();
        assert_eq!(ev.len(), 3);
        assert_eq!(
            ev[2],
            (3, "send".into(), Some("Approved".into()), "Sent".into())
        );
        assert_eq!(s.orders_in_state("Sent").await.unwrap().len(), 1);
        assert_eq!(s.orders_in_state("Approved").await.unwrap(), Vec::new());
        assert_eq!(audit_rows(&s).await, 2);
        let prev2: Vec<u8> = sqlx::query_scalar("SELECT prev_hash FROM audit_log WHERE seq = 2")
            .fetch_one(&s.pool)
            .await
            .unwrap();
        let hash1: Vec<u8> = sqlx::query_scalar("SELECT hash FROM audit_log WHERE seq = 1")
            .fetch_one(&s.pool)
            .await
            .unwrap();
        assert_eq!(prev2, hash1);
    }

    #[tokio::test]
    async fn stale_version_loses_and_changes_nothing() {
        let s = store().await;
        s.order_create("o1", "PendingApproval", "{}", 1)
            .await
            .unwrap();
        adv(&s, "o1", 1, "approve", "Approved", 2).await.unwrap();
        assert_eq!(
            adv(&s, "o1", 1, "decline", "Declined", 3).await,
            Err(RepoError::Conflict)
        );
        assert_eq!(
            adv(&s, "missing", 1, "x", "Y", 3).await,
            Err(RepoError::Conflict)
        );
        let o = s.order_get("o1").await.unwrap().unwrap();
        assert_eq!((o.state.as_str(), o.version), ("Approved", 2));
        assert_eq!(s.order_events("o1").await.unwrap().len(), 2);
        assert_eq!(audit_rows(&s).await, 1);
    }

    #[tokio::test]
    async fn concurrent_advance_has_one_winner_and_loser_gets_conflict() {
        let s = store().await;
        s.order_create("o1", "PendingApproval", "{}", 1)
            .await
            .unwrap();
        let (a, b) = tokio::join!(
            adv(&s, "o1", 1, "approve", "Approved", 2),
            adv(&s, "o1", 1, "decline", "Declined", 2)
        );
        let (ok, lost) = if a.is_ok() { (a, b) } else { (b, a) };
        assert_eq!(ok, Ok(2));
        assert_eq!(lost, Err(RepoError::Conflict));
    }

    #[tokio::test]
    async fn failed_event_insert_rolls_back_the_whole_advance() {
        let s = store().await;
        s.order_create("o1", "PendingApproval", "{}", 1)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TRIGGER boom BEFORE INSERT ON order_events WHEN NEW.event = 'boom' \
             BEGIN SELECT RAISE(ABORT, 'boom'); END",
        )
        .execute(&s.pool)
        .await
        .unwrap();
        assert!(adv(&s, "o1", 1, "boom", "Approved", 2).await.is_err());
        let o = s.order_get("o1").await.unwrap().unwrap();
        assert_eq!((o.state.as_str(), o.version), ("PendingApproval", 1));
        assert_eq!(s.order_events("o1").await.unwrap().len(), 1);
        assert_eq!(audit_rows(&s).await, 0);
    }

    #[tokio::test]
    async fn orders_in_state_orders_oldest_first_and_refuses_truncation() {
        let s = store().await;
        s.order_create("b", "P", "{}", 5).await.unwrap();
        s.order_create("a", "P", "{}", 5).await.unwrap();
        s.order_create("c", "P", "{}", 1).await.unwrap();
        let ids: Vec<String> = s
            .orders_in_state("P")
            .await
            .unwrap()
            .into_iter()
            .map(|o| o.order_id)
            .collect();
        assert_eq!(ids, ["c", "a", "b"]);
        assert_eq!(s.orders_in_state_limit("P", 3).await.unwrap().len(), 3);
        assert_eq!(
            s.orders_in_state_limit("P", 2).await,
            Err(RepoError::TooLarge)
        );
    }

    #[tokio::test]
    async fn append_only_and_no_delete_triggers() {
        let s = store().await;
        s.order_create("o1", "A", "{}", 1).await.unwrap();
        assert!(
            sqlx::query("UPDATE order_events SET event = 'x'")
                .execute(&s.pool)
                .await
                .is_err()
        );
        assert!(
            sqlx::query("DELETE FROM order_events")
                .execute(&s.pool)
                .await
                .is_err()
        );
        assert!(
            sqlx::query("DELETE FROM orders")
                .execute(&s.pool)
                .await
                .is_err()
        );
        adv(&s, "o1", 1, "e", "B", 2).await.unwrap();
        assert!(
            sqlx::query("UPDATE audit_log SET actor = 'x'")
                .execute(&s.pool)
                .await
                .is_err()
        );
        assert!(
            sqlx::query("DELETE FROM audit_log")
                .execute(&s.pool)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn rejects_empty_and_oversize_inputs() {
        let s = store().await;
        assert_eq!(
            s.order_create("", "A", "{}", 1).await,
            Err(RepoError::Other)
        );
        let big = "x".repeat(MAX_PAYLOAD + 1);
        assert_eq!(
            s.order_create("o", "A", &big, 1).await,
            Err(RepoError::TooLarge)
        );
        let long = "x".repeat(MAX_LABEL + 1);
        assert_eq!(
            s.order_create(&long, "A", "{}", 1).await,
            Err(RepoError::TooLarge)
        );
    }
}
