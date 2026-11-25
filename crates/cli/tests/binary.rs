//! The real binary: exit codes and output lines (no network; temp dirs and fixtures only).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::{Command, Output};

use bazaar_core::testutil::fixture_path;

fn run(data: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_bazaar-cli"))
        .arg("--config-dir")
        .arg(fixture_path("config"))
        .arg("--data-dir")
        .arg(data)
        .args(args)
        .output()
        .unwrap()
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn import_succeeds_and_prints_counts() {
    let tmp = tempfile::tempdir().unwrap();
    let f = fixture_path("data/instruments_good.csv");
    let o = run(
        tmp.path(),
        &["import-instruments", "--file", f.to_str().unwrap()],
    );
    assert!(o.status.success());
    assert!(out(&o).starts_with("imported 5 rows"));
}

#[test]
fn import_of_missing_file_fails_with_a_category() {
    let tmp = tempfile::tempdir().unwrap();
    let o = run(
        tmp.path(),
        &["import-instruments", "--file", "/nonexistent/x.csv"],
    );
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("import failed: input file unreadable"));
}

#[test]
fn check_on_empty_store_prints_ok_sqlite_then_failed_instruments_and_exits_1() {
    let tmp = tempfile::tempdir().unwrap();
    let o = run(tmp.path(), &["check-connections"]);
    assert_eq!(o.status.code(), Some(1));
    let text = out(&o);
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("OK sqlite"));
    assert_eq!(lines.next(), Some("OK duckdb"));
    assert!(lines.next().unwrap().starts_with("FAILED instruments "));
}

#[test]
fn no_subcommand_exits_zero() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(run(tmp.path(), &[]).status.success());
}

/// The binary prints the new wording and keeps its exit codes (`verify` only; temp dirs).
#[test]
fn history_checkpoint_verify_without_a_database_fails_with_the_next_step() {
    let tmp = tempfile::tempdir().unwrap();
    let o = run(tmp.path(), &["history-checkpoint", "verify"]);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&o.stderr),
        "History check not run: no database in the data directory. Check --data-dir, \
         permissions, and that the server is stopped, then run again.\n"
    );
    assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
}

#[test]
fn history_checkpoint_verify_passes_then_names_a_disagreement() {
    let tmp = tempfile::tempdir().unwrap();
    // Creates an empty database (it fails only on the missing instruments).
    run(tmp.path(), &["check-connections"]);
    let o = run(tmp.path(), &["history-checkpoint", "verify"]);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(
        out(&o),
        "History check passed: history checkpoint at entry number 0.\n"
    );
    let anchor = tmp.path().join("bazaar.sqlite.anchor");
    std::fs::write(&anchor, "garbage").unwrap();
    let o = run(tmp.path(), &["history-checkpoint", "verify"]);
    assert_eq!(o.status.code(), Some(1));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        err.starts_with("History check failed: the activity history and the history checkpoint"),
        "{err}"
    );
    assert_eq!(std::fs::read(&anchor).unwrap(), b"garbage");
}

#[test]
fn the_old_command_name_is_a_usage_error() {
    let tmp = tempfile::tempdir().unwrap();
    let o = run(tmp.path(), &["audit-anchor", "status"]);
    assert_eq!(o.status.code(), Some(2));
}
