//! Append-only paper ledger (`PaperLedgerRepo` shape as inherent methods, like the other store
//! repos). `ledger_append_many` is one SQL transaction: all entries or none.

use std::str::FromStr;

use bazaar_core::ports::{AuditDraft, ChainFn, LedgerEntry, RepoError};
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

impl SqliteStore {
    /// Append one entry.
    pub async fn ledger_append(&self, entry: &LedgerEntry) -> Result<(), RepoError> {
        self.ledger_append_many(std::slice::from_ref(entry)).await
    }

    /// Append all entries in order in one transaction; on any error none is stored.
    pub async fn ledger_append_many(&self, entries: &[LedgerEntry]) -> Result<(), RepoError> {
        if entries.len() > MAX_BATCH {
            return Err(RepoError::TooLarge);
        }
        entries.iter().try_for_each(check)?;
        let mut tx = self.pool.begin().await.map_err(|e| map_err(&e))?;
        // Appends go to the current epoch only (1 on an empty table); only `ledger_new_epoch`
        // raises it. A stale or jumped epoch is a `Conflict` and nothing is stored.
        let max: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(epoch), 1) FROM paper_ledger")
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
    /// second reset loses); on any error nothing is stored. Old entries stay.
    pub async fn ledger_new_epoch(
        &self,
        opening: &LedgerEntry,
        audit: &AuditDraft,
        chain: ChainFn,
    ) -> Result<u32, RepoError> {
        check(opening)?;
        let mut tx = self.pool.begin().await.map_err(|e| map_err(&e))?;
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

    #[tokio::test]
    async fn append_many_roundtrips_in_order_with_exact_decimals() {
        let s = store().await;
        let es = vec![
            entry(1, "opening", "100000.00", None),
            entry(1, "charge", "-12.345", Some(u128::MAX)),
        ];
        s.ledger_append_many(&es).await.unwrap();
        assert_eq!(s.ledger_current_epoch().await.unwrap(), es);
    }

    #[tokio::test]
    async fn append_many_is_all_or_nothing() {
        let s = store().await;
        let es = [entry(1, "a", "1", None), entry(0, "bad_epoch", "2", None)];
        assert_eq!(s.ledger_append_many(&es).await, Err(RepoError::Other));
        assert_eq!(s.ledger_current_epoch().await.unwrap(), Vec::new());
        // A failure inside the transaction rolls the earlier rows back.
        sqlx::query(
            "CREATE TRIGGER boom BEFORE INSERT ON paper_ledger WHEN NEW.kind = 'boom' \
             BEGIN SELECT RAISE(ABORT, 'x'); END",
        )
        .execute(&s.pool)
        .await
        .unwrap();
        let es = [entry(1, "a", "1", None), entry(1, "boom", "2", None)];
        assert!(s.ledger_append_many(&es).await.is_err());
        assert_eq!(s.ledger_current_epoch().await.unwrap(), Vec::new());
    }

    #[tokio::test]
    async fn appends_must_target_the_current_epoch() {
        let s = store().await;
        s.ledger_append(&entry(1, "opening", "5", None))
            .await
            .unwrap();
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
            &test_draft("r"),
            test_chain,
        )
        .await
        .unwrap();
        assert_eq!(
            s.ledger_append(&entry(1, "old", "1", None)).await,
            Err(RepoError::Conflict)
        );
        assert_eq!(s.ledger_append(&entry(2, "new", "1", None)).await, Ok(()));
        // An empty table accepts epoch 1 only.
        let e = store().await;
        assert_eq!(
            e.ledger_append(&entry(2, "x", "1", None)).await,
            Err(RepoError::Conflict)
        );
    }

    #[tokio::test]
    async fn caps_accept_exactly_the_maximum() {
        let s = store().await;
        let mut at_max = entry(1, &"k".repeat(MAX_KIND), "1", None);
        at_max.ts_ist_rfc3339 = "t".repeat(MAX_TS);
        assert_eq!(s.ledger_append(&at_max).await, Ok(()));
        let mut over_ts = at_max.clone();
        over_ts.ts_ist_rfc3339 = "t".repeat(MAX_TS + 1);
        assert_eq!(s.ledger_append(&over_ts).await, Err(RepoError::TooLarge));
        let batch = vec![entry(1, "k", "1", None); MAX_BATCH];
        assert_eq!(s.ledger_append_many(&batch).await, Ok(()));
        assert_eq!(s.ledger_current_epoch().await.unwrap().len(), MAX_BATCH + 1);
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
        let s = store().await;
        s.ledger_append(&entry(1, "opening", "5", None))
            .await
            .unwrap();
        let (d, c) = (test_draft("paper_reset"), test_chain);
        // Invalid line, stale epoch and a jumped epoch: refused, no ledger line, no audit row.
        assert_eq!(
            s.ledger_new_epoch(&entry(2, "", "9", None), &d, c).await,
            Err(RepoError::Other)
        );
        for e in [1, 3] {
            assert_eq!(
                s.ledger_new_epoch(&entry(e, "opening", "9", None), &d, c)
                    .await,
                Err(RepoError::Conflict)
            );
        }
        // An audit row that cannot be appended rolls the epoch line back too.
        let mut huge = d.clone();
        huge.payload = "x".repeat(1_000_000);
        assert!(
            s.ledger_new_epoch(&entry(2, "opening", "9", None), &huge, c)
                .await
                .is_err()
        );
        let cur = s.ledger_current_epoch().await.unwrap();
        assert_eq!((cur.len(), cur[0].epoch), (1, 1));
        assert_eq!(audit_count(&s).await, 0);
        // The epoch counter at u32::MAX cannot advance (no wrap).
        let o = store().await;
        raw(&o, i64::from(u32::MAX), "1", None).await;
        assert_eq!(
            o.ledger_new_epoch(&entry(u32::MAX, "opening", "1", None), &d, c)
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
            s.ledger_new_epoch(&o, &draft, test_chain),
            s.ledger_new_epoch(&o, &draft, test_chain)
        );
        assert_eq!(u8::from(first.is_ok()) + u8::from(second.is_ok()), 1);
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
        let s = store().await;
        s.ledger_append(&entry(1, "opening", "5", None))
            .await
            .unwrap();
        let n = s
            .ledger_new_epoch(
                &entry(2, "opening", "9", None),
                &test_draft("paper_reset"),
                test_chain,
            )
            .await
            .unwrap();
        assert_eq!(n, 2);
        assert_eq!(audit_count(&s).await, 1);
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
}
