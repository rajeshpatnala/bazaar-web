//! F12 T2 acceptance gap-fillers, run on BOTH adapters. Temp dirs only.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]

mod common;

use bazaar_core::candle::{Candle, Interval};
use bazaar_core::ports::{CandleSink, CandleSource, RepoError};
use bazaar_store::{DuckStore, ParquetStore, SqliteStore};
use common::{at, bar, fixture_candles, range};

fn all(dir: &std::path::Path) -> Vec<String> {
    let mut out = vec![];
    let mut stack = vec![dir.to_path_buf()];
    while let Some(p) = stack.pop() {
        for e in std::fs::read_dir(&p).unwrap() {
            let e = e.unwrap().path();
            if e.is_dir() {
                stack.push(e);
            } else {
                out.push(e.strip_prefix(dir).unwrap().to_string_lossy().into_owned());
            }
        }
    }
    out.sort();
    out
}

fn long_symbol() -> String {
    "A".repeat(65)
}

#[tokio::test]
async fn parquet_failed_write_leaves_earlier_data_intact() {
    let dir = tempfile::tempdir().unwrap();
    let w = ParquetStore::new(dir.path()).unwrap();
    let r = DuckStore::new(dir.path()).unwrap();
    w.write_candles("ABC", &fixture_candles()).await.unwrap();
    // refused batch (mixed adjusted)
    let mixed = [
        bar(at(9, 20), "100.00", 1, false),
        bar(at(9, 25), "100.00", 1, true),
    ];
    assert_eq!(
        w.write_candles("ABC", &mixed).await,
        Err(RepoError::Conflict)
    );
    // refused symbol
    assert!(w.write_candles("../ABC", &fixture_candles()).await.is_err());
    // corrupt existing file refuses the write and is left untouched
    let f = dir.path().join("ABC").join("min5_adj.parquet");
    std::fs::write(&f, b"junk").unwrap();
    let res = w
        .write_candles("ABC", &[bar(at(9, 20), "100.00", 1, true)])
        .await;
    assert_eq!(res, Err(RepoError::Corrupt));
    assert_eq!(std::fs::read(&f).unwrap(), b"junk");
    assert!(!all(dir.path()).iter().any(|n| n.ends_with(".tmp")));
    let got = r
        .read_candles(&range("ABC", at(9, 0), at(15, 30), false))
        .await
        .unwrap();
    assert_eq!(got, fixture_candles());
}

#[tokio::test]
async fn parquet_crash_leftover_tmp_is_ignored_then_replaced_by_next_write() {
    let dir = tempfile::tempdir().unwrap();
    let w = ParquetStore::new(dir.path()).unwrap();
    let r = DuckStore::new(dir.path()).unwrap();
    w.write_candles("ABC", &fixture_candles()).await.unwrap();
    let tmp = dir.path().join("ABC").join("min5_raw.parquet.tmp");
    std::fs::write(&tmp, b"half").unwrap();
    assert_eq!(
        r.last_candle_ts("ABC", Interval::Min5, false)
            .await
            .unwrap(),
        Some(at(9, 35))
    );
    w.write_candles("ABC", &[bar(at(9, 40), "100.00", 9, false)])
        .await
        .unwrap();
    assert!(!tmp.exists());
    let got = r
        .read_candles(&range("ABC", at(9, 0), at(15, 30), false))
        .await
        .unwrap();
    assert_eq!(got.len(), 5);
}

#[tokio::test]
async fn hostile_symbols_on_write_are_refused_and_create_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let w = ParquetStore::new(dir.path().join("root")).unwrap();
    let long = long_symbol();
    let bad = [
        "..",
        "../ABC",
        "A/B",
        "A\\B",
        "abc",
        "a'b",
        "A'; DROP TABLE c; --",
        "",
        "A.B",
        "A B",
        "A\0B",
        long.as_str(),
    ];
    for s in bad {
        assert!(
            w.write_candles(s, &fixture_candles()).await.is_err(),
            "parquet write accepted {s:?}"
        );
    }
    assert!(all(dir.path()).is_empty());
    // allowed shapes with & and - work
    w.write_candles("M&M", &fixture_candles()).await.unwrap();
    w.write_candles("BAJAJ-AUTO", &fixture_candles())
        .await
        .unwrap();
}

#[tokio::test]
async fn hostile_symbols_refused_on_read_both_adapters() {
    let dir = tempfile::tempdir().unwrap();
    let r = DuckStore::new(dir.path()).unwrap();
    let s = SqliteStore::open_in_memory().await.unwrap();
    let long = long_symbol();
    for bad in ["..", "../ABC", "A/B", "abc", "a'b", long.as_str()] {
        assert!(
            r.read_candles(&range(bad, at(9, 0), at(15, 30), false))
                .await
                .is_err(),
            "{bad}"
        );
        assert!(
            r.last_candle_ts(bad, Interval::Min5, false).await.is_err(),
            "{bad}"
        );
    }
    // SQLite: oversize refused, everything else is plain data and reads empty
    assert_eq!(
        s.read_candles(&range(&long, at(9, 0), at(15, 30), false))
            .await,
        Err(RepoError::TooLarge)
    );
    for ok in ["..", "a'b", "abc"] {
        assert!(
            s.read_candles(&range(ok, at(9, 0), at(15, 30), false))
                .await
                .unwrap()
                .is_empty()
        );
    }
}

#[tokio::test]
async fn sqlite_hostile_symbol_write_is_isolated_data() {
    let s = SqliteStore::open_in_memory().await.unwrap();
    for sym in ["..", "a'b", "x'); DROP TABLE candles;--"] {
        s.write_candles(sym, &fixture_candles()).await.unwrap();
        assert_eq!(
            s.read_candles(&range(sym, at(9, 0), at(15, 30), false))
                .await
                .unwrap(),
            fixture_candles()
        );
    }
    s.write_candles("ABC", &fixture_candles()).await.unwrap();
    assert_eq!(
        s.read_candles(&range("ABC", at(9, 0), at(15, 30), false))
            .await
            .unwrap()
            .len(),
        4
    );
    assert_eq!(
        s.write_candles(&long_symbol(), &fixture_candles()).await,
        Err(RepoError::TooLarge)
    );
}

#[tokio::test]
async fn parquet_adjusted_and_raw_live_in_separate_files_and_never_mix() {
    let dir = tempfile::tempdir().unwrap();
    let w = ParquetStore::new(dir.path()).unwrap();
    let r = DuckStore::new(dir.path()).unwrap();
    w.write_candles("ABC", &fixture_candles()).await.unwrap();
    let adj: Vec<Candle> = (0..2)
        .map(|i| bar(at(9, 20 + 5 * i), "100.00", 3, true))
        .collect();
    w.write_candles("ABC", &adj).await.unwrap();
    assert_eq!(
        all(dir.path()),
        ["ABC/min5_adj.parquet", "ABC/min5_raw.parquet"]
    );
    let a = r
        .read_candles(&range("ABC", at(9, 0), at(15, 30), true))
        .await
        .unwrap();
    assert_eq!(a, adj);
    assert_eq!(
        r.last_candle_ts("ABC", Interval::Min5, true).await.unwrap(),
        Some(at(9, 25))
    );
    assert_eq!(
        r.last_candle_ts("ABC", Interval::Min5, false)
            .await
            .unwrap(),
        Some(at(9, 35))
    );
}

#[tokio::test]
async fn last_candle_ts_never_lags_a_write_on_both_adapters() {
    let dir = tempfile::tempdir().unwrap();
    let w = ParquetStore::new(dir.path()).unwrap();
    let r = DuckStore::new(dir.path()).unwrap();
    let s = SqliteStore::open_in_memory().await.unwrap();
    for i in 0..4u32 {
        let c = [bar(at(10, 5 * i), "100.00", 1, false)];
        w.write_candles("ABC", &c).await.unwrap();
        s.write_candles("ABC", &c).await.unwrap();
        let want = Some(at(10, 5 * i));
        assert_eq!(
            r.last_candle_ts("ABC", Interval::Min5, false)
                .await
                .unwrap(),
            want
        );
        assert_eq!(
            s.last_candle_ts("ABC", Interval::Min5, false)
                .await
                .unwrap(),
            want
        );
    }
}

#[tokio::test]
async fn parquet_equivalence_with_sqlite_on_empty_and_adjusted() {
    let dir = tempfile::tempdir().unwrap();
    let w = ParquetStore::new(dir.path()).unwrap();
    let r = DuckStore::new(dir.path()).unwrap();
    let s = SqliteStore::open_in_memory().await.unwrap();
    let adj = [
        bar(at(9, 20), "100.10", 2, true),
        bar(at(9, 25), "100.20", 3, true),
    ];
    w.write_candles("ABC", &adj).await.unwrap();
    s.write_candles("ABC", &adj).await.unwrap();
    for (sym, a) in [("ABC", true), ("ABC", false), ("NOPE", false)] {
        let q = range(sym, at(9, 0), at(15, 30), a);
        assert_eq!(
            r.read_candles(&q).await,
            s.read_candles(&q).await,
            "{sym} {a}"
        );
    }
}

#[test]
fn core_ports_is_free_of_sqlx_and_duckdb_and_core_does_not_depend_on_them() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../core");
    let ports = std::fs::read_to_string(root.join("src/ports.rs")).unwrap();
    for word in ["sqlx", "duckdb", "rusqlite", "parquet", "arrow"] {
        assert!(!ports.contains(word), "core/src/ports.rs mentions {word}");
    }
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    for word in ["sqlx", "duckdb", "bazaar-store"] {
        assert!(!manifest.contains(word), "core/Cargo.toml mentions {word}");
    }
}
