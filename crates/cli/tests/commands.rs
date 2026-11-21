//! `import-instruments` and `check-connections` against fixtures and temp dirs only.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use bazaar_cli::commands::{CommandError, check_connections, import_instruments};
use bazaar_core::testutil::{FakeClock, fixture_path};
use bazaar_data::instruments::ParseLimits;
use chrono::TimeZone;
use chrono_tz::Asia::Kolkata;

fn clock(y: i32, m: u32, d: u32) -> FakeClock {
    FakeClock::at(Kolkata.with_ymd_and_hms(y, m, d, 10, 0, 0).unwrap())
}

fn good() -> PathBuf {
    fixture_path("data/instruments_good.csv")
}

fn config_dir() -> PathBuf {
    fixture_path("config")
}

async fn import(dir: &Path, file: &Path, c: &FakeClock) -> Result<(), CommandError> {
    import_instruments(dir, file, c, &ParseLimits::default())
        .await
        .map(|_| ())
}

#[tokio::test]
async fn import_twice_is_idempotent_and_counts_rows() {
    let tmp = tempfile::tempdir().unwrap();
    let c = clock(2026, 1, 6);
    let a = import_instruments(tmp.path(), &good(), &c, &ParseLimits::default())
        .await
        .unwrap();
    let b = import_instruments(tmp.path(), &good(), &c, &ParseLimits::default())
        .await
        .unwrap();
    assert_eq!(a, b);
    assert!(a.written > 0 && a.written == a.accepted);
}

#[tokio::test]
async fn bad_rows_are_counted_not_fatal() {
    let tmp = tempfile::tempdir().unwrap();
    let f = fixture_path("data/instruments_bad_rows.csv");
    let s = import_instruments(tmp.path(), &f, &clock(2026, 1, 6), &ParseLimits::default())
        .await
        .unwrap();
    assert!(s.rejected > 0);
}

#[tokio::test]
async fn check_fresh_after_import_then_stale_a_week_later() {
    let tmp = tempfile::tempdir().unwrap();
    let c = clock(2026, 1, 6);
    import(tmp.path(), &good(), &c).await.unwrap();
    let r = check_connections(&config_dir(), tmp.path(), &c).await;
    assert_eq!(r.lines, ["OK sqlite", "OK duckdb", "OK instruments"]);
    assert!(r.ok);
    let later = check_connections(&config_dir(), tmp.path(), &clock(2026, 1, 20)).await;
    assert_eq!(
        later.lines,
        ["OK sqlite", "OK duckdb", "FAILED instruments stale"]
    );
    assert!(!later.ok);
}

#[tokio::test]
async fn check_on_empty_store_reports_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let r = check_connections(&config_dir(), tmp.path(), &clock(2026, 1, 6)).await;
    assert_eq!(
        r.lines,
        ["OK sqlite", "OK duckdb", "FAILED instruments empty"]
    );
}

#[tokio::test]
async fn placeholder_calendar_is_never_fresh() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();
    std::fs::write(
        cfg.path().join("calendar.toml"),
        "schema_version = 1\nplaceholder = true\nsource = \"not populated\"\ncovered_years = []\n",
    )
    .unwrap();
    let c = clock(2026, 1, 6);
    import(tmp.path(), &good(), &c).await.unwrap();
    let r = check_connections(cfg.path(), tmp.path(), &c).await;
    assert_eq!(
        r.lines,
        [
            "OK sqlite",
            "OK duckdb",
            "FAILED instruments calendar_placeholder"
        ]
    );
}

#[tokio::test]
async fn missing_calendar_and_uncovered_year_fail_with_categories() {
    let tmp = tempfile::tempdir().unwrap();
    let empty_cfg = tempfile::tempdir().unwrap();
    let c = clock(2026, 1, 6);
    import(tmp.path(), &good(), &c).await.unwrap();
    let r = check_connections(empty_cfg.path(), tmp.path(), &c).await;
    assert_eq!(r.lines[2], "FAILED instruments calendar_unreadable");
    let r = check_connections(&config_dir(), tmp.path(), &clock(2031, 6, 2)).await;
    assert_eq!(r.lines[2], "FAILED instruments calendar_out_of_range");
}

#[tokio::test]
async fn oversize_file_is_refused_before_reading() {
    let tmp = tempfile::tempdir().unwrap();
    let limits = ParseLimits {
        max_bytes: 10,
        ..ParseLimits::default()
    };
    let e = import_instruments(tmp.path(), &good(), &clock(2026, 1, 6), &limits)
        .await
        .unwrap_err();
    assert_eq!(e, CommandError::TooLarge);
}

#[tokio::test]
async fn unreadable_missing_column_and_empty_inputs() {
    let tmp = tempfile::tempdir().unwrap();
    let c = clock(2026, 1, 6);
    let missing = tmp.path().join("nope.csv");
    assert_eq!(
        import(tmp.path(), &missing, &c).await,
        Err(CommandError::Unreadable)
    );
    assert_eq!(
        import(tmp.path(), tmp.path(), &c).await,
        Err(CommandError::Unreadable)
    );
    let bad = tmp.path().join("bad.csv");
    std::fs::write(&bad, "a,b\n1,2\n").unwrap();
    assert_eq!(import(tmp.path(), &bad, &c).await, Err(CommandError::Parse));
    let header_only = tmp.path().join("h.csv");
    let hdr = "instrument_token,exchange_token,tradingsymbol,name,expiry,strike,tick_size,lot_size,instrument_type,segment,exchange\n";
    std::fs::write(&header_only, hdr).unwrap();
    assert_eq!(
        import(tmp.path(), &header_only, &c).await,
        Err(CommandError::NoValidRows)
    );
}

#[tokio::test]
async fn row_cap_is_enforced() {
    let tmp = tempfile::tempdir().unwrap();
    let limits = ParseLimits {
        max_rows: 1,
        ..ParseLimits::default()
    };
    let e = import_instruments(tmp.path(), &good(), &clock(2026, 1, 6), &limits)
        .await
        .unwrap_err();
    assert_eq!(e, CommandError::Parse);
}

#[tokio::test]
async fn older_dated_import_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    import(tmp.path(), &good(), &clock(2026, 1, 8))
        .await
        .unwrap();
    let e = import(tmp.path(), &good(), &clock(2026, 1, 6))
        .await
        .unwrap_err();
    assert_eq!(e, CommandError::Store("other"));
}

#[cfg(unix)]
#[tokio::test]
async fn data_dir_is_created_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("a").join("data");
    import(&dir, &good(), &clock(2026, 1, 6)).await.unwrap();
    let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o700);
}

#[cfg(unix)]
#[tokio::test]
async fn existing_data_dir_keeps_its_mode() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    import(tmp.path(), &good(), &clock(2026, 1, 6))
        .await
        .unwrap();
    let mode = std::fs::metadata(tmp.path()).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o755);
}
