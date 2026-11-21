//! Approval decisions: one row per order, enforced by a unique index, so of two concurrent
//! decisions the first wins and the second gets `RepoError::Conflict` (the caller then reads
//! the stored outcome with [`SqliteStore::approval_get`]).

use bazaar_core::ports::RepoError;
use sqlx::Row;

use crate::orders::conflict_or;
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

impl SqliteStore {
    /// Record the decision for `order_id`. A second decision for the same order is `Conflict`.
    pub async fn approval_record(
        &self,
        order_id: &str,
        decision: ApprovalDecision,
        actor: &str,
        at_ns: i64,
    ) -> Result<(), RepoError> {
        if actor.len() > MAX_ACTOR {
            return Err(RepoError::TooLarge);
        }
        sqlx::query(
            "INSERT INTO approvals (order_id, decision, actor, decided_ns) VALUES (?, ?, ?, ?)",
        )
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
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store_with_order() -> SqliteStore {
        let s = SqliteStore::open_in_memory().await.unwrap();
        s.order_create("o1", "PendingApproval", "{}", 1)
            .await
            .unwrap();
        s
    }

    #[tokio::test]
    async fn first_decision_wins() {
        let s = store_with_order().await;
        s.approval_record("o1", ApprovalDecision::Approved, "me", 5)
            .await
            .unwrap();
        assert_eq!(
            s.approval_record("o1", ApprovalDecision::Declined, "me", 6)
                .await,
            Err(RepoError::Conflict)
        );
        let a = s.approval_get("o1").await.unwrap().unwrap();
        assert_eq!((a.decision, a.decided_ns), (ApprovalDecision::Approved, 5));
        assert_eq!(s.approval_get("o2").await.unwrap(), None);
    }

    #[tokio::test]
    async fn concurrent_approvals_have_one_winner() {
        let s = store_with_order().await;
        let (a, b) = tokio::join!(
            s.approval_record("o1", ApprovalDecision::Approved, "a", 5),
            s.approval_record("o1", ApprovalDecision::Approved, "b", 5)
        );
        assert_eq!([a.is_ok(), b.is_ok()].iter().filter(|x| **x).count(), 1);
    }

    #[tokio::test]
    async fn unknown_order_and_oversize_actor_refused() {
        let s = store_with_order().await;
        assert!(
            s.approval_record("ghost", ApprovalDecision::Approved, "me", 1)
                .await
                .is_err()
        );
        let long = "a".repeat(MAX_ACTOR + 1);
        assert_eq!(
            s.approval_record("o1", ApprovalDecision::Approved, &long, 1)
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
}
