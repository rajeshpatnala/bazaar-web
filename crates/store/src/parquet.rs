//! Parquet writer for closed candles (`DuckDB` does the encoding; no sqlx here).
//!
//! Layout: `<root>/<symbol>/<interval>_<raw|adj>.parquet`, one file per `(symbol, interval,
//! adjusted)`. A write merges into the existing file (new row wins per close time), writes
//! `<file>.parquet.tmp`, fsyncs it, renames over the target and fsyncs the directory. Readers
//! only open the exact `.parquet` name, so a leftover `.tmp` is ignored. Prices are stored as
//! decimal text (exact), so a damaged value is `RepoError::Corrupt`, never a zero. The symbol is
//! checked against [`safe_symbol`] before it touches a path and is never formatted into SQL; the
//! only text put into SQL is a path built from the allowlisted symbol and constants under a root
//! that contains no quote character.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use bazaar_core::candle::{Candle, Interval};
use bazaar_core::ports::{CandleSink, RepoError};
use chrono::TimeZone;
use chrono_tz::Asia::Kolkata;
use duckdb::{Connection, params};
use rust_decimal::Decimal;

use crate::candles::interval_label;

const MAX_SYMBOL_LEN: usize = 64;
const MAX_BATCH: usize = 100_000;
/// Rows kept in one file (the merge holds them in memory).
const MAX_FILE_ROWS: usize = 5_000_000;

/// Symbols allowed in a file path: ASCII uppercase letters, digits, `_`, `-`, `&` (e.g. `M&M`,
/// `BAJAJ-AUTO`). No dots, slashes, spaces or quotes: no traversal and nothing SQL-significant.
/// Uppercase only, so `abc` and `ABC` can never share a file on a case-insensitive filesystem.
pub(crate) fn safe_symbol(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_SYMBOL_LEN
        && s.bytes().all(|b| {
            b.is_ascii_uppercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'&')
        })
}

/// Writer over a Parquet directory tree.
///
/// Use one store per root: the lock that serialises read-merge-rename is per store, so a second
/// store (or process) writing the same root can lose an update.
#[derive(Debug, Clone)]
pub struct ParquetStore {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    root: PathBuf,
    /// Serialises writers so two merges never race on one file.
    lock: Mutex<()>,
}

impl ParquetStore {
    /// Store rooted at `root` (created on first write).
    ///
    /// # Errors
    /// `RepoError::Other` if the path is not UTF-8 or contains a quote or a control character
    /// (it ends up inside one SQL string).
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, RepoError> {
        let root = check_root(root.into())?;
        Ok(Self {
            inner: Arc::new(Inner {
                root,
                lock: Mutex::new(()),
            }),
        })
    }

    fn file_path(&self, symbol: &str, interval: Interval, adjusted: bool) -> PathBuf {
        file_path(&self.inner.root, symbol, interval, adjusted)
    }

    fn write_sync(&self, symbol: &str, candles: &[Candle]) -> Result<u64, RepoError> {
        if !safe_symbol(symbol) {
            return Err(RepoError::Other);
        }
        if candles.len() > MAX_BATCH {
            return Err(RepoError::TooLarge);
        }
        let Some(first) = candles.first() else {
            return Ok(0);
        };
        if candles
            .iter()
            .any(|c| c.interval() != first.interval() || c.adjusted() != first.adjusted())
        {
            return Err(RepoError::Conflict);
        }
        let mut batch: BTreeMap<i64, Row> = BTreeMap::new();
        for c in candles {
            let row = Row::from_candle(c)?;
            batch.insert(row.close_ns, row);
        }
        let _guard = self.inner.lock.lock().map_err(|_| RepoError::Other)?;
        let target = self.file_path(symbol, first.interval(), first.adjusted());
        let dir = target.parent().ok_or(RepoError::Other)?;
        fs::create_dir_all(dir).map_err(|_| RepoError::Unavailable)?;
        let conn = Connection::open_in_memory().map_err(|_| RepoError::Unavailable)?;
        // Existing rows first so the batch overwrites them; a corrupt file refuses the write.
        let mut merged = if target.is_file() {
            read_rows(&conn, &target)?
        } else {
            BTreeMap::new()
        };
        let written = batch.len();
        merged.extend(batch);
        if merged.len() > MAX_FILE_ROWS {
            return Err(RepoError::TooLarge);
        }
        let tmp = target.with_extension("parquet.tmp");
        // A leftover from a crashed write is stale by definition: replace it.
        let _ = fs::remove_file(&tmp);
        let result = write_tmp(&conn, &tmp, &merged)
            .and_then(|()| sync_path(&tmp))
            .and_then(|()| fs::rename(&tmp, &target).map_err(|_| RepoError::Other))
            .and_then(|()| sync_path(dir));
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result?;
        u64::try_from(written).map_err(|_| RepoError::Other)
    }
}

impl CandleSink for ParquetStore {
    async fn write_candles(&self, symbol: &str, candles: &[Candle]) -> Result<u64, RepoError> {
        // DuckDB, fsync and the merge block: keep them off the async executor.
        let store = self.clone();
        let symbol = symbol.to_owned();
        let candles = candles.to_vec();
        tokio::task::spawn_blocking(move || store.write_sync(&symbol, &candles))
            .await
            .map_err(|_| RepoError::Other)?
    }
}

/// Root must be UTF-8 and free of quotes and control characters: it ends up inside SQL text.
/// Invariant relied on by `write_tmp`: `DuckDB` single-quoted strings have no backslash escapes,
/// so with no quote and no control character the path cannot end the literal.
pub(crate) fn check_root(root: PathBuf) -> Result<PathBuf, RepoError> {
    let text = root.to_str().ok_or(RepoError::Other)?;
    if text.contains('\'') || text.chars().any(char::is_control) {
        return Err(RepoError::Other);
    }
    Ok(root)
}

/// `<root>/<symbol>/<interval>_<raw|adj>.parquet`; the caller has already checked the symbol.
pub(crate) fn file_path(root: &Path, symbol: &str, interval: Interval, adjusted: bool) -> PathBuf {
    let kind = if adjusted { "adj" } else { "raw" };
    root.join(symbol)
        .join(format!("{}_{kind}.parquet", interval_label(interval)))
}

#[derive(Debug, Clone)]
pub(crate) struct Row {
    pub(crate) close_ns: i64,
    pub(crate) ohlc: [String; 4],
    pub(crate) volume: i64,
}

impl Row {
    fn from_candle(c: &Candle) -> Result<Self, RepoError> {
        Ok(Self {
            close_ns: c
                .close_time()
                .timestamp_nanos_opt()
                .ok_or(RepoError::Other)?,
            ohlc: [c.open(), c.high(), c.low(), c.close()].map(|d| d.to_string()),
            volume: i64::try_from(c.volume()).map_err(|_| RepoError::Other)?,
        })
    }

    pub(crate) fn to_candle(
        &self,
        interval: Interval,
        adjusted: bool,
    ) -> Result<Candle, RepoError> {
        let d = |s: &String| Decimal::from_str(s).map_err(|_| RepoError::Corrupt);
        let [open, high, low, close] = &self.ohlc;
        Candle::new(
            Kolkata.timestamp_nanos(self.close_ns),
            interval,
            [d(open)?, d(high)?, d(low)?, d(close)?],
            u64::try_from(self.volume).map_err(|_| RepoError::Corrupt)?,
            adjusted,
        )
        .map_err(|_| RepoError::Corrupt)
    }
}

fn read_rows(conn: &Connection, file: &Path) -> Result<BTreeMap<i64, Row>, RepoError> {
    let path = file.to_str().ok_or(RepoError::Other)?;
    let corrupt = |_| RepoError::Corrupt;
    let mut stmt = conn
        .prepare("SELECT close_ns, open, high, low, close, volume FROM read_parquet(?)")
        .map_err(corrupt)?;
    let mapped = stmt
        .query_map(params![path], |r| {
            Ok(Row {
                close_ns: r.get(0)?,
                ohlc: [r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?],
                volume: r.get(5)?,
            })
        })
        .map_err(corrupt)?;
    let mut out = BTreeMap::new();
    for row in mapped {
        let row = row.map_err(corrupt)?;
        // Validate on the way in so a bad stored value is never re-written as good data.
        row.to_candle(Interval::Day, false)?;
        out.insert(row.close_ns, row);
    }
    Ok(out)
}

fn write_tmp(conn: &Connection, tmp: &Path, rows: &BTreeMap<i64, Row>) -> Result<(), RepoError> {
    let other = |_| RepoError::Other;
    conn.execute_batch(
        "CREATE OR REPLACE TABLE c (close_ns BIGINT, open VARCHAR, high VARCHAR, \
         low VARCHAR, close VARCHAR, volume BIGINT)",
    )
    .map_err(other)?;
    {
        let mut app = conn.appender("c").map_err(other)?;
        for r in rows.values() {
            let [open, high, low, close] = &r.ohlc;
            app.append_row(params![r.close_ns, open, high, low, close, r.volume])
                .map_err(other)?;
        }
        app.flush().map_err(other)?;
    }
    let path = tmp.to_str().ok_or(RepoError::Other)?;
    // The path is text inside the SQL: safe only because `ParquetStore::new` rejects quotes and
    // control characters in the root and `safe_symbol` constrains the rest.
    conn.execute_batch(&format!(
        "COPY (SELECT * FROM c ORDER BY close_ns) TO '{path}' (FORMAT PARQUET)"
    ))
    .map_err(other)
}

/// fsync a file or a directory.
fn sync_path(p: &Path) -> Result<(), RepoError> {
    File::open(p)
        .and_then(|f| f.sync_all())
        .map_err(|_| RepoError::Other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;
    use chrono_tz::Tz;

    fn t(m: u32) -> DateTime<Tz> {
        Kolkata.with_ymd_and_hms(2026, 1, 5, 9, m, 0).unwrap()
    }

    fn bar(m: u32, px: i64, adjusted: bool) -> Candle {
        let p = Decimal::new(px, 2);
        Candle::new(t(m), Interval::Min5, [p; 4], 7, adjusted).unwrap()
    }

    fn read(s: &ParquetStore, sym: &str, adj: bool) -> Result<Vec<Candle>, RepoError> {
        let conn = Connection::open_in_memory().unwrap();
        read_rows(&conn, &s.file_path(sym, Interval::Min5, adj))?
            .values()
            .map(|r| r.to_candle(Interval::Min5, adj))
            .collect()
    }

    fn store() -> (tempfile::TempDir, ParquetStore) {
        let d = tempfile::tempdir().unwrap();
        let s = ParquetStore::new(d.path()).unwrap();
        (d, s)
    }

    #[tokio::test]
    async fn round_trips_exact_decimals_oldest_first() {
        let (_d, s) = store();
        let n = s
            .write_candles("M&M", &[bar(25, 123_450, false), bar(20, 10_005, false)])
            .await
            .unwrap();
        assert_eq!(n, 2);
        let got = read(&s, "M&M", false).unwrap();
        assert_eq!(got, vec![bar(20, 10_005, false), bar(25, 123_450, false)]);
        assert_eq!(got[1].open().to_string(), "1234.50");
    }

    #[tokio::test]
    async fn overlapping_write_new_row_wins_without_duplicates() {
        let (_d, s) = store();
        s.write_candles("ABC", &[bar(20, 100, false), bar(25, 200, false)])
            .await
            .unwrap();
        s.write_candles("ABC", &[bar(25, 300, false), bar(30, 400, false)])
            .await
            .unwrap();
        assert_eq!(
            read(&s, "ABC", false).unwrap(),
            vec![
                bar(20, 100, false),
                bar(25, 300, false),
                bar(30, 400, false)
            ]
        );
    }

    #[tokio::test]
    async fn adjusted_and_raw_are_separate_and_mixed_batch_is_refused() {
        let (_d, s) = store();
        s.write_candles("ABC", &[bar(20, 100, false)])
            .await
            .unwrap();
        s.write_candles("ABC", &[bar(20, 90, true)]).await.unwrap();
        assert_eq!(read(&s, "ABC", false).unwrap(), vec![bar(20, 100, false)]);
        assert_eq!(read(&s, "ABC", true).unwrap(), vec![bar(20, 90, true)]);
        let mixed = [bar(30, 1, false), bar(35, 1, true)];
        assert_eq!(
            s.write_candles("ABC", &mixed).await,
            Err(RepoError::Conflict)
        );
    }

    #[tokio::test]
    async fn hostile_symbols_are_refused_and_nothing_is_written() {
        let (d, s) = store();
        let long = "A".repeat(65);
        for bad in [
            "", "../x", "a/b", "a b", "A'B", "a.b", "..", "x;DROP", &long,
        ] {
            assert_eq!(
                s.write_candles(bad, &[bar(20, 1, false)]).await,
                Err(RepoError::Other),
                "{bad}"
            );
        }
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 0);
        assert!(safe_symbol("BAJAJ-AUTO") && safe_symbol("M&M") && safe_symbol("A_1"));
    }

    #[tokio::test]
    async fn empty_batch_writes_nothing_and_quote_in_root_is_refused() {
        let (d, s) = store();
        assert_eq!(s.write_candles("ABC", &[]).await, Ok(0));
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 0);
        assert!(ParquetStore::new("/tmp/a'b").is_err());
        assert!(ParquetStore::new("/tmp/a\nb").is_err());
    }

    #[tokio::test]
    async fn leftover_tmp_is_ignored_and_replaced_by_the_next_write() {
        let (_d, s) = store();
        s.write_candles("ABC", &[bar(20, 100, false)])
            .await
            .unwrap();
        let target = s.file_path("ABC", Interval::Min5, false);
        let tmp = target.with_extension("parquet.tmp");
        fs::write(&tmp, b"half written garbage").unwrap();
        assert_eq!(read(&s, "ABC", false).unwrap(), vec![bar(20, 100, false)]);
        s.write_candles("ABC", &[bar(25, 200, false)])
            .await
            .unwrap();
        assert_eq!(read(&s, "ABC", false).unwrap().len(), 2);
        assert!(!tmp.exists());
    }

    #[tokio::test]
    async fn corrupt_file_or_value_refuses_the_write_and_keeps_the_file() {
        let (_d, s) = store();
        s.write_candles("ABC", &[bar(20, 100, false)])
            .await
            .unwrap();
        let target = s.file_path("ABC", Interval::Min5, false);
        fs::write(&target, b"junk").unwrap();
        assert_eq!(
            s.write_candles("ABC", &[bar(25, 1, false)]).await,
            Err(RepoError::Corrupt)
        );
        assert_eq!(fs::read(&target).unwrap(), b"junk");
        // a valid parquet file whose price is not a decimal
        fs::remove_file(&target).unwrap();
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(&format!(
            "COPY (SELECT 1::BIGINT AS close_ns, 'junk' AS open, '1' AS high, '1' AS low, \
             '1' AS close, 1::BIGINT AS volume) TO '{}' (FORMAT PARQUET)",
            target.to_str().unwrap()
        ))
        .unwrap();
        assert_eq!(
            s.write_candles("ABC", &[bar(25, 1, false)]).await,
            Err(RepoError::Corrupt)
        );
        assert_eq!(read(&s, "ABC", false), Err(RepoError::Corrupt));
    }

    #[tokio::test]
    async fn failed_write_leaves_earlier_data() {
        let (_d, s) = store();
        s.write_candles("ABC", &[bar(20, 100, false)])
            .await
            .unwrap();
        let target = s.file_path("ABC", Interval::Min5, false);
        // a directory squatting on the tmp name cannot be removed as a file, so the COPY fails
        let tmp = target.with_extension("parquet.tmp");
        fs::create_dir(&tmp).unwrap();
        assert!(s.write_candles("ABC", &[bar(25, 1, false)]).await.is_err());
        assert_eq!(read(&s, "ABC", false).unwrap(), vec![bar(20, 100, false)]);
    }
}
