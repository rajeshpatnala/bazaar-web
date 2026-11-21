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

/// Actor recorded on a port approval row (the port carries none; the audit row has the user).
const APPROVAL_ACTOR: &str = "user";

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
}

impl ApprovalRepo for SqliteStore {
    async fn decide(
        &self,
        intent_id: u128,
        decision: ApprovalDecision,
        at_ist_rfc3339: String,
    ) -> Result<ApprovalDecision, RepoError> {
        let id = id_str(intent_id);
        let stored = match decision {
            ApprovalDecision::Approve => StoreDecision::Approved,
            ApprovalDecision::Decline => StoreDecision::Declined,
        };
        let at = ns_of(&at_ist_rfc3339)?;
        match self.approval_insert(&id, stored, APPROVAL_ACTOR, at).await {
            Ok(()) => Ok(decision),
            Err(RepoError::Conflict) => {
                let row = self.approval_get(&id).await?.ok_or(RepoError::Corrupt)?;
                match row.decision {
                    StoreDecision::Approved => Ok(ApprovalDecision::Approve),
                    StoreDecision::Declined => Ok(ApprovalDecision::Decline),
                    StoreDecision::Expired => Err(RepoError::Corrupt),
                }
            }
            Err(e) => Err(e),
        }
    }
}

impl KillSwitchRepo for SqliteStore {
    async fn is_engaged(&self) -> Result<bool, RepoError> {
        Ok(self.kill_switch_get().await?.engaged)
    }

    async fn set(
        &self,
        engaged: bool,
        audit: AuditDraft,
        chain: ChainFn,
    ) -> Result<(), RepoError> {
        let at = ns_of(&audit.ts_ist_rfc3339)?;
        self.kill_switch_set_audited(engaged, &audit.actor, &audit.kind, at, &audit, chain)
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

    async fn new_epoch(&self, opening: LedgerEntry) -> Result<u32, RepoError> {
        self.ledger_new_epoch(&opening).await
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
    async fn approval_first_wins_and_returns_standing_decision() {
        let s = store().await;
        OrderRepo::insert(&s, rec(1), test_draft("a"), test_chain)
            .await
            .unwrap();
        let at = "2026-11-22T09:16:00+05:30".to_owned();
        assert_eq!(
            ApprovalRepo::decide(&s, 1, ApprovalDecision::Approve, at.clone()).await,
            Ok(ApprovalDecision::Approve)
        );
        assert_eq!(
            ApprovalRepo::decide(&s, 1, ApprovalDecision::Decline, at).await,
            Ok(ApprovalDecision::Approve)
        );
        assert_eq!(
            ApprovalRepo::decide(&s, 1, ApprovalDecision::Approve, "bad".into()).await,
            Err(RepoError::Other)
        );
    }

    #[tokio::test]
    async fn approval_expired_row_reads_corrupt_and_unknown_order_errors() {
        let s = store().await;
        OrderRepo::insert(&s, rec(1), test_draft("a"), test_chain)
            .await
            .unwrap();
        s.approval_record("1", StoreDecision::Expired, "system", 1)
            .await
            .unwrap();
        let at = "2026-11-22T09:16:00+05:30".to_owned();
        assert_eq!(
            ApprovalRepo::decide(&s, 1, ApprovalDecision::Approve, at.clone()).await,
            Err(RepoError::Corrupt)
        );
        assert!(
            ApprovalRepo::decide(&s, 42, ApprovalDecision::Approve, at)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn kill_switch_set_and_read_with_audit() {
        let s = store().await;
        assert_eq!(KillSwitchRepo::is_engaged(&s).await, Ok(false));
        KillSwitchRepo::set(&s, true, test_draft("kill.engage"), test_chain)
            .await
            .unwrap();
        assert_eq!(KillSwitchRepo::is_engaged(&s).await, Ok(true));
        KillSwitchRepo::set(&s, false, test_draft("kill.release"), test_chain)
            .await
            .unwrap();
        assert_eq!(KillSwitchRepo::is_engaged(&s).await, Ok(false));
        let rows = AuditRepo::read_all(&s).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].draft.kind, "kill.engage");
        let empty = test_draft("");
        assert_eq!(
            KillSwitchRepo::set(&s, true, empty, test_chain).await,
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
        PaperLedgerRepo::append(&s, entry(1, "100000.50"))
            .await
            .unwrap();
        // One bad epoch in the batch stores none of it.
        let r = PaperLedgerRepo::append_many(&s, vec![entry(1, "-1"), entry(9, "-2")]).await;
        assert_eq!(r, Err(RepoError::Conflict));
        PaperLedgerRepo::append_many(&s, vec![entry(1, "-1.25")])
            .await
            .unwrap();
        let cur = PaperLedgerRepo::current_epoch(&s).await.unwrap();
        assert_eq!(cur.len(), 2);
        assert_eq!(cur[1].amount, Decimal::from_str("-1.25").unwrap());
        assert_eq!(PaperLedgerRepo::new_epoch(&s, entry(0, "50")).await, Ok(2));
        assert_eq!(PaperLedgerRepo::current_epoch(&s).await.unwrap().len(), 1);
    }
}
