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

const UPSERT: &str = "INSERT INTO candles (symbol, interval, adjusted, close_ns, open, high, \
    low, close, volume) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
    ON CONFLICT (symbol, interval, adjusted, close_ns) DO UPDATE SET open = excluded.open, \
    high = excluded.high, low = excluded.low, close = excluded.close, volume = excluded.volume";

const READ: &str = "SELECT interval, adjusted, close_ns, open, high, low, close, volume \
    FROM candles WHERE symbol = ? AND interval = ? AND adjusted = ? \
    AND close_ns >= ? AND close_ns <= ? ORDER BY close_ns";

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
        Ok(rows.len() as u64)
    }
}

impl CandleSource for SqliteStore {
    async fn read_candles(&self, range: &CandleRange) -> Result<Vec<Candle>, RepoError> {
        let rows = sqlx::query(READ)
            .bind(&range.symbol)
            .bind(interval_label(range.interval))
            .bind(i64::from(range.adjusted))
            .bind(to_ns(&range.from)?)
            .bind(to_ns(&range.to)?)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| map_err(&e))?;
        rows.iter().map(to_candle).collect()
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
