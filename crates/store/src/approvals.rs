//! Approval decisions. The rule (F10 record, slice 6): a decision, its order transition, the
//! order event and the audit row are ONE transaction (`approval_decide`); any failure rolls
//! all of it back. The unique index on `order_id` makes the first decision win; a second one
//! gets `RepoError::Conflict` and the caller reads the stored outcome with `approval_get`.
//! Startup guard (`approvals_reconcile`): an approval row whose order is still pending cannot
//! happen now; if it is ever seen it is never re-presented, it is expired with an audit row.

use bazaar_core::ports::{AuditDraft, ChainFn, RepoError};
use sqlx::Row;

use crate::audit::append_in_tx;
use crate::orders::{OrderTransition, advance_in_tx, check_label, conflict_or};
use crate::sqlite::{SqliteStore, map_err};

const MAX_ACTOR: usize = 500;

/// A recorded decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    Approved,
    Declined,
    Expired,
}

impl ApprovalDecision {
    const fn label(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Declined => "declined",
            Self::Expired => "expired",
        }
    }

    fn parse(s: &str) -> Result<Self, RepoError> {
        match s {
            "approved" => Ok(Self::Approved),
            "declined" => Ok(Self::Declined),
            "expired" => Ok(Self::Expired),
            _ => Err(RepoError::Corrupt),
        }
    }
}

/// One stored approval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRow {
    pub order_id: String,
    pub decision: ApprovalDecision,
    pub actor: String,
    pub decided_ns: i64,
}

fn check_actor(actor: &str) -> Result<(), RepoError> {
    if actor.is_empty() {
        return Err(RepoError::Other);
    }
    if actor.len() > MAX_ACTOR {
        return Err(RepoError::TooLarge);
    }
    Ok(())
}

const INSERT: &str =
    "INSERT INTO approvals (order_id, decision, actor, decided_ns) VALUES (?, ?, ?, ?)";

/// JSON string escape for the guard's audit payload (ids are caller labels).
fn json_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if u32::from(c) < 0x20 => {
                use std::fmt::Write as _;
                let _ = write!(o, "\\u{:04x}", u32::from(c));
            }
            c => o.push(c),
        }
    }
    o
}

/// A unique violation, or a busy/snapshot failure on the first write (another writer committed
/// between our state read and our write: we lost the race), is `Conflict`.
fn lost_race_or_conflict(e: &sqlx::Error) -> RepoError {
    if let sqlx::Error::Database(d) = e
        && let Some(code) = d.code()
        && matches!(code.as_ref(), "5" | "6" | "517" | "261" | "773")
    {
        return RepoError::Conflict;
    }
    conflict_or(e)
}

impl SqliteStore {
    /// Test-only: the decision write without the order-state guard. Production code must use
    /// `approval_decide_from` (RISK-131-3).
    #[cfg(test)]
    pub(crate) async fn approval_decide(
        &self,
        t: &OrderTransition<'_>,
        decision: ApprovalDecision,
        actor: &str,
        audit: &AuditDraft,
        chain: ChainFn,
    ) -> Result<u64, RepoError> {
        self.decide_tx(t, None, decision, actor, audit, chain).await
    }

    /// Record the decision, move the order, write its event and the audit row in ONE
    /// transaction, and only while the order is in `required_state` (checked inside the
    /// transaction). `Conflict` when the state differs, a decision already exists, the order
    /// version is stale, or a concurrent writer won the race; on any error nothing is stored.
    /// Returns the new order version.
    pub(crate) async fn approval_decide_from(
        &self,
        t: &OrderTransition<'_>,
        required_state: &str,
        decision: ApprovalDecision,
        actor: &str,
        audit: &AuditDraft,
        chain: ChainFn,
    ) -> Result<u64, RepoError> {
        self.decide_tx(t, Some(required_state), decision, actor, audit, chain)
            .await
    }

    async fn decide_tx(
        &self,
        t: &OrderTransition<'_>,
        required_state: Option<&str>,
        decision: ApprovalDecision,
        actor: &str,
        audit: &AuditDraft,
        chain: ChainFn,
    ) -> Result<u64, RepoError> {
        check_label(t.order_id)?;
        check_actor(actor)?;
        let mut tx = self.pool.begin().await.map_err(|e| map_err(&e))?;
        if let Some(req) = required_state {
            let state: Option<String> =
                sqlx::query_scalar("SELECT state FROM orders WHERE order_id = ?")
                    .bind(t.order_id)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(|e| map_err(&e))?;
            if state.as_deref() != Some(req) {
                return Err(RepoError::Conflict);
            }
        }
        sqlx::query(INSERT)
            .bind(t.order_id)
            .bind(decision.label())
            .bind(actor)
            .bind(t.at_ns)
            .execute(&mut *tx)
            .await
            .map_err(|e| lost_race_or_conflict(&e))?;
        let v = advance_in_tx(&mut tx, t).await?;
        append_in_tx(&mut tx, audit, chain).await?;
        tx.commit().await.map_err(|e| map_err(&e))?;
        Ok(v)
    }

    /// Test-only: the bare insert, so the unique index and CHECK can be exercised alone.
    #[cfg(test)]
    pub(crate) async fn approval_record(
        &self,
        order_id: &str,
        decision: ApprovalDecision,
        actor: &str,
        at_ns: i64,
    ) -> Result<(), RepoError> {
        check_label(order_id)?;
        check_actor(actor)?;
        sqlx::query(INSERT)
            .bind(order_id)
            .bind(decision.label())
            .bind(actor)
            .bind(at_ns)
            .execute(&self.pool)
            .await
            .map_err(|e| conflict_or(&e))?;
        Ok(())
    }

    /// Bare first-wins decision insert (port `decide`): no order transition, no audit row.
    pub(crate) async fn approval_insert(
        &self,
        order_id: &str,
        decision: ApprovalDecision,
        actor: &str,
        at_ns: i64,
    ) -> Result<(), RepoError> {
        check_label(order_id)?;
        check_actor(actor)?;
        sqlx::query(INSERT)
            .bind(order_id)
            .bind(decision.label())
            .bind(actor)
            .bind(at_ns)
            .execute(&self.pool)
            .await
            .map_err(|e| conflict_or(&e))?;
        Ok(())
    }

    /// The decision for an order, if any.
    pub async fn approval_get(&self, order_id: &str) -> Result<Option<ApprovalRow>, RepoError> {
        let row = sqlx::query(
            "SELECT order_id, decision, actor, decided_ns FROM approvals WHERE order_id = ?",
        )
        .bind(order_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| map_err(&e))?;
        let g = |e: sqlx::Error| map_err(&e);
        row.map(|r| {
            let d: String = r.try_get("decision").map_err(g)?;
            Ok(ApprovalRow {
                order_id: r.try_get("order_id").map_err(g)?,
                decision: ApprovalDecision::parse(&d)?,
                actor: r.try_get("actor").map_err(g)?,
                decided_ns: r.try_get("decided_ns").map_err(g)?,
            })
        })
        .transpose()
    }

    /// Startup guard, run before pending orders are reloaded. Every order still in
    /// `pending_state` that already has an approval row is inconsistent: it is moved to
    /// `expired_state` (event `expired_inconsistent`) with an audit row, in one transaction
    /// per order, and its id is returned. It is never re-presented for approval.
    pub async fn approvals_reconcile(
        &self,
        pending_state: &str,
        expired_state: &str,
        at_ns: i64,
        ts_ist_rfc3339: &str,
        chain: ChainFn,
    ) -> Result<Vec<String>, RepoError> {
        let rows = sqlx::query(
            "SELECT o.order_id, o.version FROM orders o \
             JOIN approvals a ON a.order_id = o.order_id \
             WHERE o.state = ? ORDER BY o.created_ns, o.order_id",
        )
        .bind(pending_state)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| map_err(&e))?;
        let mut fixed = Vec::with_capacity(rows.len());
        for r in rows {
            let id: String = r.try_get("order_id").map_err(|e| map_err(&e))?;
            let ver: i64 = r.try_get("version").map_err(|e| map_err(&e))?;
            let t = OrderTransition {
                order_id: &id,
                expected_version: u64::try_from(ver).map_err(|_| RepoError::Corrupt)?,
                event: "expired_inconsistent",
                new_state: expired_state,
                at_ns,
            };
            let draft = AuditDraft {
                ts_ist_rfc3339: ts_ist_rfc3339.to_owned(),
                actor: "system".to_owned(),
                kind: "order.expired_inconsistent".to_owned(),
                payload: format!(
                    "{{\"order_id\":\"{}\",\"reason\":\"approval_row_without_transition\"}}",
                    json_escape(&id)
                ),
            };
            let mut tx = self.pool.begin().await.map_err(|e| map_err(&e))?;
            advance_in_tx(&mut tx, &t).await?;
            append_in_tx(&mut tx, &draft, chain).await?;
            tx.commit().await.map_err(|e| map_err(&e))?;
            fixed.push(id);
        }
        Ok(fixed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::{test_chain, test_draft};

    async fn store_with_order() -> SqliteStore {
        let s = SqliteStore::open_in_memory().await.unwrap();
        s.order_create("o1", "PendingApproval", "{}", 1)
            .await
            .unwrap();
        s
    }

    fn tr<'a>(id: &'a str, ver: u64, event: &'static str, to: &'static str) -> OrderTransition<'a> {
        OrderTransition {
            order_id: id,
            expected_version: ver,
            event,
            new_state: to,
            at_ns: 5,
        }
    }

    async fn count(s: &SqliteStore, table: &str) -> i64 {
        let sql = match table {
            "approvals" => "SELECT COUNT(*) FROM approvals",
            "audit_log" => "SELECT COUNT(*) FROM audit_log",
            _ => "SELECT -1",
        };
        sqlx::query_scalar(sql).fetch_one(&s.pool).await.unwrap()
    }

    #[tokio::test]
    async fn decide_is_atomic_across_approval_order_event_audit() {
        let s = store_with_order().await;
        let v = s
            .approval_decide(
                &tr("o1", 1, "approve", "Approved"),
                ApprovalDecision::Approved,
                "me",
                &test_draft("approval"),
                test_chain,
            )
            .await
            .unwrap();
        assert_eq!(v, 2);
        let a = s.approval_get("o1").await.unwrap().unwrap();
        assert_eq!(
            (a.decision, a.actor.as_str()),
            (ApprovalDecision::Approved, "me")
        );
        assert_eq!(s.order_get("o1").await.unwrap().unwrap().state, "Approved");
        assert_eq!(s.order_events("o1").await.unwrap().len(), 2);
        assert_eq!(count(&s, "audit_log").await, 1);
    }

    #[tokio::test]
    async fn failed_advance_leaves_no_approval_row_and_order_pending() {
        let s = store_with_order().await;
        sqlx::query(
            "CREATE TRIGGER boom BEFORE INSERT ON order_events WHEN NEW.event = 'boom' \
             BEGIN SELECT RAISE(ABORT, 'boom'); END",
        )
        .execute(&s.pool)
        .await
        .unwrap();
        let r = s
            .approval_decide(
                &tr("o1", 1, "boom", "Approved"),
                ApprovalDecision::Approved,
                "me",
                &test_draft("approval"),
                test_chain,
            )
            .await;
        assert!(r.is_err());
        assert_eq!(s.approval_get("o1").await.unwrap(), None);
        let o = s.order_get("o1").await.unwrap().unwrap();
        assert_eq!((o.state.as_str(), o.version), ("PendingApproval", 1));
        assert_eq!(count(&s, "audit_log").await, 0);
    }

    #[tokio::test]
    async fn stale_version_leaves_no_approval_row() {
        let s = store_with_order().await;
        let r = s
            .approval_decide(
                &tr("o1", 9, "approve", "Approved"),
                ApprovalDecision::Approved,
                "me",
                &test_draft("approval"),
                test_chain,
            )
            .await;
        assert_eq!(r, Err(RepoError::Conflict));
        assert_eq!(count(&s, "approvals").await, 0);
    }

    #[tokio::test]
    async fn second_decision_conflicts_and_changes_nothing() {
        let s = store_with_order().await;
        s.approval_decide(
            &tr("o1", 1, "approve", "Approved"),
            ApprovalDecision::Approved,
            "me",
            &test_draft("approval"),
            test_chain,
        )
        .await
        .unwrap();
        let r = s
            .approval_decide(
                &tr("o1", 2, "decline", "Declined"),
                ApprovalDecision::Declined,
                "me",
                &test_draft("approval"),
                test_chain,
            )
            .await;
        assert_eq!(r, Err(RepoError::Conflict));
        let a = s.approval_get("o1").await.unwrap().unwrap();
        assert_eq!(a.decision, ApprovalDecision::Approved);
        let o = s.order_get("o1").await.unwrap().unwrap();
        assert_eq!((o.state.as_str(), o.version), ("Approved", 2));
        assert_eq!(count(&s, "audit_log").await, 1);
    }

    #[tokio::test]
    async fn concurrent_decisions_have_one_winner() {
        let s = store_with_order().await;
        let (t1, t2) = (
            tr("o1", 1, "approve", "Approved"),
            tr("o1", 1, "approve", "Approved"),
        );
        let d = test_draft("approval");
        let (a, b) = tokio::join!(
            s.approval_decide(&t1, ApprovalDecision::Approved, "a", &d, test_chain),
            s.approval_decide(&t2, ApprovalDecision::Approved, "b", &d, test_chain)
        );
        // Note: the pool has max_connections(1), so join! serialises the two calls; the loser
        // fails on the unique index / stale version, not on real write contention.
        let (ok, lost) = if a.is_ok() { (a, b) } else { (b, a) };
        assert_eq!(ok, Ok(2));
        assert_eq!(lost, Err(RepoError::Conflict));
        assert_eq!(count(&s, "approvals").await, 1);
    }

    #[tokio::test]
    async fn bare_insert_unique_check_and_bad_label() {
        let s = store_with_order().await;
        s.approval_record("o1", ApprovalDecision::Approved, "me", 5)
            .await
            .unwrap();
        assert_eq!(
            s.approval_record("o1", ApprovalDecision::Declined, "me", 6)
                .await,
            Err(RepoError::Conflict)
        );
        assert_eq!(s.approval_get("o2").await.unwrap(), None);
        // The CHECK constraint refuses a decision label outside the enum.
        s.order_create("o2", "P", "{}", 1).await.unwrap();
        assert!(
            sqlx::query("INSERT INTO approvals (order_id, decision, actor, decided_ns) VALUES ('o2', 'maybe', 'me', 1)")
                .execute(&s.pool)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn empty_actor_long_ids_and_unknown_order_refused() {
        let s = store_with_order().await;
        assert!(
            s.approval_record("ghost", ApprovalDecision::Approved, "me", 1)
                .await
                .is_err()
        );
        assert_eq!(
            s.approval_record("o1", ApprovalDecision::Approved, "", 1)
                .await,
            Err(RepoError::Other)
        );
        let long = "a".repeat(MAX_ACTOR + 1);
        assert_eq!(
            s.approval_record("o1", ApprovalDecision::Approved, &long, 1)
                .await,
            Err(RepoError::TooLarge)
        );
        let id65 = "i".repeat(65);
        assert_eq!(
            s.approval_record(&id65, ApprovalDecision::Approved, "me", 1)
                .await,
            Err(RepoError::TooLarge)
        );
        assert_eq!(
            s.approval_decide(
                &tr(&id65, 1, "e", "S"),
                ApprovalDecision::Approved,
                "me",
                &test_draft("a"),
                test_chain
            )
            .await,
            Err(RepoError::TooLarge)
        );
    }

    #[tokio::test]
    async fn approvals_are_append_only() {
        let s = store_with_order().await;
        s.approval_record("o1", ApprovalDecision::Declined, "me", 5)
            .await
            .unwrap();
        assert!(
            sqlx::query("UPDATE approvals SET decision='approved'")
                .execute(&s.pool)
                .await
                .is_err()
        );
        assert!(
            sqlx::query("DELETE FROM approvals")
                .execute(&s.pool)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn reconcile_expires_pending_order_with_approval_row_and_audits() {
        let s = store_with_order().await;
        s.order_create("o2", "PendingApproval", "{}", 2)
            .await
            .unwrap();
        // Impossible through approval_decide; forged to prove the guard fails closed.
        s.approval_record("o1", ApprovalDecision::Approved, "me", 3)
            .await
            .unwrap();
        let fixed = s
            .approvals_reconcile(
                "PendingApproval",
                "Expired",
                9,
                "2026-11-22T09:00:00+05:30",
                test_chain,
            )
            .await
            .unwrap();
        assert_eq!(fixed, ["o1"]);
        assert_eq!(s.order_get("o1").await.unwrap().unwrap().state, "Expired");
        assert_eq!(
            s.order_get("o2").await.unwrap().unwrap().state,
            "PendingApproval"
        );
        assert_eq!(count(&s, "audit_log").await, 1);
        let again = s
            .approvals_reconcile("PendingApproval", "Expired", 10, "t", test_chain)
            .await
            .unwrap();
        assert_eq!(again, Vec::<String>::new());
    }

    #[tokio::test]
    async fn failing_audit_insert_as_last_write_rolls_back_decide() {
        let s = store_with_order().await;
        sqlx::query(
            "CREATE TRIGGER boom BEFORE INSERT ON audit_log \
             BEGIN SELECT RAISE(ABORT, 'boom'); END",
        )
        .execute(&s.pool)
        .await
        .unwrap();
        let r = s
            .approval_decide(
                &tr("o1", 1, "approve", "Approved"),
                ApprovalDecision::Approved,
                "me",
                &test_draft("approval"),
                test_chain,
            )
            .await;
        assert!(r.is_err());
        assert_eq!(count(&s, "approvals").await, 0);
        let o = s.order_get("o1").await.unwrap().unwrap();
        assert_eq!((o.state.as_str(), o.version), ("PendingApproval", 1));
        assert_eq!(s.order_events("o1").await.unwrap().len(), 1);
        assert_eq!(count(&s, "audit_log").await, 0);
    }

    #[tokio::test]
    async fn failing_audit_insert_rolls_back_reconcile() {
        let s = store_with_order().await;
        s.approval_record("o1", ApprovalDecision::Approved, "me", 3)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TRIGGER boom BEFORE INSERT ON audit_log \
             BEGIN SELECT RAISE(ABORT, 'boom'); END",
        )
        .execute(&s.pool)
        .await
        .unwrap();
        let r = s
            .approvals_reconcile("PendingApproval", "Expired", 9, "t", test_chain)
            .await;
        assert!(r.is_err());
        let o = s.order_get("o1").await.unwrap().unwrap();
        assert_eq!((o.state.as_str(), o.version), ("PendingApproval", 1));
        assert_eq!(s.order_events("o1").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn audit_chain_links_across_callers() {
        let s = store_with_order().await;
        s.order_create("o2", "PendingApproval", "{}", 2)
            .await
            .unwrap();
        s.order_create("o3", "PendingApproval", "{}", 3)
            .await
            .unwrap();
        // Three different writers: decide, plain advance, reconcile.
        s.approval_decide(
            &tr("o1", 1, "approve", "Approved"),
            ApprovalDecision::Approved,
            "me",
            &test_draft("approval"),
            test_chain,
        )
        .await
        .unwrap();
        s.order_advance(
            &tr("o2", 1, "cancel", "Cancelled"),
            &test_draft("order"),
            test_chain,
        )
        .await
        .unwrap();
        s.approval_record("o3", ApprovalDecision::Declined, "me", 4)
            .await
            .unwrap();
        s.approvals_reconcile("PendingApproval", "Expired", 9, "t", test_chain)
            .await
            .unwrap();
        let rows: Vec<(i64, Vec<u8>, Vec<u8>)> =
            sqlx::query_as("SELECT seq, prev_hash, hash FROM audit_log ORDER BY seq")
                .fetch_all(&s.pool)
                .await
                .unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].1, vec![0_u8; 32]);
        for (i, r) in rows.iter().enumerate() {
            assert_eq!(r.0, i64::try_from(i).unwrap() + 1);
            if i > 0 {
                assert_eq!(r.1, rows[i - 1].2);
            }
        }
    }

    #[tokio::test]
    async fn reconcile_handles_several_orders_including_declined() {
        let s = store_with_order().await;
        s.order_create("o2", "PendingApproval", "{}", 2)
            .await
            .unwrap();
        s.order_create("o3", "PendingApproval", "{}", 3)
            .await
            .unwrap();
        s.approval_record("o1", ApprovalDecision::Approved, "me", 4)
            .await
            .unwrap();
        s.approval_record("o3", ApprovalDecision::Declined, "me", 5)
            .await
            .unwrap();
        let fixed = s
            .approvals_reconcile("PendingApproval", "Expired", 9, "t", test_chain)
            .await
            .unwrap();
        assert_eq!(fixed, ["o1", "o3"]);
        assert_eq!(
            s.order_get("o2").await.unwrap().unwrap().state,
            "PendingApproval"
        );
        assert_eq!(count(&s, "audit_log").await, 2);
    }

    #[test]
    fn json_escape_handles_quotes_and_controls() {
        assert_eq!(json_escape("a\"b\\c\n"), "a\\\"b\\\\c\\u000a");
    }
}
