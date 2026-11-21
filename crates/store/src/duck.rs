//! Read side of the Parquet candle store: [`CandleSource`] over the files `ParquetStore` writes.
//!
//! Each read opens its own in-memory `DuckDB` connection on a blocking thread. The file is the
//! exact `<root>/<symbol>/<interval>_<raw|adj>.parquet` name (never a glob over the directory),
//! so a leftover `*.tmp` is never opened. The symbol is allowlisted before it builds the path;
//! the path and the bounds are bound parameters, never formatted into SQL. A value that does not
//! decode is `RepoError::Corrupt`.

use std::path::{Path, PathBuf};

use bazaar_core::candle::{Candle, Interval};
use bazaar_core::ports::{CandleRange, CandleSource, RepoError};
use chrono::{DateTime, TimeZone};
use chrono_tz::{Asia::Kolkata, Tz};
use duckdb::{Connection, params};

use crate::parquet::{Row, check_root, file_path, safe_symbol};

/// Most rows one read may return (local memory guard, same as the `SQLite` adapter).
const MAX_READ_ROWS: usize = 1_000_000;

const READ: &str = "SELECT close_ns, open, high, low, close, volume FROM read_parquet(?) \
    WHERE close_ns >= ? AND close_ns <= ? ORDER BY close_ns LIMIT ?";
const LAST: &str = "SELECT MAX(close_ns) FROM read_parquet(?)";

/// Reader over a Parquet directory tree written by `ParquetStore`.
#[derive(Debug, Clone)]
pub struct DuckStore {
    root: PathBuf,
}

impl DuckStore {
    /// Reader rooted at `root`.
    ///
    /// # Errors
    /// `RepoError::Other` if the path is not UTF-8 or holds a quote or control character.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, RepoError> {
        Ok(Self {
            root: check_root(root.into())?,
        })
    }

    /// Liveness check for `check-connections` (`OK duckdb`): the engine starts and the Parquet
    /// reader is loaded. Touches no data file.
    ///
    /// # Errors
    /// `RepoError::Unavailable` if the engine or its Parquet support is missing.
    pub async fn ping(&self) -> Result<(), RepoError> {
        tokio::task::spawn_blocking(|| {
            let conn = Connection::open_in_memory().map_err(|_| RepoError::Unavailable)?;
            let loaded: i64 = conn
                .query_row(
                    "SELECT count(*) FROM duckdb_extensions() \
                     WHERE extension_name = 'parquet' AND loaded",
                    [],
                    |r| r.get(0),
                )
                .map_err(|_| RepoError::Unavailable)?;
            if loaded == 1 {
                Ok(())
            } else {
                Err(RepoError::Unavailable)
            }
        })
        .await
        .map_err(|_| RepoError::Unavailable)?
    }
}

/// The data file for a key, `None` if it does not exist yet. Refuses an unsafe symbol.
fn existing(
    root: &Path,
    symbol: &str,
    interval: Interval,
    adjusted: bool,
) -> Result<Option<String>, RepoError> {
    if !safe_symbol(symbol) {
        return Err(RepoError::Other);
    }
    let file = file_path(root, symbol, interval, adjusted);
    if !file.is_file() {
        return Ok(None);
    }
    file.to_str()
        .map(str::to_owned)
        .map(Some)
        .ok_or(RepoError::Other)
}

fn read_sync(root: &Path, range: &CandleRange, cap: usize) -> Result<Vec<Candle>, RepoError> {
    let Some(path) = existing(root, &range.symbol, range.interval, range.adjusted)? else {
        return Ok(vec![]);
    };
    let from = range.from.timestamp_nanos_opt().ok_or(RepoError::Other)?;
    let to = range.to.timestamp_nanos_opt().ok_or(RepoError::Other)?;
    let limit = i64::try_from(cap + 1).map_err(|_| RepoError::Other)?;
    let corrupt = |_| RepoError::Corrupt;
    let conn = Connection::open_in_memory().map_err(|_| RepoError::Unavailable)?;
    let mut stmt = conn.prepare(READ).map_err(corrupt)?;
    let rows = stmt
        .query_map(params![path, from, to, limit], |r| {
            Ok(Row {
                close_ns: r.get(0)?,
                ohlc: [r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?],
                volume: r.get(5)?,
            })
        })
        .map_err(corrupt)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(
            row.map_err(corrupt)?
                .to_candle(range.interval, range.adjusted)?,
        );
    }
    if out.len() > cap {
        return Err(RepoError::TooLarge);
    }
    Ok(out)
}

fn last_sync(
    root: &Path,
    symbol: &str,
    interval: Interval,
    adjusted: bool,
) -> Result<Option<DateTime<Tz>>, RepoError> {
    let Some(path) = existing(root, symbol, interval, adjusted)? else {
        return Ok(None);
    };
    let conn = Connection::open_in_memory().map_err(|_| RepoError::Unavailable)?;
    let last: Option<i64> = conn
        .query_row(LAST, params![path], |r| r.get(0))
        .map_err(|_| RepoError::Corrupt)?;
    Ok(last.map(|ns| Kolkata.timestamp_nanos(ns)))
}

impl CandleSource for DuckStore {
    async fn read_candles(&self, range: &CandleRange) -> Result<Vec<Candle>, RepoError> {
        // DuckDB and file reads block: keep them off the async executor.
        let (root, range) = (self.root.clone(), range.clone());
        tokio::task::spawn_blocking(move || read_sync(&root, &range, MAX_READ_ROWS))
            .await
            .map_err(|_| RepoError::Other)?
    }

    async fn last_candle_ts(
        &self,
        symbol: &str,
        interval: Interval,
        adjusted: bool,
    ) -> Result<Option<DateTime<Tz>>, RepoError> {
        let (root, symbol) = (self.root.clone(), symbol.to_owned());
        tokio::task::spawn_blocking(move || last_sync(&root, &symbol, interval, adjusted))
            .await
            .map_err(|_| RepoError::Other)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bazaar_core::ports::CandleSink;
    use rust_decimal::Decimal;

    fn t(m: u32) -> DateTime<Tz> {
        Kolkata.with_ymd_and_hms(2026, 1, 5, 9, m, 0).unwrap()
    }

    fn bar(m: u32) -> Candle {
        let p = Decimal::new(10_000, 2);
        Candle::new(t(m), Interval::Min5, [p; 4], 1, false).unwrap()
    }

    #[tokio::test]
    async fn read_cap_is_enforced_and_exact_cap_passes() {
        let d = tempfile::tempdir().unwrap();
        let w = crate::ParquetStore::new(d.path()).unwrap();
        w.write_candles("ABC", &[bar(20), bar(25), bar(30)])
            .await
            .unwrap();
        let r = CandleRange::new("ABC", Interval::Min5, t(0), t(59), false);
        assert_eq!(read_sync(d.path(), &r, 3).unwrap().len(), 3);
        assert_eq!(read_sync(d.path(), &r, 2), Err(RepoError::TooLarge));
    }

    #[test]
    fn bad_symbol_and_bad_root_are_refused() {
        let d = tempfile::tempdir().unwrap();
        for bad in ["", "../x", "abc", "A'B", "a b"] {
            let r = CandleRange::new(bad, Interval::Min5, t(0), t(59), false);
            assert_eq!(read_sync(d.path(), &r, 10), Err(RepoError::Other), "{bad}");
            assert_eq!(
                last_sync(d.path(), bad, Interval::Min5, false),
                Err(RepoError::Other),
                "{bad}"
            );
        }
        assert!(DuckStore::new("/tmp/a'b").is_err());
    }

    #[tokio::test]
    async fn ping_is_ok() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(DuckStore::new(d.path()).unwrap().ping().await, Ok(()));
    }
}
