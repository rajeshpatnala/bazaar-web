//! Parquet/DuckDB candle adapter: shared contract suite, SQLite equivalence, leftover `.tmp`,
//! corrupt files. Temp dirs only.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]

mod common;

use bazaar_core::candle::{Candle, Interval};
use bazaar_core::ports::{CandleRange, CandleSink, CandleSource, RepoError};
use bazaar_store::{DuckStore, ParquetStore, SqliteStore};
use chrono::DateTime;
use chrono_tz::Tz;
use common::{at, bar, fixture_candles, range, run_contract};

/// Writer and reader over one temp dir, so the contract suite sees one adapter.
struct Pair {
    dir: tempfile::TempDir,
    w: ParquetStore,
    r: DuckStore,
}

impl Pair {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let w = ParquetStore::new(dir.path()).unwrap();
        let r = DuckStore::new(dir.path()).unwrap();
        Self { dir, w, r }
    }
}

impl CandleSink for Pair {
    async fn write_candles(&self, symbol: &str, candles: &[Candle]) -> Result<u64, RepoError> {
        self.w.write_candles(symbol, candles).await
    }
}

impl CandleSource for Pair {
    async fn read_candles(&self, range: &CandleRange) -> Result<Vec<Candle>, RepoError> {
        self.r.read_candles(range).await
    }

    async fn last_candle_ts(
        &self,
        symbol: &str,
        interval: Interval,
        adjusted: bool,
    ) -> Result<Option<DateTime<Tz>>, RepoError> {
        self.r.last_candle_ts(symbol, interval, adjusted).await
    }
}

#[tokio::test]
async fn contract_holds_for_parquet_adapter() {
    run_contract(Pair::new).await;
}

#[tokio::test]
async fn parquet_and_sqlite_return_identical_candles_and_markers() {
    let p = Pair::new();
    let s = SqliteStore::open_in_memory().await.unwrap();
    let mut batch = fixture_candles();
    batch.reverse();
    batch.push(bar(at(9, 45), "100.99", 5, false));
    for sink in [&p as &dyn Sink, &s as &dyn Sink] {
        sink.put("ABC", &batch).await;
    }
    let overwrite = [bar(at(9, 25), "100.50", 7777, false)];
    p.write_candles("ABC", &overwrite).await.unwrap();
    s.write_candles("ABC", &overwrite).await.unwrap();
    for (from, to) in [
        (at(9, 0), at(15, 30)),
        (at(9, 25), at(9, 35)),
        (at(16, 0), at(17, 0)),
    ] {
        let r = range("ABC", from, to, false);
        assert_eq!(
            p.read_candles(&r).await.unwrap(),
            s.read_candles(&r).await.unwrap()
        );
    }
    assert_eq!(
        p.last_candle_ts("ABC", Interval::Min5, false).await,
        s.last_candle_ts("ABC", Interval::Min5, false).await
    );
}

/// Object-safe shim so both adapters can take the same fixture in a loop.
trait Sink {
    fn put<'a>(
        &'a self,
        symbol: &'a str,
        c: &'a [Candle],
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + 'a>>;
}

impl<T: CandleSink> Sink for T {
    fn put<'a>(
        &'a self,
        symbol: &'a str,
        c: &'a [Candle],
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + 'a>> {
        Box::pin(async move {
            self.write_candles(symbol, c).await.unwrap();
        })
    }
}

#[tokio::test]
async fn leftover_tmp_is_ignored_by_readers() {
    let p = Pair::new();
    p.write_candles("ABC", &fixture_candles()).await.unwrap();
    let tmp = p.dir.path().join("ABC").join("min5_raw.parquet.tmp");
    std::fs::write(&tmp, b"half written").unwrap();
    // a stray tmp for a key that has no real file reads as empty, not an error
    std::fs::create_dir(p.dir.path().join("XYZ")).unwrap();
    std::fs::write(p.dir.path().join("XYZ").join("min5_raw.parquet.tmp"), b"x").unwrap();
    let got = p
        .read_candles(&range("ABC", at(9, 0), at(15, 30), false))
        .await
        .unwrap();
    assert_eq!(got, fixture_candles());
    let none = p
        .read_candles(&range("XYZ", at(9, 0), at(15, 30), false))
        .await
        .unwrap();
    assert_eq!(none, Vec::<Candle>::new());
    assert_eq!(
        p.last_candle_ts("XYZ", Interval::Min5, false).await,
        Ok(None)
    );
}

#[tokio::test]
async fn corrupt_file_is_corrupt_never_a_panic_or_zero() {
    let p = Pair::new();
    p.write_candles("ABC", &fixture_candles()).await.unwrap();
    let file = p.dir.path().join("ABC").join("min5_raw.parquet");
    std::fs::write(&file, b"not parquet").unwrap();
    let r = range("ABC", at(9, 0), at(15, 30), false);
    assert_eq!(p.read_candles(&r).await, Err(RepoError::Corrupt));
    assert_eq!(
        p.last_candle_ts("ABC", Interval::Min5, false).await,
        Err(RepoError::Corrupt)
    );
}

#[tokio::test]
async fn non_decimal_value_in_a_valid_file_is_corrupt() {
    let p = Pair::new();
    let file = p.dir.path().join("ABC");
    std::fs::create_dir(&file).unwrap();
    let file = file.join("min5_raw.parquet");
    let conn = duckdb_writer();
    conn.execute_batch(&format!(
        "COPY (SELECT 1767585000000000000::BIGINT AS close_ns, 'junk' AS open, '1' AS high, '1' AS low, \
         '1' AS close, 1::BIGINT AS volume) TO '{}' (FORMAT PARQUET)",
        file.to_str().unwrap()
    ))
    .unwrap();
    assert_eq!(
        p.read_candles(&range("ABC", at(9, 0), at(15, 30), false))
            .await,
        Err(RepoError::Corrupt)
    );
}

#[tokio::test]
async fn hostile_symbols_are_refused_on_read() {
    let p = Pair::new();
    for bad in ["../ABC", "abc", "A'; DROP TABLE c; --", ""] {
        assert_eq!(
            p.read_candles(&range(bad, at(9, 0), at(15, 30), false))
                .await,
            Err(RepoError::Other),
            "{bad}"
        );
    }
}

fn duckdb_writer() -> duckdb::Connection {
    duckdb::Connection::open_in_memory().unwrap()
}
