//! SQLite implementation of [`CandleSource`] and [`CandleSink`].
//!
//! The symbol is always a bound parameter. A write is one transaction. Any stored value that
//! does not decode into a valid `Candle` is `RepoError::Corrupt`, never a zero or a panic.

use std::str::FromStr;

use bazaar_core::candle::{Candle, Interval};
use bazaar_core::ports::{CandleRange, CandleSink, CandleSource, RepoError};
use chrono::{DateTime, TimeZone};
use chrono_tz::{Asia::Kolkata, Tz};
use rust_decimal::Decimal;
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use crate::sqlite::{SqliteStore, map_err};

/// Longest accepted symbol, most rows per write and per read (local memory/disk guard).
const MAX_SYMBOL_LEN: usize = 64;
const MAX_BATCH: usize = 100_000;
const MAX_READ_ROWS: usize = 1_000_000;

fn check_symbol(symbol: &str) -> Result<(), RepoError> {
    if symbol.is_empty() || symbol.len() > MAX_SYMBOL_LEN {
        return Err(RepoError::TooLarge);
    }
    Ok(())
}

const UPSERT: &str = "INSERT INTO candles (symbol, interval, adjusted, close_ns, open, high, \
    low, close, volume) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
    ON CONFLICT (symbol, interval, adjusted, close_ns) DO UPDATE SET open = excluded.open, \
    high = excluded.high, low = excluded.low, close = excluded.close, volume = excluded.volume";

const READ: &str = "SELECT interval, adjusted, close_ns, open, high, low, close, volume \
    FROM candles WHERE symbol = ? AND interval = ? AND adjusted = ? \
    AND close_ns >= ? AND close_ns <= ? ORDER BY close_ns LIMIT ?";

const LAST: &str = "SELECT MAX(close_ns) AS last FROM candles \
    WHERE symbol = ? AND interval = ? AND adjusted = ?";

const fn interval_label(i: Interval) -> &'static str {
    match i {
        Interval::Min1 => "min1",
        Interval::Min3 => "min3",
        Interval::Min5 => "min5",
        Interval::Min10 => "min10",
        Interval::Min15 => "min15",
        Interval::Min30 => "min30",
        Interval::Min60 => "min60",
        Interval::Day => "day",
    }
}

fn parse_interval(s: &str) -> Result<Interval, RepoError> {
    [
        Interval::Min1,
        Interval::Min3,
        Interval::Min5,
        Interval::Min10,
        Interval::Min15,
        Interval::Min30,
        Interval::Min60,
        Interval::Day,
    ]
    .into_iter()
    .find(|i| interval_label(*i) == s)
    .ok_or(RepoError::Corrupt)
}

fn to_ns(t: &DateTime<Tz>) -> Result<i64, RepoError> {
    t.timestamp_nanos_opt().ok_or(RepoError::Other)
}

fn from_ns(ns: i64) -> DateTime<Tz> {
    Kolkata.timestamp_nanos(ns)
}

fn dec(row: &SqliteRow, col: &str) -> Result<Decimal, RepoError> {
    let s: String = row.try_get(col).map_err(|_| RepoError::Corrupt)?;
    Decimal::from_str(&s).map_err(|_| RepoError::Corrupt)
}

fn to_candle(row: &SqliteRow) -> Result<Candle, RepoError> {
    let corrupt = |_| RepoError::Corrupt;
    let interval: String = row.try_get("interval").map_err(corrupt)?;
    let adjusted: i64 = row.try_get("adjusted").map_err(corrupt)?;
    let ns: i64 = row.try_get("close_ns").map_err(corrupt)?;
    let volume: i64 = row.try_get("volume").map_err(corrupt)?;
    Candle::new(
        from_ns(ns),
        parse_interval(&interval)?,
        [
            dec(row, "open")?,
            dec(row, "high")?,
            dec(row, "low")?,
            dec(row, "close")?,
        ],
        u64::try_from(volume).map_err(|_| RepoError::Corrupt)?,
        adjusted != 0,
    )
    .map_err(|_| RepoError::Corrupt)
}

impl CandleSink for SqliteStore {
    async fn write_candles(&self, symbol: &str, candles: &[Candle]) -> Result<u64, RepoError> {
        check_symbol(symbol)?;
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
        // Convert everything before opening the transaction: nothing partial on a bad row.
        let mut rows = Vec::with_capacity(candles.len());
        for c in candles {
            let volume = i64::try_from(c.volume()).map_err(|_| RepoError::Other)?;
            rows.push((to_ns(&c.close_time())?, c, volume));
        }
        let mut tx = self.pool.begin().await.map_err(|e| map_err(&e))?;
        for (ns, c, volume) in &rows {
            sqlx::query(UPSERT)
                .bind(symbol)
                .bind(interval_label(c.interval()))
                .bind(i64::from(c.adjusted()))
                .bind(ns)
                .bind(c.open().to_string())
                .bind(c.high().to_string())
                .bind(c.low().to_string())
                .bind(c.close().to_string())
                .bind(volume)
                .execute(&mut *tx)
                .await
                .map_err(|e| map_err(&e))?;
        }
        tx.commit().await.map_err(|e| map_err(&e))?;
        // Rows upserted (inserted or replaced), not net new rows.
        u64::try_from(rows.len()).map_err(|_| RepoError::Other)
    }
}

impl SqliteStore {
    /// `read_candles` with an injectable row cap (the public path uses `MAX_READ_ROWS`).
    async fn read_capped(&self, range: &CandleRange, cap: usize) -> Result<Vec<Candle>, RepoError> {
        check_symbol(&range.symbol)?;
        let rows = sqlx::query(READ)
            .bind(&range.symbol)
            .bind(interval_label(range.interval))
            .bind(i64::from(range.adjusted))
            .bind(to_ns(&range.from)?)
            .bind(to_ns(&range.to)?)
            .bind(i64::try_from(cap + 1).map_err(|_| RepoError::Other)?)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| map_err(&e))?;
        if rows.len() > cap {
            return Err(RepoError::TooLarge);
        }
        rows.iter().map(to_candle).collect()
    }
}

impl CandleSource for SqliteStore {
    async fn read_candles(&self, range: &CandleRange) -> Result<Vec<Candle>, RepoError> {
        self.read_capped(range, MAX_READ_ROWS).await
    }

    async fn last_candle_ts(
        &self,
        symbol: &str,
        interval: Interval,
        adjusted: bool,
    ) -> Result<Option<DateTime<Tz>>, RepoError> {
        let row = sqlx::query(LAST)
            .bind(symbol)
            .bind(interval_label(interval))
            .bind(i64::from(adjusted))
            .fetch_one(&self.pool)
            .await
            .map_err(|e| map_err(&e))?;
        let last: Option<i64> = row.try_get("last").map_err(|_| RepoError::Corrupt)?;
        Ok(last.map(from_ns))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    fn t(m: u32) -> DateTime<Tz> {
        Kolkata.with_ymd_and_hms(2026, 1, 5, 9, m, 0).unwrap()
    }

    fn bar(m: u32) -> Candle {
        let p = Decimal::from(100);
        Candle::new(t(m), Interval::Min5, [p; 4], 1, false).unwrap()
    }

    fn rng() -> CandleRange {
        CandleRange::new("ABC", Interval::Min5, t(0), t(59), false)
    }

    #[tokio::test]
    async fn read_cap_is_enforced_and_exact_cap_passes() {
        let s = SqliteStore::open_in_memory().await.unwrap();
        s.write_candles("ABC", &[bar(20), bar(25), bar(30)])
            .await
            .unwrap();
        assert_eq!(s.read_capped(&rng(), 3).await.unwrap().len(), 3);
        assert_eq!(s.read_capped(&rng(), 2).await, Err(RepoError::TooLarge));
        assert_eq!(RepoError::TooLarge.safe_reason(), "too_large");
    }

    #[test]
    fn unknown_interval_label_is_corrupt_and_known_ones_round_trip() {
        assert_eq!(parse_interval("bogus"), Err(RepoError::Corrupt));
        assert_eq!(parse_interval(""), Err(RepoError::Corrupt));
        assert_eq!(parse_interval("min5"), Ok(Interval::Min5));
        assert_eq!(parse_interval("day"), Ok(Interval::Day));
    }

    #[tokio::test]
    async fn extreme_close_ns_reads_without_panic_and_last_ts_ignores_bad_prices() {
        let s = SqliteStore::open_in_memory().await.unwrap();
        s.write_candles("ABC", &[bar(20)]).await.unwrap();
        sqlx::query("UPDATE candles SET close = 'junk'")
            .execute(&s.pool)
            .await
            .unwrap();
        // freshness marker only needs the key column, so damaged prices do not hide it
        assert_eq!(
            s.last_candle_ts("ABC", Interval::Min5, false).await,
            Ok(Some(t(20)))
        );
        assert_eq!(s.read_candles(&rng()).await, Err(RepoError::Corrupt));
        // a row at the i64 extremes decodes to a time (or is Corrupt), never a panic
        sqlx::query("UPDATE candles SET close = '100', close_ns = 9223372036854775807")
            .execute(&s.pool)
            .await
            .unwrap();
        let wide = CandleRange::new("ABC", Interval::Min5, t(0), t(0), false);
        assert_eq!(s.read_candles(&wide).await, Ok(vec![]));
        assert!(
            s.last_candle_ts("ABC", Interval::Min5, false)
                .await
                .unwrap()
                .is_some()
        );
    }
}
