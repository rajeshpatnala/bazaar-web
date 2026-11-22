//! `open_store` verifies the audit chain; cli code never opens the execution store plainly.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use bazaar_cli::compose::open_store;
use bazaar_core::ports::{AuditDraft, AuditRepo, RepoError};
use bazaar_execution::audit::chain_hash;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Connection};

fn draft(kind: &str) -> AuditDraft {
    AuditDraft {
        ts_ist_rfc3339: "2026-11-22T09:15:00+05:30".to_owned(),
        actor: "tester".to_owned(),
        kind: kind.to_owned(),
        payload: "{}".to_owned(),
    }
}

async fn seeded(path: &Path) {
    let s = open_store(path).await.unwrap();
    for k in ["a", "b", "c"] {
        AuditRepo::append(&s, draft(k), chain_hash).await.unwrap();
    }
}

#[tokio::test]
async fn untouched_db_reopens_with_its_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    seeded(&path).await;
    let s = open_store(&path).await.unwrap();
    assert_eq!(AuditRepo::read_all(&s).await.unwrap().len(), 3);
}

#[tokio::test]
async fn one_altered_audit_row_fails_open_with_corrupt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    seeded(&path).await;
    // Out-of-band edit by someone with file access: drop the guard trigger, change one row.
    let mut c = SqliteConnectOptions::new()
        .filename(&path)
        .connect()
        .await
        .unwrap();
    sqlx::query("DROP TRIGGER audit_log_no_update")
        .execute(&mut c)
        .await
        .unwrap();
    sqlx::query("UPDATE audit_log SET actor = 'mallory' WHERE seq = 2")
        .execute(&mut c)
        .await
        .unwrap();
    c.close().await.unwrap();
    // A plain open would succeed here; only the verified open refuses.
    let r = open_store(&path).await;
    assert_eq!(r.err(), Some(RepoError::Corrupt));
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

fn code_only(src: &str) -> String {
    src.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn compose_opens_verified_with_the_execution_chain_hash() {
    let src =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/compose.rs")).unwrap();
    let code = code_only(&src);
    assert!(code.contains("open_verified(path, chain_hash)"));
    assert!(code.contains("bazaar_execution::audit::chain_hash"));
    assert!(!plain_open_violation(&code));
}

/// True when the code could open a store without audit verification: whitespace is stripped so
/// `SqliteStore :: open (` is caught, any `SqliteStore` alias (`use ... as X`) or a bare `::open(`
/// is flagged, and so is any `open_*` variant other than `open_verified` (SEC-129-3).
fn plain_open_violation(code: &str) -> bool {
    let c: String = code.chars().filter(|ch| !ch.is_whitespace()).collect();
    if c.contains("SqliteStoreas") || c.contains("::open(") || c.contains("open_in_memory") {
        return true;
    }
    let mut rest = c.as_str();
    while let Some(i) = rest.find("SqliteStore::open_") {
        rest = &rest[i + "SqliteStore::open_".len()..];
        if !rest.starts_with("verified") {
            return true;
        }
    }
    false
}

#[test]
fn plain_open_guard_catches_spacing_aliases_and_variants() {
    assert!(plain_open_violation("SqliteStore::open(p)"));
    assert!(plain_open_violation("bazaar_store::SqliteStore :: open (p)"));
    assert!(plain_open_violation("use bazaar_store::SqliteStore as S;"));
    assert!(plain_open_violation("S::open(p)"));
    assert!(plain_open_violation("SqliteStore::open_in_memory()"));
    assert!(plain_open_violation("SqliteStore::open_with(p)"));
    assert!(!plain_open_violation("SqliteStore::open_verified(p, h)"));
    assert!(!plain_open_violation("fn open_store() {}"));
}

#[test]
fn no_cli_code_outside_the_instrument_commands_opens_a_plain_store() {
    // The instrument commands own their own file handling and never touch execution tables;
    // everything else (composition, future execution wiring) must go through `open_store`.
    let mut files = Vec::new();
    rs_files(
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src")),
        &mut files,
    );
    assert!(files.len() >= 3, "scan found no cli sources");
    for f in files {
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        if name == "check_connections.rs" || name == "import_instruments.rs" {
            continue;
        }
        let code = code_only(&std::fs::read_to_string(&f).unwrap());
        assert!(
            !plain_open_violation(&code),
            "{} opens the store without audit verification; use compose::open_store",
            f.display()
        );
    }
}
