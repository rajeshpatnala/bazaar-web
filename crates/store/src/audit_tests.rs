//! Tests for the audit hardening: continuity trigger, field caps, chain verify on open.

use bazaar_core::ports::{AuditDraft, ChainFn, RepoError};

use crate::audit::{
    MAX_ACTOR, MAX_KIND, MAX_PAYLOAD, MAX_TS, append_in_tx, test_chain, test_draft, verify_rows,
};
use crate::sqlite::SqliteStore;

async fn append(s: &SqliteStore, d: &AuditDraft) -> Result<u64, RepoError> {
    let mut tx = s.pool.begin().await.unwrap();
    let r = append_in_tx(&mut tx, d, test_chain).await;
    if r.is_ok() {
        tx.commit().await.unwrap();
    }
    r
}

async fn store_with(n: usize) -> SqliteStore {
    let s = SqliteStore::open_in_memory().await.unwrap();
    for _ in 0..n {
        append(&s, &test_draft("k")).await.unwrap();
    }
    s
}

async fn raw_insert(s: &SqliteStore, seq: i64, prev: Vec<u8>) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO audit_log (seq, ts_ist, actor, kind, payload, prev_hash, hash) \
         VALUES (?, 't', 'a', 'k', '{}', ?, ?)",
    )
    .bind(seq)
    .bind(prev)
    .bind(vec![7_u8; 32])
    .execute(&s.pool)
    .await
    .map(|_| ())
}

#[tokio::test]
async fn trigger_refuses_gap_duplicate_and_wrong_prev_hash() {
    let s = store_with(1).await;
    let h1: Vec<u8> = sqlx::query_scalar("SELECT hash FROM audit_log WHERE seq = 1")
        .fetch_one(&s.pool)
        .await
        .unwrap();
    assert!(raw_insert(&s, 3, h1.clone()).await.is_err(), "gap");
    assert!(
        raw_insert(&s, 1, h1.clone()).await.is_err(),
        "duplicate seq"
    );
    assert!(
        raw_insert(&s, 2, vec![0; 32]).await.is_err(),
        "wrong prev_hash"
    );
    assert!(raw_insert(&s, 2, h1).await.is_ok(), "valid continuation");
}

#[tokio::test]
async fn trigger_requires_genesis_prev_hash_on_empty_log() {
    let s = store_with(0).await;
    assert!(raw_insert(&s, 1, vec![9; 32]).await.is_err());
    assert!(raw_insert(&s, 2, vec![0; 32]).await.is_err());
    assert!(raw_insert(&s, 1, vec![0; 32]).await.is_ok());
}

#[tokio::test]
async fn caps_refuse_oversized_fields_and_write_nothing() {
    let s = store_with(0).await;
    let cases = [
        AuditDraft {
            actor: "a".repeat(MAX_ACTOR + 1),
            ..test_draft("k")
        },
        AuditDraft {
            kind: "k".repeat(MAX_KIND + 1),
            ..test_draft("k")
        },
        AuditDraft {
            payload: "p".repeat(MAX_PAYLOAD + 1),
            ..test_draft("k")
        },
        AuditDraft {
            ts_ist_rfc3339: "t".repeat(MAX_TS + 1),
            ..test_draft("k")
        },
    ];
    for d in &cases {
        assert_eq!(append(&s, d).await, Err(RepoError::TooLarge));
    }
    assert_eq!(s.audit_read_all().await.unwrap(), Vec::new());
    let edge = AuditDraft {
        actor: "a".repeat(MAX_ACTOR),
        ..test_draft("k")
    };
    assert_eq!(append(&s, &edge).await, Ok(1));
}

#[tokio::test]
async fn verify_passes_on_good_chain_and_fails_on_forged_hashes() {
    let s = store_with(3).await;
    assert_eq!(s.audit_read_all().await.unwrap().len(), 3);
    assert_eq!(s.audit_verify(test_chain).await, Ok(()));
    let other: ChainFn = |p, _, _| *p;
    assert_eq!(s.audit_verify(other).await, Err(RepoError::Corrupt));
}

#[tokio::test]
async fn open_verified_accepts_good_file_and_refuses_tampered_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.db");
    let s = SqliteStore::open(&path).await.unwrap();
    for _ in 0..3 {
        append(&s, &test_draft("k")).await.unwrap();
    }
    drop(s);
    assert!(SqliteStore::open_verified(&path, test_chain).await.is_ok());
    // Out-of-band tamper: someone with file access drops the trigger and edits a row.
    let raw = SqliteStore::open(&path).await.unwrap();
    sqlx::query("DROP TRIGGER audit_log_no_update")
        .execute(&raw.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE audit_log SET payload = '{\"x\":1}' WHERE seq = 2")
        .execute(&raw.pool)
        .await
        .unwrap();
    drop(raw);
    assert_eq!(
        SqliteStore::open_verified(&path, test_chain).await.err(),
        Some(RepoError::Corrupt)
    );
}

#[tokio::test]
async fn migrations_8_and_9_apply_on_an_existing_v7_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    let s = SqliteStore::open(&path).await.unwrap();
    for _ in 0..2 {
        append(&s, &test_draft("k")).await.unwrap();
    }
    // Rewind to the schema as it was at migration 7.
    for sql in [
        "DROP TRIGGER audit_log_continuity",
        "DROP TABLE paper_ledger",
        "DELETE FROM _sqlx_migrations WHERE version >= 8",
    ] {
        sqlx::query(sql).execute(&s.pool).await.unwrap();
    }
    drop(s);
    let s = SqliteStore::open_verified(&path, test_chain).await.unwrap();
    assert_eq!(s.audit_read_all().await.unwrap().len(), 2, "rows survive");
    assert!(
        raw_insert(&s, 9, vec![0; 32]).await.is_err(),
        "trigger is back"
    );
    assert_eq!(s.ledger_current_epoch().await.unwrap(), Vec::new());
    sqlx::query("INSERT INTO paper_ledger (epoch, kind, amount, ts_ist) VALUES (1, 'k', '1', 't')")
        .execute(&s.pool)
        .await
        .unwrap();
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

#[tokio::test]
async fn verify_rows_detects_removed_swapped_and_altered_rows() {
    let rows = store_with(4).await.audit_read_all().await.unwrap();
    assert_eq!(verify_rows(&rows, test_chain), Ok(()));
    let mut removed = rows.clone();
    removed.remove(1);
    assert_eq!(verify_rows(&removed, test_chain), Err(RepoError::Corrupt));
    let mut swapped = rows.clone();
    swapped.swap(1, 2);
    assert_eq!(verify_rows(&swapped, test_chain), Err(RepoError::Corrupt));
    let mut altered = rows.clone();
    altered[2].draft.payload = "{\"x\":1}".to_owned();
    assert_eq!(verify_rows(&altered, test_chain), Err(RepoError::Corrupt));
    let mut relinked = rows;
    relinked[2].prev_hash = [1; 32];
    assert_eq!(verify_rows(&relinked, test_chain), Err(RepoError::Corrupt));
}

#[tokio::test]
async fn truncated_tail_still_verifies_documented_limit() {
    // ADR 0001: tail truncation is undetectable by hash; the slice-8 anchor covers it.
    let mut rows = store_with(4).await.audit_read_all().await.unwrap();
    rows.truncate(2);
    assert_eq!(verify_rows(&rows, test_chain), Ok(()));
    let s = store_with(4).await;
    sqlx::query("DROP TRIGGER audit_log_no_delete")
        .execute(&s.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM audit_log WHERE seq > 2")
        .execute(&s.pool)
        .await
        .unwrap();
    assert_eq!(s.audit_verify(test_chain).await, Ok(()));
}

#[tokio::test]
async fn caps_accept_exactly_the_maximum() {
    let s = store_with(0).await;
    let d = AuditDraft {
        ts_ist_rfc3339: "t".repeat(MAX_TS),
        actor: "a".repeat(MAX_ACTOR),
        kind: "k".repeat(MAX_KIND),
        payload: "p".repeat(MAX_PAYLOAD),
    };
    assert_eq!(append(&s, &d).await, Ok(1));
}

#[tokio::test]
async fn audit_hash_column_must_be_32_bytes() {
    let s = store_with(0).await;
    let r = sqlx::query(
        "INSERT INTO audit_log (seq, ts_ist, actor, kind, payload, prev_hash, hash) \
         VALUES (1, 't', 'a', 'k', '{}', zeroblob(32), x'00')",
    )
    .execute(&s.pool)
    .await;
    assert!(r.is_err());
}

#[tokio::test]
async fn verify_detects_removed_row() {
    let s = store_with(3).await;
    sqlx::query("DROP TRIGGER audit_log_no_delete")
        .execute(&s.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM audit_log WHERE seq = 2")
        .execute(&s.pool)
        .await
        .unwrap();
    assert_eq!(s.audit_verify(test_chain).await, Err(RepoError::Corrupt));
}
