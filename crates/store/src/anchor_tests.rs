use std::path::{Path, PathBuf};

use bazaar_core::ports::{AuditRow, RepoError};

use crate::anchor::{
    AnchorFault, AnchorRecord, MAX_ANCHOR_BYTES, anchor_path_for, check_anchor, encode, parse,
    read_file, write_atomic,
};
use crate::audit::{test_chain, test_draft};
use crate::sqlite::SqliteStore;

async fn seeded(db: &Path, n: usize) -> SqliteStore {
    let s = SqliteStore::open_anchored(db, test_chain).await.unwrap();
    for i in 0..n {
        s.audit_append(&test_draft(&format!("k{i}")), test_chain)
            .await
            .unwrap();
    }
    s
}

fn paths() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("t.db");
    let ap = anchor_path_for(&db);
    (dir, db, ap)
}

fn rec_of(rows: &[AuditRow], seq: usize) -> AnchorRecord {
    AnchorRecord {
        seq: seq as u64,
        hash: rows[seq - 1].hash,
    }
}

#[tokio::test]
async fn every_append_moves_the_anchor_to_the_tail() {
    let (_d, db, ap) = paths();
    let s = seeded(&db, 3).await;
    let rows = s.audit_read_all().await.unwrap();
    assert_eq!(read_file(&ap), Ok(rec_of(&rows, 3)));
    let st = s.anchor_status().await.unwrap();
    assert_eq!((st.last_seq, st.lag), (3, false));
    drop(s);
    // Reopen passes and nothing changes.
    let s = SqliteStore::open_anchored(&db, test_chain).await.unwrap();
    assert_eq!(s.audit_read_all().await.unwrap().len(), 3);
}

#[tokio::test]
async fn concurrent_appends_leave_the_anchor_at_the_tail() {
    let (_d, db, ap) = paths();
    let s = seeded(&db, 0).await;
    let (d1, d2, d3) = (test_draft("a"), test_draft("b"), test_draft("c"));
    let (r1, r2, r3) = tokio::join!(
        s.audit_append(&d1, test_chain),
        s.audit_append(&d2, test_chain),
        s.audit_append(&d3, test_chain)
    );
    assert!(r1.is_ok() && r2.is_ok() && r3.is_ok());
    let rows = s.audit_read_all().await.unwrap();
    assert_eq!(read_file(&ap), Ok(rec_of(&rows, 3)));
}

#[tokio::test]
async fn cut_tail_is_refused_by_the_ahead_check_and_nothing_is_appended() {
    let (_d, db, _ap) = paths();
    drop(seeded(&db, 3).await);
    let raw = SqliteStore::open(&db).await.unwrap();
    sqlx::query("DROP TRIGGER audit_log_no_delete")
        .execute(&raw.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM audit_log WHERE seq = 3")
        .execute(&raw.pool)
        .await
        .unwrap();
    let rows = raw.audit_read_all().await.unwrap();
    assert_eq!(rows.len(), 2);
    drop(raw);
    // The chain alone passes (ADR 0001 limit); the named check is `Ahead`.
    assert_eq!(
        check_anchor(&rows, read_file(&anchor_path_for(&db))),
        Err(AnchorFault::Ahead)
    );
    assert_eq!(
        SqliteStore::open_anchored(&db, test_chain).await.err(),
        Some(RepoError::Corrupt)
    );
    let raw = SqliteStore::open(&db).await.unwrap();
    assert_eq!(raw.audit_read_all().await.unwrap().len(), 2);
}

#[test]
fn check_anchor_names_each_refusing_rule() {
    let rows: Vec<AuditRow> = Vec::new();
    assert_eq!(check_anchor(&rows, Err(AnchorFault::Missing)), Ok(0));
    let ghost = AnchorRecord {
        seq: 1,
        hash: [7; 32],
    };
    // Empty log but an anchor exists: the log was wiped.
    assert_eq!(check_anchor(&rows, Ok(ghost)), Err(AnchorFault::Ahead));
    for f in [
        AnchorFault::Oversized,
        AnchorFault::Unparsable,
        AnchorFault::Io,
    ] {
        assert_eq!(check_anchor(&rows, Err(f)), Err(f));
    }
    assert_eq!(RepoError::from(AnchorFault::Io), RepoError::Unavailable);
    assert_eq!(RepoError::from(AnchorFault::Ahead), RepoError::Corrupt);
}

#[tokio::test]
async fn edited_hash_missing_file_garbage_and_oversize_are_each_refused_by_their_rule() {
    let (_d, db, ap) = paths();
    let s = seeded(&db, 2).await;
    let rows = s.audit_read_all().await.unwrap();
    drop(s);
    let good = rec_of(&rows, 2);

    let edited = AnchorRecord {
        hash: [9; 32],
        ..good
    };
    assert_eq!(
        check_anchor(&rows, Ok(edited)),
        Err(AnchorFault::HashMismatch)
    );
    std::fs::write(&ap, encode(&edited)).unwrap();
    assert_eq!(
        SqliteStore::open_anchored(&db, test_chain).await.err(),
        Some(RepoError::Corrupt)
    );

    std::fs::write(&ap, "not an anchor").unwrap();
    assert_eq!(read_file(&ap), Err(AnchorFault::Unparsable));
    assert_eq!(
        SqliteStore::open_anchored(&db, test_chain).await.err(),
        Some(RepoError::Corrupt)
    );

    std::fs::write(
        &ap,
        vec![b'a'; usize::try_from(MAX_ANCHOR_BYTES).unwrap() + 1],
    )
    .unwrap();
    assert_eq!(read_file(&ap), Err(AnchorFault::Oversized));
    assert_eq!(
        SqliteStore::open_anchored(&db, test_chain).await.err(),
        Some(RepoError::Corrupt)
    );

    std::fs::remove_file(&ap).unwrap();
    assert_eq!(read_file(&ap), Err(AnchorFault::Missing));
    assert_eq!(
        check_anchor(&rows, Err(AnchorFault::Missing)),
        Err(AnchorFault::Missing)
    );
    assert_eq!(
        SqliteStore::open_anchored(&db, test_chain).await.err(),
        Some(RepoError::Corrupt)
    );
}

#[test]
fn parse_rejects_malformed_variants() {
    let ok = encode(&AnchorRecord {
        seq: 5,
        hash: [1; 32],
    });
    assert_eq!(parse(ok.as_bytes()).map(|r| r.seq), Ok(5));
    let hex = "01".repeat(32);
    for bad in [
        format!("bazaar-audit-anchor v2\nseq=5\nhash={hex}\n"),
        format!("bazaar-audit-anchor v1\nseq=0\nhash={hex}\n"),
        format!("bazaar-audit-anchor v1\nseq=-1\nhash={hex}\n"),
        format!("bazaar-audit-anchor v1\nseq=5\nhash={}\n", "AB".repeat(32)),
        format!("bazaar-audit-anchor v1\nseq=5\nhash={}\n", &hex[2..]),
        format!("bazaar-audit-anchor v1\nseq=5\nhash={hex}\nextra=1\n"),
        "bazaar-audit-anchor v1\nseq=5\n".to_owned(),
    ] {
        assert_eq!(parse(bad.as_bytes()), Err(AnchorFault::Unparsable), "{bad}");
    }
    assert_eq!(parse(&[0xff, 0xfe]), Err(AnchorFault::Unparsable));
}

#[tokio::test]
async fn anchor_behind_the_log_passes_and_catches_up_at_open() {
    let (_d, db, ap) = paths();
    let s = seeded(&db, 3).await;
    let rows = s.audit_read_all().await.unwrap();
    drop(s);
    // Crash between commit and anchor write: the anchor still names row 2.
    write_atomic(&ap, &rec_of(&rows, 2)).unwrap();
    let s = SqliteStore::open_anchored(&db, test_chain).await.unwrap();
    assert_eq!(read_file(&ap), Ok(rec_of(&rows, 3)));
    assert_eq!(s.anchor_status().await.unwrap().last_seq, 3);
}

#[tokio::test]
async fn failed_anchor_write_keeps_the_commit_sets_lag_and_the_next_append_repairs() {
    let (_d, db, ap) = paths();
    let s = seeded(&db, 1).await;
    // A directory where the file belongs makes the rename fail.
    std::fs::remove_file(&ap).unwrap();
    std::fs::create_dir(&ap).unwrap();
    let row = s
        .audit_append(&test_draft("during-failure"), test_chain)
        .await
        .expect("the commit stands although the anchor write failed");
    assert_eq!(row.seq, 2);
    assert_eq!(s.audit_read_all().await.unwrap().len(), 2);
    let st = s.anchor_status().await.unwrap();
    assert!(st.lag);
    assert_eq!(st.last_seq, 1);
    assert!(s.anchor_flush().await.is_err());
    // Repair: the next append writes the (newer) tail and clears the flag.
    std::fs::remove_dir(&ap).unwrap();
    s.audit_append(&test_draft("after"), test_chain)
        .await
        .unwrap();
    let rows = s.audit_read_all().await.unwrap();
    assert_eq!(read_file(&ap), Ok(rec_of(&rows, 3)));
    let st = s.anchor_status().await.unwrap();
    assert_eq!((st.last_seq, st.lag), (3, false));
}

#[tokio::test]
async fn a_failing_write_leaves_the_old_anchor_intact() {
    let (_d, db, ap) = paths();
    let s = seeded(&db, 1).await;
    let before = std::fs::read(&ap).unwrap();
    let mut tmp = ap.as_os_str().to_owned();
    tmp.push(".tmp");
    std::fs::create_dir(PathBuf::from(tmp)).unwrap(); // the temp file cannot be created
    s.audit_append(&test_draft("x"), test_chain).await.unwrap();
    assert_eq!(std::fs::read(&ap).unwrap(), before);
    assert!(s.anchor_status().await.unwrap().lag);
}

#[tokio::test]
async fn adopt_writes_the_tail_for_a_verified_chain_and_open_then_works() {
    let (_d, db, ap) = paths();
    drop(seeded(&db, 2).await);
    std::fs::remove_file(&ap).unwrap();
    let r = SqliteStore::anchor_adopt(&db, test_chain, false)
        .await
        .unwrap();
    assert_eq!((r.old_seq, r.new_seq), (None, 2));
    assert!(SqliteStore::open_anchored(&db, test_chain).await.is_ok());
}

#[tokio::test]
async fn adopt_refuses_a_broken_chain_and_writes_nothing() {
    let (_d, db, ap) = paths();
    drop(seeded(&db, 2).await);
    std::fs::remove_file(&ap).unwrap();
    let raw = SqliteStore::open(&db).await.unwrap();
    sqlx::query("DROP TRIGGER audit_log_no_update")
        .execute(&raw.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE audit_log SET payload = '{\"edited\":1}' WHERE seq = 1")
        .execute(&raw.pool)
        .await
        .unwrap();
    drop(raw);
    assert_eq!(
        SqliteStore::anchor_adopt(&db, test_chain, true).await,
        Err(RepoError::Corrupt)
    );
    assert!(!ap.exists());
}

#[tokio::test]
async fn adopt_over_an_anchor_ahead_of_the_log_needs_accept_rollback() {
    let (_d, db, ap) = paths();
    drop(seeded(&db, 3).await);
    let ahead = std::fs::read(&ap).unwrap();
    let raw = SqliteStore::open(&db).await.unwrap();
    sqlx::query("DROP TRIGGER audit_log_no_delete")
        .execute(&raw.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM audit_log WHERE seq = 3")
        .execute(&raw.pool)
        .await
        .unwrap();
    drop(raw);
    assert_eq!(
        SqliteStore::anchor_adopt(&db, test_chain, false).await,
        Err(RepoError::Conflict)
    );
    assert_eq!(std::fs::read(&ap).unwrap(), ahead, "refusal writes nothing");
    let r = SqliteStore::anchor_adopt(&db, test_chain, true)
        .await
        .unwrap();
    assert_eq!((r.old_seq, r.new_seq), (Some(3), 2));
    assert!(SqliteStore::open_anchored(&db, test_chain).await.is_ok());
}

#[tokio::test]
async fn empty_log_without_a_file_opens_and_creates_the_anchor_on_first_append() {
    let (_d, db, ap) = paths();
    let s = SqliteStore::open_anchored(&db, test_chain).await.unwrap();
    assert!(!ap.exists());
    s.audit_append(&test_draft("first"), test_chain)
        .await
        .unwrap();
    assert_eq!(read_file(&ap).map(|r| r.seq), Ok(1));
}

#[tokio::test]
async fn order_and_kill_switch_commits_also_anchor() {
    let (_d, db, ap) = paths();
    let s = seeded(&db, 0).await;
    let d = test_draft("order.created");
    s.order_create_audited("o1", "PendingApproval", "{}", 1, &d, test_chain)
        .await
        .unwrap();
    assert_eq!(read_file(&ap).map(|r| r.seq), Ok(1));
    s.kill_switch_set_audited(true, "me", "t", 2, &test_draft("ks"), test_chain)
        .await
        .unwrap();
    assert_eq!(read_file(&ap).map(|r| r.seq), Ok(2));
}
