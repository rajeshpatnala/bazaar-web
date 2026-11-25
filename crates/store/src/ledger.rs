//! Append-only paper ledger (`PaperLedgerRepo` shape as inherent methods, like the other store
//! repos). `ledger_append_many` is one SQL transaction: all entries or none.

use std::str::FromStr;

use bazaar_core::ports::{
    AuditDraft, ChainFn, LEDGER_OPENING_KIND, LedgerEntry, PAPER_RESET_AUDIT_KIND, RepoError,
};
use rust_decimal::Decimal;
use sqlx::Row;

use crate::audit::append_in_tx;
use crate::sqlite::{SqliteStore, map_err};

const MAX_BATCH: usize = 1_000;
const MAX_KIND: usize = 64;
const MAX_TS: usize = 64;

const INSERT: &str =
    "INSERT INTO paper_ledger (epoch, kind, amount, ref_id, ts_ist) VALUES (?, ?, ?, ?, ?)";

fn check(e: &LedgerEntry) -> Result<(), RepoError> {
    if e.kind.is_empty() || e.epoch == 0 {
        return Err(RepoError::Other);
    }
    if e.kind.len() > MAX_KIND || e.ts_ist_rfc3339.len() > MAX_TS {
        return Err(RepoError::TooLarge);
    }
    Ok(())
}

fn bind<'q>(
    e: &LedgerEntry,
) -> sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments> {
    sqlx::query(INSERT)
        .bind(i64::from(e.epoch))
        .bind(e.kind.clone())
        .bind(e.amount.to_string())
        .bind(e.ref_id.map(|v| v.to_string()))
        .bind(e.ts_ist_rfc3339.clone())
}

/// Highest ledger line id, 0 on an empty ledger. A negative id is `Corrupt`.
async fn ledger_tail(tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>) -> Result<u64, RepoError> {
    let id: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(id), 0) FROM paper_ledger")
        .fetch_one(&mut **tx)
        .await
        .map_err(|e| map_err(&e))?;
    u64::try_from(id).map_err(|_| RepoError::Corrupt)
}

impl SqliteStore {
    /// Append one entry.
    pub async fn ledger_append(&self, entry: &LedgerEntry) -> Result<(), RepoError> {
        self.ledger_append_many(std::slice::from_ref(entry)).await
    }

    /// Append all entries in order in one transaction; on any error none is stored. An opening
    /// line is refused (`Other`) and so is any append on an empty ledger (`Conflict`): only
    /// [`Self::ledger_new_epoch`], which audits, starts an epoch.
    pub async fn ledger_append_many(&self, entries: &[LedgerEntry]) -> Result<(), RepoError> {
        if entries.len() > MAX_BATCH {
            return Err(RepoError::TooLarge);
        }
        entries.iter().try_for_each(check)?;
        if entries.iter().any(|e| e.kind == LEDGER_OPENING_KIND) {
            return Err(RepoError::Other);
        }
        let mut tx = self.pool.begin().await.map_err(|e| map_err(&e))?;
        // Appends go to the current epoch only; only `ledger_new_epoch` raises it. An empty table
        // reads as 0, which `check` never lets an entry carry. A stale or jumped epoch is a
        // `Conflict` and nothing is stored.
        let max: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(epoch), 0) FROM paper_ledger")
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| map_err(&e))?;
        if entries.iter().any(|e| i64::from(e.epoch) != max) {
            return Err(RepoError::Conflict);
        }
        for e in entries {
            bind(e).execute(&mut *tx).await.map_err(|e| map_err(&e))?;
        }
        tx.commit().await.map_err(|e| map_err(&e))
    }

    /// Entries of the highest epoch, in insertion order.
    pub async fn ledger_current_epoch(&self) -> Result<Vec<LedgerEntry>, RepoError> {
        let rows = sqlx::query(
            "SELECT epoch, kind, amount, ref_id, ts_ist FROM paper_ledger \
             WHERE epoch = (SELECT MAX(epoch) FROM paper_ledger) ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| map_err(&e))?;
        let g = |e: sqlx::Error| map_err(&e);
        rows.iter()
            .map(|r| {
                let epoch: i64 = r.try_get("epoch").map_err(g)?;
                let amount: String = r.try_get("amount").map_err(g)?;
                let ref_id: Option<String> = r.try_get("ref_id").map_err(g)?;
                Ok(LedgerEntry {
                    epoch: u32::try_from(epoch).map_err(|_| RepoError::Corrupt)?,
                    kind: r.try_get("kind").map_err(g)?,
                    amount: Decimal::from_str(&amount).map_err(|_| RepoError::Corrupt)?,
                    ref_id: ref_id
                        .map(|s| s.parse::<u128>().map_err(|_| RepoError::Corrupt))
                        .transpose()?,
                    ts_ist_rfc3339: r.try_get("ts_ist").map_err(g)?,
                })
            })
            .collect()
    }

    /// Audited reset: the opening line of epoch `opening.epoch` and the audit row in ONE
    /// transaction. `opening.epoch` must be the current max + 1, else `Conflict` (a racing
    /// second reset loses). The ledger tail must still be `expected_tail` inside this
    /// transaction, else `Conflict` (a line landed after the caller's decision). On any error
    /// nothing is stored. Old entries stay.
    pub async fn ledger_new_epoch(
        &self,
        opening: &LedgerEntry,
        expected_tail: u64,
        audit: &AuditDraft,
        chain: ChainFn,
    ) -> Result<u32, RepoError> {
        check(opening)?;
        if opening.kind != LEDGER_OPENING_KIND || audit.kind != PAPER_RESET_AUDIT_KIND {
            return Err(RepoError::Other);
        }
        let mut tx = self.pool.begin().await.map_err(|e| map_err(&e))?;
        if ledger_tail(&mut tx).await? != expected_tail {
            return Err(RepoError::Conflict);
        }
        let max: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(epoch), 0) FROM paper_ledger")
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| map_err(&e))?;
        if i64::from(opening.epoch) != max.checked_add(1).ok_or(RepoError::Other)? {
            return Err(RepoError::Conflict);
        }
        bind(opening)
            .execute(&mut *tx)
            .await
            .map_err(|e| map_err(&e))?;
        append_in_tx(&mut tx, audit, chain).await?;
        tx.commit().await.map_err(|e| map_err(&e))?;
        self.anchor_after_commit().await;
        Ok(opening.epoch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::{test_chain, test_draft};

    fn entry(epoch: u32, kind: &str, amt: &str, r: Option<u128>) -> LedgerEntry {
        LedgerEntry {
            epoch,
            kind: kind.into(),
            amount: Decimal::from_str(amt).unwrap(),
            ref_id: r,
            ts_ist_rfc3339: "2026-11-22T09:15:00+05:30".into(),
        }
    }

    async fn store() -> SqliteStore {
        SqliteStore::open_in_memory().await.unwrap()
    }

    /// A store whose epoch 1 was opened the only allowed way (writes one audit row).
    async fn opened() -> SqliteStore {
        let s = store().await;
        s.ledger_new_epoch(
            &entry(1, "opening", "5", None),
            0,
            &test_draft("paper_reset"),
            test_chain,
        )
        .await
        .unwrap();
        s
    }

    async fn line_count(s: &SqliteStore) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM paper_ledger")
            .fetch_one(&s.pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn append_many_roundtrips_in_order_with_exact_decimals() {
        let s = opened().await;
        let es = vec![
            entry(1, "buy", "-100000.00", None),
            entry(1, "charge", "-12.345", Some(u128::MAX)),
        ];
        s.ledger_append_many(&es).await.unwrap();
        assert_eq!(s.ledger_current_epoch().await.unwrap()[1..], es);
    }

    #[tokio::test]
    async fn append_many_is_all_or_nothing() {
        let s = opened().await;
        let es = [entry(1, "a", "1", None), entry(0, "bad_epoch", "2", None)];
        assert_eq!(s.ledger_append_many(&es).await, Err(RepoError::Other));
        assert_eq!(line_count(&s).await, 1);
        // A failure inside the transaction rolls the earlier rows back.
        sqlx::query(
            "CREATE TRIGGER boom BEFORE INSERT ON paper_ledger WHEN NEW.kind = 'boom' \
             BEGIN SELECT RAISE(ABORT, 'x'); END",
        )
        .execute(&s.pool)
        .await
        .unwrap();
        let es = [entry(1, "a", "1", None), entry(1, "boom", "2", None)];
        assert_eq!(s.ledger_append_many(&es).await, Err(RepoError::Other));
        assert_eq!(line_count(&s).await, 1);
    }

    #[tokio::test]
    async fn appends_must_target_the_current_epoch() {
        let s = opened().await;
        assert_eq!(
            s.ledger_append(&entry(2, "x", "1", None)).await,
            Err(RepoError::Conflict)
        );
        assert_eq!(
            s.ledger_append(&entry(u32::MAX, "x", "1", None)).await,
            Err(RepoError::Conflict)
        );
        // A mixed batch is refused whole.
        let mixed = [entry(1, "ok", "1", None), entry(7, "bad", "1", None)];
        assert_eq!(s.ledger_append_many(&mixed).await, Err(RepoError::Conflict));
        assert_eq!(s.ledger_current_epoch().await.unwrap().len(), 1);
        s.ledger_new_epoch(
            &entry(2, "opening", "9", None),
            1,
            &test_draft("paper_reset"),
            test_chain,
        )
        .await
        .unwrap();
        assert_eq!(
            s.ledger_append(&entry(1, "old", "1", None)).await,
            Err(RepoError::Conflict)
        );
        assert_eq!(s.ledger_append(&entry(2, "new", "1", None)).await, Ok(()));
    }

    /// RISK-176-3 / CR-176-2 / SEC-176-1: no epoch can be started without an audit row.
    #[tokio::test]
    async fn append_on_an_empty_ledger_is_refused_for_any_epoch() {
        let s = store().await;
        for epoch in [1, 2] {
            let e = entry(epoch, "charge", "-1", None);
            assert_eq!(s.ledger_append(&e).await, Err(RepoError::Conflict));
            assert_eq!(s.ledger_append_many(&[e]).await, Err(RepoError::Conflict));
        }
        assert_eq!((line_count(&s).await, audit_count(&s).await), (0, 0));
    }

    #[tokio::test]
    async fn append_refuses_an_opening_line_alone_or_in_a_batch() {
        for s in [store().await, opened().await] {
            let before = (line_count(&s).await, audit_count(&s).await);
            let opening = entry(1, "opening", "100000", None);
            assert_eq!(s.ledger_append(&opening).await, Err(RepoError::Other));
            let mixed = [entry(1, "charge", "-1", None), opening];
            assert_eq!(s.ledger_append_many(&mixed).await, Err(RepoError::Other));
            assert_eq!((line_count(&s).await, audit_count(&s).await), before);
        }
    }

    #[tokio::test]
    async fn new_epoch_requires_an_opening_line_and_a_paper_reset_audit_row() {
        let s = opened().await;
        for (line, audit) in [
            ("charge", "paper_reset"),
            ("sell", "paper_reset"),
            ("opening", "approve"),
        ] {
            assert_eq!(
                s.ledger_new_epoch(
                    &entry(2, line, "100000", None),
                    1,
                    &test_draft(audit),
                    test_chain
                )
                .await,
                Err(RepoError::Other),
                "{line}/{audit}"
            );
        }
        assert_eq!((line_count(&s).await, audit_count(&s).await), (1, 1));
    }

    /// TS-176-2: a failing INSERT on either side stores neither the epoch line nor the audit row.
    #[tokio::test]
    async fn new_epoch_insert_failure_on_either_table_rolls_back_both() {
        for trigger in [
            "CREATE TRIGGER boom BEFORE INSERT ON paper_ledger BEGIN SELECT RAISE(ABORT, 'x'); END",
            "CREATE TRIGGER boom BEFORE INSERT ON audit_log BEGIN SELECT RAISE(ABORT, 'x'); END",
        ] {
            let s = opened().await;
            let before = s.ledger_current_epoch().await.unwrap();
            let rows = s.audit_read_all().await.unwrap();
            sqlx::query(trigger).execute(&s.pool).await.unwrap();
            assert_eq!(
                s.ledger_new_epoch(
                    &entry(2, "opening", "9", None),
                    1,
                    &test_draft("paper_reset"),
                    test_chain
                )
                .await,
                Err(RepoError::Other),
                "{trigger}"
            );
            assert_eq!(s.ledger_current_epoch().await.unwrap(), before, "{trigger}");
            assert_eq!(line_count(&s).await, 1, "{trigger}");
            assert_eq!(s.audit_read_all().await.unwrap(), rows, "{trigger}");
        }
    }

    #[tokio::test]
    async fn caps_accept_exactly_the_maximum() {
        let s = opened().await;
        let mut at_max = entry(1, &"k".repeat(MAX_KIND), "1", None);
        at_max.ts_ist_rfc3339 = "t".repeat(MAX_TS);
        assert_eq!(s.ledger_append(&at_max).await, Ok(()));
        let mut over_ts = at_max.clone();
        over_ts.ts_ist_rfc3339 = "t".repeat(MAX_TS + 1);
        assert_eq!(s.ledger_append(&over_ts).await, Err(RepoError::TooLarge));
        let batch = vec![entry(1, "k", "1", None); MAX_BATCH];
        assert_eq!(s.ledger_append_many(&batch).await, Ok(()));
        assert_eq!(s.ledger_current_epoch().await.unwrap().len(), MAX_BATCH + 2);
    }

    async fn raw(s: &SqliteStore, epoch: i64, amount: &str, ref_id: Option<&str>) {
        sqlx::query(
            "INSERT INTO paper_ledger (epoch, kind, amount, ref_id, ts_ist) \
             VALUES (?, 'k', ?, ?, 't')",
        )
        .bind(epoch)
        .bind(amount)
        .bind(ref_id)
        .execute(&s.pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn corrupt_stored_values_fail_closed() {
        let bad_amount = store().await;
        raw(&bad_amount, 1, "not-a-number", None).await;
        assert_eq!(
            bad_amount.ledger_current_epoch().await,
            Err(RepoError::Corrupt)
        );
        let bad_ref = store().await;
        raw(&bad_ref, 1, "1", Some("xyz")).await;
        assert_eq!(
            bad_ref.ledger_current_epoch().await,
            Err(RepoError::Corrupt)
        );
        let big_epoch = store().await;
        raw(&big_epoch, 4_294_967_296, "1", None).await;
        assert_eq!(
            big_epoch.ledger_current_epoch().await,
            Err(RepoError::Corrupt)
        );
    }

    async fn audit_count(s: &SqliteStore) -> usize {
        s.audit_read_all().await.unwrap().len()
    }

    #[tokio::test]
    async fn new_epoch_failures_store_nothing() {
        let s = opened().await;
        let (d, c) = (test_draft("paper_reset"), test_chain);
        // Invalid line, stale epoch and a jumped epoch: refused, no ledger line, no audit row.
        assert_eq!(
            s.ledger_new_epoch(&entry(2, "", "9", None), 1, &d, c).await,
            Err(RepoError::Other)
        );
        for e in [1, 3] {
            assert_eq!(
                s.ledger_new_epoch(&entry(e, "opening", "9", None), 1, &d, c)
                    .await,
                Err(RepoError::Conflict)
            );
        }
        // An audit row that cannot be appended rolls the epoch line back too.
        let mut huge = d.clone();
        huge.payload = "x".repeat(1_000_000);
        assert_eq!(
            s.ledger_new_epoch(&entry(2, "opening", "9", None), 1, &huge, c)
                .await,
            Err(RepoError::TooLarge)
        );
        let cur = s.ledger_current_epoch().await.unwrap();
        assert_eq!((cur.len(), cur[0].epoch), (1, 1));
        assert_eq!(audit_count(&s).await, 1);
        // The epoch counter at u32::MAX cannot advance (no wrap).
        let o = store().await;
        raw(&o, i64::from(u32::MAX), "1", None).await;
        assert_eq!(
            o.ledger_new_epoch(&entry(u32::MAX, "opening", "1", None), 1, &d, c)
                .await,
            Err(RepoError::Conflict)
        );
        assert_eq!(audit_count(&o).await, 0);
    }

    #[tokio::test]
    async fn racing_resets_one_wins_and_writes_one_audit_row() {
        let s = store().await;
        let draft = test_draft("paper_reset");
        let o = entry(1, "opening", "9", None);
        let (first, second) = tokio::join!(
            s.ledger_new_epoch(&o, 0, &draft, test_chain),
            s.ledger_new_epoch(&o, 0, &draft, test_chain)
        );
        assert_eq!(u8::from(first.is_ok()) + u8::from(second.is_ok()), 1);
        assert_eq!(first.and(second), Err(RepoError::Conflict), "the loser");
        assert_eq!(audit_count(&s).await, 1);
        assert_eq!(s.ledger_current_epoch().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn sql_checks_back_the_app_caps() {
        let s = store().await;
        let ins = |kind: String| {
            sqlx::query(
                "INSERT INTO paper_ledger (epoch, kind, amount, ts_ist) VALUES (1, ?, '1', 't')",
            )
            .bind(kind)
            .execute(&s.pool)
        };
        assert!(ins(String::new()).await.is_err());
        assert!(ins("k".repeat(MAX_KIND + 1)).await.is_err());
        assert!(ins("k".repeat(MAX_KIND)).await.is_ok());
    }

    #[tokio::test]
    async fn caps_and_empty_batch() {
        let s = store().await;
        assert_eq!(s.ledger_append_many(&[]).await, Ok(()));
        let big = vec![entry(1, "k", "1", None); MAX_BATCH + 1];
        assert_eq!(s.ledger_append_many(&big).await, Err(RepoError::TooLarge));
        let long = entry(1, &"k".repeat(MAX_KIND + 1), "1", None);
        assert_eq!(s.ledger_append(&long).await, Err(RepoError::TooLarge));
        assert_eq!(
            s.ledger_append(&entry(1, "", "1", None)).await,
            Err(RepoError::Other)
        );
    }

    #[tokio::test]
    async fn new_epoch_keeps_old_entries_and_is_append_only() {
        let s = opened().await;
        let n = s
            .ledger_new_epoch(
                &entry(2, "opening", "9", None),
                1,
                &test_draft("paper_reset"),
                test_chain,
            )
            .await
            .unwrap();
        assert_eq!(n, 2);
        assert_eq!(audit_count(&s).await, 2);
        let cur = s.ledger_current_epoch().await.unwrap();
        assert_eq!(cur.len(), 1);
        assert_eq!(cur[0].epoch, 2);
        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM paper_ledger")
            .fetch_one(&s.pool)
            .await
            .unwrap();
        assert_eq!(total, 2);
        assert!(
            sqlx::query("DELETE FROM paper_ledger")
                .execute(&s.pool)
                .await
                .is_err()
        );
        assert!(
            sqlx::query("UPDATE paper_ledger SET kind = 'x'")
                .execute(&s.pool)
                .await
                .is_err()
        );
    }

    /// RISK-176-2: the tail is rechecked inside the write transaction. The epoch number is
    /// right in every refused call, so only the tail check can refuse it.
    #[tokio::test]
    async fn new_epoch_refuses_a_tail_that_is_not_the_ledger_tail() {
        let s = opened().await;
        let judged = 1; // the opening line
        s.ledger_append(&entry(1, "charge", "-1", None))
            .await
            .unwrap();
        let (next, d) = (entry(2, "opening", "9", None), test_draft("paper_reset"));
        // Moved by an append; `Initial` (0) on a non-empty ledger; a tail ahead of the ledger.
        for stale in [judged, 0, 3] {
            assert_eq!(
                s.ledger_new_epoch(&next, stale, &d, test_chain).await,
                Err(RepoError::Conflict),
                "tail {stale}"
            );
        }
        assert_eq!((line_count(&s).await, audit_count(&s).await), (2, 1));
        assert_eq!(s.ledger_new_epoch(&next, 2, &d, test_chain).await, Ok(2));
    }

    /// An unreadable tail is never taken as "empty": nothing is written.
    #[tokio::test]
    async fn new_epoch_with_a_negative_tail_is_corrupt() {
        let s = store().await;
        sqlx::query(
            "INSERT INTO paper_ledger (id, epoch, kind, amount, ts_ist) \
             VALUES (-1, 1, 'k', '1', 't')",
        )
        .execute(&s.pool)
        .await
        .unwrap();
        let (next, d) = (entry(2, "opening", "9", None), test_draft("paper_reset"));
        assert_eq!(
            s.ledger_new_epoch(&next, 0, &d, test_chain).await,
            Err(RepoError::Corrupt)
        );
        assert_eq!((line_count(&s).await, audit_count(&s).await), (1, 0));
    }
}
