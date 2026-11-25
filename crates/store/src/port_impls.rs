//! Core port impls for `SqliteStore` over the inherent methods.
//!
//! Bridge rules (ports stay unchanged): the port order version is the store version minus 1
//! (port 0 = store 1); `intent_id` is stored as its decimal string; timestamps come from the
//! caller's RFC 3339 IST string (the store reads no clock); a store approval of `expired` has
//! no port meaning and reads as `Corrupt`.

use bazaar_core::ports::{
    ApprovalDecision, ApprovalRepo, AuditDraft, AuditRepo, AuditRow, ChainFn, KillSwitchRepo,
    LedgerEntry, OrderRecord, OrderRepo, PaperLedgerRepo, RepoError,
};
use chrono::DateTime;

use crate::approvals::ApprovalDecision as StoreDecision;
use crate::orders::{OrderRow, OrderTransition};
use crate::sqlite::SqliteStore;

/// The only order state an approval decision may be written from.
const PENDING_STATE: &str = "pending_approval";

fn ns_of(rfc3339: &str) -> Result<i64, RepoError> {
    DateTime::parse_from_rfc3339(rfc3339)
        .ok()
        .and_then(|t| t.timestamp_nanos_opt())
        .ok_or(RepoError::Other)
}

fn id_str(id: u128) -> String {
    id.to_string()
}

fn to_record(r: OrderRow) -> Result<OrderRecord, RepoError> {
    Ok(OrderRecord {
        intent_id: r.order_id.parse().map_err(|_| RepoError::Corrupt)?,
        version: u32::try_from(r.version.checked_sub(1).ok_or(RepoError::Corrupt)?)
            .map_err(|_| RepoError::Corrupt)?,
        state: r.state,
        payload: r.payload,
    })
}

impl AuditRepo for SqliteStore {
    async fn append(&self, draft: AuditDraft, chain: ChainFn) -> Result<AuditRow, RepoError> {
        self.audit_append(&draft, chain).await
    }

    async fn read_all(&self) -> Result<Vec<AuditRow>, RepoError> {
        self.audit_read_all().await
    }
}

impl OrderRepo for SqliteStore {
    async fn insert(
        &self,
        order: OrderRecord,
        audit: AuditDraft,
        chain: ChainFn,
    ) -> Result<(), RepoError> {
        if order.version != 0 {
            return Err(RepoError::Other);
        }
        let at = ns_of(&audit.ts_ist_rfc3339)?;
        self.order_create_audited(
            &id_str(order.intent_id),
            &order.state,
            &order.payload,
            at,
            &audit,
            chain,
        )
        .await
        .map(|_| ())
    }

    async fn get(&self, intent_id: u128) -> Result<Option<OrderRecord>, RepoError> {
        self.order_get(&id_str(intent_id))
            .await?
            .map(to_record)
            .transpose()
    }

    async fn advance(
        &self,
        intent_id: u128,
        expected_version: u32,
        new_state: String,
        event_kind: String,
        audit: AuditDraft,
        chain: ChainFn,
    ) -> Result<u32, RepoError> {
        let id = id_str(intent_id);
        let t = OrderTransition {
            order_id: &id,
            expected_version: u64::from(expected_version) + 1,
            event: &event_kind,
            new_state: &new_state,
            at_ns: ns_of(&audit.ts_ist_rfc3339)?,
        };
        let v = self.order_advance(&t, &audit, chain).await?;
        u32::try_from(v.checked_sub(1).ok_or(RepoError::Corrupt)?).map_err(|_| RepoError::Other)
    }

    async fn list_in_state(&self, state: &str, limit: u32) -> Result<Vec<OrderRecord>, RepoError> {
        self.orders_in_state_limit(state, i64::from(limit))
            .await?
            .into_iter()
            .map(to_record)
            .collect()
    }
}

impl ApprovalRepo for SqliteStore {
    /// `Conflict` does NOT prove an Approve exists: callers must re-read the order state before
    /// acting on it.
    #[allow(clippy::too_many_arguments)] // mirrors the port: one atomic write
    async fn decide_and_advance(
        &self,
        intent_id: u128,
        decision: ApprovalDecision,
        actor: &str,
        expected_version: u32,
        new_state: String,
        event_kind: String,
        audit: AuditDraft,
        chain: ChainFn,
    ) -> Result<u32, RepoError> {
        // The target must fit the decision (SEC-131-1): an approval can only reach `approved`.
        let (stored, allowed): (StoreDecision, &[&str]) = match decision {
            ApprovalDecision::Approve => (StoreDecision::Approved, &["approved"]),
            ApprovalDecision::Decline => {
                (StoreDecision::Declined, &["declined", "expired", "blocked"])
            }
        };
        if !allowed.contains(&new_state.as_str()) {
            return Err(RepoError::Conflict);
        }
        let id = id_str(intent_id);
        let t = OrderTransition {
            order_id: &id,
            expected_version: u64::from(expected_version) + 1,
            event: &event_kind,
            new_state: &new_state,
            at_ns: ns_of(&audit.ts_ist_rfc3339)?,
        };
        let v = self
            .approval_decide_from(&t, PENDING_STATE, stored, actor, &audit, chain)
            .await?;
        u32::try_from(v.checked_sub(1).ok_or(RepoError::Corrupt)?).map_err(|_| RepoError::Other)
    }

    async fn reconcile_stray_decisions(
        &self,
        ts_ist_rfc3339: &str,
        chain: ChainFn,
    ) -> Result<Vec<u128>, RepoError> {
        let at = ns_of(ts_ist_rfc3339)?;
        self.approvals_reconcile(PENDING_STATE, "expired", at, ts_ist_rfc3339, chain)
            .await?
            .iter()
            .map(|id| id.parse::<u128>().map_err(|_| RepoError::Corrupt))
            .collect()
    }
}

impl KillSwitchRepo for SqliteStore {
    async fn is_engaged(&self) -> Result<bool, RepoError> {
        Ok(self.kill_switch_get().await?.engaged)
    }

    async fn set(
        &self,
        engaged: bool,
        reason: &str,
        audit: AuditDraft,
        chain: ChainFn,
    ) -> Result<(), RepoError> {
        let at = ns_of(&audit.ts_ist_rfc3339)?;
        self.kill_switch_set_audited(engaged, &audit.actor, reason, at, &audit, chain)
            .await
    }
}

impl PaperLedgerRepo for SqliteStore {
    async fn append(&self, entry: LedgerEntry) -> Result<(), RepoError> {
        self.ledger_append(&entry).await
    }

    async fn append_many(&self, entries: Vec<LedgerEntry>) -> Result<(), RepoError> {
        self.ledger_append_many(&entries).await
    }

    async fn current_epoch(&self) -> Result<Vec<LedgerEntry>, RepoError> {
        self.ledger_current_epoch().await
    }

    async fn new_epoch(
        &self,
        opening: LedgerEntry,
        expected_tail: u64,
        audit: AuditDraft,
        chain: ChainFn,
    ) -> Result<u32, RepoError> {
        self.ledger_new_epoch(&opening, expected_tail, &audit, chain)
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use rust_decimal::Decimal;

    use super::*;
    use crate::audit::{test_chain, test_draft};

    async fn store() -> SqliteStore {
        SqliteStore::open_in_memory().await.unwrap()
    }

    fn rec(id: u128) -> OrderRecord {
        OrderRecord {
            intent_id: id,
            version: 0,
            state: "PendingApproval".into(),
            payload: "{}".into(),
        }
    }

    fn entry(epoch: u32, amt: &str) -> LedgerEntry {
        LedgerEntry {
            epoch,
            kind: "fill".into(),
            amount: Decimal::from_str(amt).unwrap(),
            ref_id: Some(7),
            ts_ist_rfc3339: "2026-11-22T09:15:00+05:30".into(),
        }
    }

    #[tokio::test]
    async fn order_insert_get_advance_use_port_versions() {
        let s = store().await;
        let id = u128::MAX;
        OrderRepo::insert(&s, rec(id), test_draft("o.insert"), test_chain)
            .await
            .unwrap();
        let got = OrderRepo::get(&s, id).await.unwrap().unwrap();
        assert_eq!((got.version, got.intent_id), (0, id));
        let v = OrderRepo::advance(
            &s,
            id,
            0,
            "Approved".into(),
            "approve".into(),
            test_draft("o.adv"),
            test_chain,
        )
        .await;
        assert_eq!(v, Ok(1));
        assert_eq!(
            OrderRepo::get(&s, id).await.unwrap().unwrap().state,
            "Approved"
        );
        assert_eq!(AuditRepo::read_all(&s).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn order_duplicate_stale_and_bad_inputs() {
        let s = store().await;
        OrderRepo::insert(&s, rec(1), test_draft("a"), test_chain)
            .await
            .unwrap();
        assert_eq!(
            OrderRepo::insert(&s, rec(1), test_draft("a"), test_chain).await,
            Err(RepoError::Conflict)
        );
        // The failed duplicate left no extra audit row.
        assert_eq!(AuditRepo::read_all(&s).await.unwrap().len(), 1);
        let stale = OrderRepo::advance(
            &s,
            1,
            5,
            "X".into(),
            "e".into(),
            test_draft("a"),
            test_chain,
        )
        .await;
        assert_eq!(stale, Err(RepoError::Conflict));
        let mut v1 = rec(2);
        v1.version = 1;
        assert_eq!(
            OrderRepo::insert(&s, v1, test_draft("a"), test_chain).await,
            Err(RepoError::Other)
        );
        let mut bad_ts = test_draft("a");
        bad_ts.ts_ist_rfc3339 = "nope".into();
        assert_eq!(
            OrderRepo::insert(&s, rec(3), bad_ts, test_chain).await,
            Err(RepoError::Other)
        );
        assert_eq!(OrderRepo::get(&s, 99).await, Ok(None));
    }

    #[tokio::test]
    async fn kill_switch_set_and_read_with_audit() {
        let s = store().await;
        assert_eq!(KillSwitchRepo::is_engaged(&s).await, Ok(false));
        KillSwitchRepo::set(
            &s,
            true,
            "daily loss",
            test_draft("kill.engage"),
            test_chain,
        )
        .await
        .unwrap();
        assert_eq!(KillSwitchRepo::is_engaged(&s).await, Ok(true));
        let k = s.kill_switch_get().await.unwrap();
        assert_eq!(
            k.reason, "daily loss",
            "typed reason, not audit.kind (RISK-129-1)"
        );
        KillSwitchRepo::set(
            &s,
            false,
            "released",
            test_draft("kill.release"),
            test_chain,
        )
        .await
        .unwrap();
        assert_eq!(KillSwitchRepo::is_engaged(&s).await, Ok(false));
        let rows = AuditRepo::read_all(&s).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].draft.kind, "kill.engage");
        assert_eq!(
            KillSwitchRepo::set(&s, true, "", test_draft("kill.engage"), test_chain).await,
            Err(RepoError::Other)
        );
        assert_eq!(KillSwitchRepo::is_engaged(&s).await, Ok(false));
    }

    #[tokio::test]
    async fn audit_append_returns_stored_row_and_chains() {
        let s = store().await;
        let a = AuditRepo::append(&s, test_draft("one"), test_chain)
            .await
            .unwrap();
        let b = AuditRepo::append(&s, test_draft("two"), test_chain)
            .await
            .unwrap();
        assert_eq!((a.seq, b.seq, b.prev_hash), (1, 2, a.hash));
        assert_eq!(AuditRepo::read_all(&s).await.unwrap(), vec![a, b]);
        s.audit_verify(test_chain).await.unwrap();
        let mut big = test_draft("x");
        big.payload = "x".repeat(70_000);
        assert_eq!(
            AuditRepo::append(&s, big, test_chain).await,
            Err(RepoError::TooLarge)
        );
    }

    #[tokio::test]
    async fn ledger_port_append_many_atomic_and_epochs() {
        let s = store().await;
        let opening = LedgerEntry {
            kind: "opening".into(),
            ..entry(1, "100000.50")
        };
        assert_eq!(
            PaperLedgerRepo::new_epoch(&s, opening, 0, test_draft("paper_reset"), test_chain).await,
            Ok(1)
        );
        // One bad epoch in the batch stores none of it.
        let r = PaperLedgerRepo::append_many(&s, vec![entry(1, "-1"), entry(9, "-2")]).await;
        assert_eq!(r, Err(RepoError::Conflict));
        PaperLedgerRepo::append_many(&s, vec![entry(1, "-1.25")])
            .await
            .unwrap();
        let cur = PaperLedgerRepo::current_epoch(&s).await.unwrap();
        assert_eq!(cur.len(), 2);
        assert_eq!(cur[1].amount, Decimal::from_str("-1.25").unwrap());
        let next = LedgerEntry {
            kind: "opening".into(),
            ..entry(2, "50")
        };
        assert_eq!(
            PaperLedgerRepo::new_epoch(&s, next, 2, test_draft("paper_reset"), test_chain).await,
            Ok(2)
        );
        let cur = PaperLedgerRepo::current_epoch(&s).await.unwrap();
        assert_eq!((cur.len(), cur[0].epoch), (1, 2));
        // The audit row landed in the same transaction and the chain verifies.
        let rows = AuditRepo::read_all(&s).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.draft.kind == "paper_reset"));
        s.audit_verify(test_chain).await.unwrap();
    }

    fn pending(id: u128) -> OrderRecord {
        OrderRecord {
            state: "pending_approval".into(),
            ..rec(id)
        }
    }

    async fn seeded(id: u128) -> SqliteStore {
        let s = store().await;
        OrderRepo::insert(&s, pending(id), test_draft("order.created"), test_chain)
            .await
            .unwrap();
        s
    }

    async fn dna(
        s: &SqliteStore,
        id: u128,
        d: ApprovalDecision,
        v: u32,
        to: &str,
    ) -> Result<u32, RepoError> {
        ApprovalRepo::decide_and_advance(
            s,
            id,
            d,
            "me",
            v,
            to.into(),
            "order.approve".into(),
            test_draft("approval"),
            test_chain,
        )
        .await
    }

    async fn count(s: &SqliteStore, table: &str) -> i64 {
        let sql = match table {
            "approvals" => "SELECT COUNT(*) FROM approvals",
            "audit_log" => "SELECT COUNT(*) FROM audit_log",
            "order_events" => "SELECT COUNT(*) FROM order_events",
            _ => "SELECT -1",
        };
        sqlx::query_scalar(sql).fetch_one(&s.pool).await.unwrap()
    }

    #[tokio::test]
    async fn decide_and_advance_writes_all_four_rows_and_returns_the_port_version() {
        let s = seeded(7).await;
        assert_eq!(
            dna(&s, 7, ApprovalDecision::Approve, 0, "approved").await,
            Ok(1)
        );
        let o = OrderRepo::get(&s, 7).await.unwrap().unwrap();
        assert_eq!((o.state.as_str(), o.version), ("approved", 1));
        assert_eq!(count(&s, "approvals").await, 1);
        assert_eq!(count(&s, "order_events").await, 2);
        assert_eq!(count(&s, "audit_log").await, 2);
    }

    #[tokio::test]
    async fn decide_and_advance_stores_the_actor_on_the_approval_row_and_the_audit_row() {
        let s = seeded(7).await;
        let mut draft = test_draft("approval");
        draft.actor = "alice".into();
        ApprovalRepo::decide_and_advance(
            &s,
            7,
            ApprovalDecision::Approve,
            "alice",
            0,
            "approved".into(),
            "order.approve".into(),
            draft,
            test_chain,
        )
        .await
        .unwrap();
        let approval: String = sqlx::query_scalar("SELECT actor FROM approvals")
            .fetch_one(&s.pool)
            .await
            .unwrap();
        let audit: String =
            sqlx::query_scalar("SELECT actor FROM audit_log ORDER BY seq DESC LIMIT 1")
                .fetch_one(&s.pool)
                .await
                .unwrap();
        assert_eq!((approval.as_str(), audit.as_str()), ("alice", "alice"));
    }

    #[tokio::test]
    async fn reconcile_stray_decisions_expires_a_pending_order_with_a_decision_row() {
        let s = seeded(7).await;
        OrderRepo::insert(&s, pending(8), test_draft("order.created"), test_chain)
            .await
            .unwrap();
        sqlx::query("INSERT INTO approvals (order_id, decision, actor, decided_ns) VALUES ('7', 'approved', 'a', 1)")
            .execute(&s.pool)
            .await
            .unwrap();
        let audits_before = count(&s, "audit_log").await;
        let ids =
            ApprovalRepo::reconcile_stray_decisions(&s, "2026-11-22T09:15:00+05:30", test_chain)
                .await
                .unwrap();
        assert_eq!(ids, vec![7]);
        assert_eq!(count(&s, "audit_log").await, audits_before + 1);
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM audit_log WHERE kind = 'order.expired_inconsistent' AND payload LIKE '%7%'",
        )
        .fetch_one(&s.pool)
        .await
        .unwrap();
        assert_eq!(n, 1);
        assert_eq!(
            OrderRepo::get(&s, 7).await.unwrap().unwrap().state,
            "expired"
        );
        assert_eq!(
            OrderRepo::get(&s, 8).await.unwrap().unwrap().state,
            "pending_approval"
        );
    }

    #[tokio::test]
    async fn decide_and_advance_refuses_terminal_and_approved_orders_and_writes_nothing() {
        for state in ["approved", "declined", "expired", "blocked", "placed"] {
            let s = seeded(7).await;
            // Move the order out of pending_approval with a plain advance.
            OrderRepo::advance(
                &s,
                7,
                0,
                state.into(),
                "test.move".into(),
                test_draft("move"),
                test_chain,
            )
            .await
            .unwrap();
            let (ev, au) = (
                count(&s, "order_events").await,
                count(&s, "audit_log").await,
            );
            assert_eq!(
                dna(&s, 7, ApprovalDecision::Approve, 1, "approved").await,
                Err(RepoError::Conflict),
                "{state}"
            );
            assert_eq!(count(&s, "approvals").await, 0, "{state}");
            assert_eq!(count(&s, "order_events").await, ev, "{state}");
            assert_eq!(count(&s, "audit_log").await, au, "{state}");
            let o = OrderRepo::get(&s, 7).await.unwrap().unwrap();
            assert_eq!(o.state, state);
        }
    }

    #[tokio::test]
    async fn decide_and_advance_missing_order_and_stale_version_are_refused() {
        let s = seeded(7).await;
        assert_eq!(
            dna(&s, 99, ApprovalDecision::Approve, 0, "approved").await,
            Err(RepoError::Conflict)
        );
        assert_eq!(
            dna(&s, 7, ApprovalDecision::Approve, 5, "approved").await,
            Err(RepoError::Conflict)
        );
        assert_eq!(count(&s, "approvals").await, 0);
        let o = OrderRepo::get(&s, 7).await.unwrap().unwrap();
        assert_eq!((o.state.as_str(), o.version), ("pending_approval", 0));
    }

    #[tokio::test]
    async fn decide_and_advance_first_wins_and_a_second_decision_changes_nothing() {
        let s = seeded(7).await;
        assert_eq!(
            dna(&s, 7, ApprovalDecision::Approve, 0, "approved").await,
            Ok(1)
        );
        let (ev, au) = (
            count(&s, "order_events").await,
            count(&s, "audit_log").await,
        );
        assert_eq!(
            dna(&s, 7, ApprovalDecision::Decline, 1, "declined").await,
            Err(RepoError::Conflict)
        );
        assert_eq!(count(&s, "approvals").await, 1);
        assert_eq!(count(&s, "order_events").await, ev);
        assert_eq!(count(&s, "audit_log").await, au);
        let o = OrderRepo::get(&s, 7).await.unwrap().unwrap();
        assert_eq!(o.state, "approved");
    }

    #[tokio::test]
    async fn decide_and_advance_audit_failure_rolls_back_approval_and_advance() {
        let s = seeded(7).await;
        sqlx::query(
            "CREATE TRIGGER boom BEFORE INSERT ON audit_log \
             WHEN NEW.kind = 'approval' BEGIN SELECT RAISE(ABORT, 'boom'); END",
        )
        .execute(&s.pool)
        .await
        .unwrap();
        assert!(
            dna(&s, 7, ApprovalDecision::Approve, 0, "approved")
                .await
                .is_err()
        );
        assert_eq!(count(&s, "approvals").await, 0);
        assert_eq!(count(&s, "order_events").await, 1);
        assert_eq!(count(&s, "audit_log").await, 1);
        let o = OrderRepo::get(&s, 7).await.unwrap().unwrap();
        assert_eq!((o.state.as_str(), o.version), ("pending_approval", 0));
    }

    #[tokio::test]
    async fn decide_and_advance_failure_between_approval_row_and_advance_rolls_back() {
        let s = seeded(7).await;
        sqlx::query(
            "CREATE TRIGGER boom BEFORE INSERT ON order_events \
             WHEN NEW.event = 'order.approve' BEGIN SELECT RAISE(ABORT, 'boom'); END",
        )
        .execute(&s.pool)
        .await
        .unwrap();
        assert!(
            dna(&s, 7, ApprovalDecision::Approve, 0, "approved")
                .await
                .is_err()
        );
        assert_eq!(count(&s, "approvals").await, 0);
        assert_eq!(count(&s, "order_events").await, 1);
        assert_eq!(count(&s, "audit_log").await, 1);
        let o = OrderRepo::get(&s, 7).await.unwrap().unwrap();
        assert_eq!((o.state.as_str(), o.version), ("pending_approval", 0));
    }

    #[tokio::test]
    async fn decide_and_advance_target_state_must_fit_the_decision() {
        let s = seeded(7).await;
        for (d, to) in [
            (ApprovalDecision::Approve, "placed"),
            (ApprovalDecision::Approve, "declined"),
            (ApprovalDecision::Decline, "approved"),
            (ApprovalDecision::Decline, "placed"),
        ] {
            assert_eq!(dna(&s, 7, d, 0, to).await, Err(RepoError::Conflict), "{to}");
        }
        assert_eq!(count(&s, "approvals").await, 0);
        assert_eq!(count(&s, "order_events").await, 1);
        assert_eq!(
            dna(&s, 7, ApprovalDecision::Decline, 0, "expired").await,
            Ok(1)
        );
    }

    #[tokio::test]
    async fn duplicate_approval_row_is_a_conflict_through_the_store_method() {
        let s = seeded(7).await;
        let id = id_str(7);
        s.approval_record(&id, StoreDecision::Approved, "a", 1)
            .await
            .unwrap();
        assert_eq!(
            s.approval_record(&id, StoreDecision::Declined, "b", 2)
                .await,
            Err(RepoError::Conflict)
        );
    }

    #[tokio::test]
    async fn list_in_state_is_oldest_first_limited_and_never_truncates_silently() {
        let s = store().await;
        // Creation time orders the list (then id): ids 3, 1, 2 created at 09:15, 09:16, 09:17.
        for (id, minute) in [(3u128, 15), (1, 16), (2, 17)] {
            let mut d = test_draft("order.created");
            d.ts_ist_rfc3339 = format!("2026-11-22T09:{minute}:00+05:30");
            OrderRepo::insert(&s, pending(id), d, test_chain)
                .await
                .unwrap();
        }
        OrderRepo::insert(&s, rec(9), test_draft("order.created"), test_chain)
            .await
            .unwrap();
        let ids: Vec<u128> = OrderRepo::list_in_state(&s, "pending_approval", 10)
            .await
            .unwrap()
            .iter()
            .map(|o| o.intent_id)
            .collect();
        assert_eq!(ids, vec![3, 1, 2], "oldest (earliest created) first");
        assert_eq!(
            OrderRepo::list_in_state(&s, "pending_approval", 2).await,
            Err(RepoError::TooLarge)
        );
        assert_eq!(
            OrderRepo::list_in_state(&s, "nothing", 2).await,
            Ok(Vec::new())
        );
    }
}
