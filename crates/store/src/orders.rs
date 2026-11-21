//! Order repo: rows with an optimistic version; every `advance` persists the new state and an
//! event row in one transaction, and a stale version loses with `RepoError::Conflict`.
//! States and events are opaque text labels owned by `execution` (store depends on core only).

use bazaar_core::ports::RepoError;
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

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

fn check_label(s: &str) -> Result<(), RepoError> {
    if s.is_empty() || s.len() > MAX_LABEL {
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
    pub async fn orders_in_state(&self, state: &str) -> Result<Vec<OrderRow>, RepoError> {
        let rows = sqlx::query(BY_STATE)
            .bind(state)
            .bind(MAX_LIST)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| map_err(&e))?;
        rows.iter().map(to_row).collect()
    }

    /// Move to `new_state` if the stored version is still `expected_version`; returns the new
    /// version. State change and event row commit together; a stale version is `Conflict`.
    pub async fn order_advance(
        &self,
        order_id: &str,
        expected_version: u64,
        event: &str,
        new_state: &str,
        at_ns: i64,
    ) -> Result<u64, RepoError> {
        check_label(event)?;
        check_label(new_state)?;
        let expected = i64::try_from(expected_version).map_err(|_| RepoError::Other)?;
        let next = expected.checked_add(1).ok_or(RepoError::Other)?;
        let mut tx = self.pool.begin().await.map_err(|e| map_err(&e))?;
        let from: Option<String> =
            sqlx::query_scalar("SELECT state FROM orders WHERE order_id = ? AND version = ?")
                .bind(order_id)
                .bind(expected)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| map_err(&e))?;
        let Some(from) = from else {
            return Err(RepoError::Conflict);
        };
        let done = sqlx::query(
            "UPDATE orders SET state = ?, version = ?, updated_ns = ? \
             WHERE order_id = ? AND version = ?",
        )
        .bind(new_state)
        .bind(next)
        .bind(at_ns)
        .bind(order_id)
        .bind(expected)
        .execute(&mut *tx)
        .await
        .map_err(|e| map_err(&e))?;
        if done.rows_affected() != 1 {
            return Err(RepoError::Conflict);
        }
        sqlx::query(
            "INSERT INTO order_events (order_id, version, event, from_state, to_state, at_ns) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(order_id)
        .bind(next)
        .bind(event)
        .bind(from)
        .bind(new_state)
        .bind(at_ns)
        .execute(&mut *tx)
        .await
        .map_err(|e| map_err(&e))?;
        tx.commit().await.map_err(|e| map_err(&e))?;
        u64::try_from(next).map_err(|_| RepoError::Other)
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

    async fn store() -> SqliteStore {
        SqliteStore::open_in_memory().await.unwrap()
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
    async fn advance_bumps_version_and_logs_events() {
        let s = store().await;
        s.order_create("o1", "PendingApproval", "{}", 1)
            .await
            .unwrap();
        assert_eq!(
            s.order_advance("o1", 1, "approve", "Approved", 2).await,
            Ok(2)
        );
        assert_eq!(s.order_advance("o1", 2, "send", "Sent", 3).await, Ok(3));
        let ev = s.order_events("o1").await.unwrap();
        assert_eq!(ev.len(), 3);
        assert_eq!(
            ev[2],
            (3, "send".into(), Some("Approved".into()), "Sent".into())
        );
        assert_eq!(s.orders_in_state("Sent").await.unwrap().len(), 1);
        assert_eq!(s.orders_in_state("Approved").await.unwrap(), Vec::new());
    }

    #[tokio::test]
    async fn stale_version_loses_and_changes_nothing() {
        let s = store().await;
        s.order_create("o1", "PendingApproval", "{}", 1)
            .await
            .unwrap();
        s.order_advance("o1", 1, "approve", "Approved", 2)
            .await
            .unwrap();
        assert_eq!(
            s.order_advance("o1", 1, "decline", "Declined", 3).await,
            Err(RepoError::Conflict)
        );
        assert_eq!(
            s.order_advance("missing", 1, "x", "Y", 3).await,
            Err(RepoError::Conflict)
        );
        let o = s.order_get("o1").await.unwrap().unwrap();
        assert_eq!((o.state.as_str(), o.version), ("Approved", 2));
        assert_eq!(s.order_events("o1").await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn concurrent_advance_has_one_winner() {
        let s = store().await;
        s.order_create("o1", "PendingApproval", "{}", 1)
            .await
            .unwrap();
        let (a, b) = tokio::join!(
            s.order_advance("o1", 1, "approve", "Approved", 2),
            s.order_advance("o1", 1, "decline", "Declined", 2)
        );
        assert_eq!([a.is_ok(), b.is_ok()].iter().filter(|x| **x).count(), 1);
    }

    #[tokio::test]
    async fn events_are_append_only() {
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
    }

    #[tokio::test]
    async fn rejects_oversize_inputs() {
        let s = store().await;
        assert_eq!(
            s.order_create("", "A", "{}", 1).await,
            Err(RepoError::TooLarge)
        );
        let big = "x".repeat(MAX_PAYLOAD + 1);
        assert_eq!(
            s.order_create("o", "A", &big, 1).await,
            Err(RepoError::TooLarge)
        );
    }
}
