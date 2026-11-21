//! SQLite candle adapter: shared contract suite plus corrupt-row and injection cases.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]

mod common;

use bazaar_core::candle::{Candle, Interval};
use bazaar_core::ports::{CandleRange, CandleSink, CandleSource, RepoError};
use bazaar_store::SqliteStore;
use common::{at, bar, range, run_contract};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Connection, SqliteConnection};

#[tokio::test(flavor = "multi_thread")]
async fn contract_holds_for_sqlite_adapter() {
    // `make` is sync: block in place on the running runtime so the pool's worker survives.
    run_contract(|| {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(SqliteStore::open_in_memory())
                .unwrap()
        })
    })
    .await;
}

#[tokio::test]
async fn file_store_survives_reopen_with_exact_values() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("c.db");
    let s = SqliteStore::open(&path).await.unwrap();
    s.write_candles("ABC", &common::fixture_candles())
        .await
        .unwrap();
    drop(s);
    let s = SqliteStore::open(&path).await.unwrap();
    let got = s
        .read_candles(&range("ABC", at(9, 0), at(15, 30), false))
        .await
        .unwrap();
    assert_eq!(got, common::fixture_candles());
}

#[tokio::test]
async fn hostile_symbol_is_data_not_sql() {
    let s = SqliteStore::open_in_memory().await.unwrap();
    let evil = "A'; DROP TABLE candles; --";
    s.write_candles(evil, &[bar(at(9, 20), "100.00", 1, false)])
        .await
        .unwrap();
    s.write_candles("ABC", &[bar(at(9, 20), "100.00", 1, false)])
        .await
        .unwrap();
    let got = s
        .read_candles(&range(evil, at(9, 0), at(15, 30), false))
        .await
        .unwrap();
    assert_eq!(got.len(), 1);
    let other = s
        .read_candles(&range("ABC", at(9, 0), at(15, 30), false))
        .await
        .unwrap();
    assert_eq!(other.len(), 1);
}

#[tokio::test]
async fn volume_above_i64_is_refused_and_writes_nothing() {
    let s = SqliteStore::open_in_memory().await.unwrap();
    let big = Candle::new(
        at(9, 20),
        Interval::Min5,
        [common::d("1"); 4],
        u64::MAX,
        false,
    )
    .unwrap();
    let ok = bar(at(9, 25), "100.00", 1, false);
    assert_eq!(
        s.write_candles("ABC", &[ok, big]).await,
        Err(RepoError::Other)
    );
    let got = s
        .read_candles(&range("ABC", at(9, 0), at(15, 30), false))
        .await
        .unwrap();
    assert_eq!(got, Vec::<Candle>::new());
}

/// Write one good row to a file DB, damage it through a second connection, read via the adapter.
async fn damaged(sql: &'static str) -> Result<Vec<Candle>, RepoError> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("c.db");
    let s = SqliteStore::open(&path).await.unwrap();
    s.write_candles("ABC", &[bar(at(9, 20), "100.00", 1, false)])
        .await
        .unwrap();
    let mut raw = SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(&path))
        .await
        .unwrap();
    sqlx::query(sql).execute(&mut raw).await.unwrap();
    s.read_candles(&CandleRange::new(
        "ABC",
        Interval::Min5,
        at(9, 0),
        at(15, 30),
        false,
    ))
    .await
}

#[tokio::test]
async fn corrupt_stored_values_fail_closed() {
    for sql in [
        "UPDATE candles SET close = 'abc'",
        "UPDATE candles SET open = ''",
        "UPDATE candles SET high = '1.2.3'",
        "UPDATE candles SET volume = -5",
        "UPDATE candles SET low = '-1'",
        "UPDATE candles SET high = '50'",
    ] {
        assert_eq!(damaged(sql).await, Err(RepoError::Corrupt), "{sql}");
    }
}

#[tokio::test]
async fn db_level_failure_mid_transaction_rolls_back_earlier_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("c.db");
    let s = SqliteStore::open(&path).await.unwrap();
    let mut raw = SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(&path))
        .await
        .unwrap();
    // The trigger rejects the second row of the batch inside the transaction.
    sqlx::query(
        "CREATE TRIGGER no_999 BEFORE INSERT ON candles WHEN NEW.volume = 999 \
         BEGIN SELECT RAISE(ABORT, 'boom'); END",
    )
    .execute(&mut raw)
    .await
    .unwrap();
    let batch = [
        bar(at(9, 20), "100.00", 1, false),
        bar(at(9, 25), "100.00", 999, false),
    ];
    assert!(s.write_candles("ABC", &batch).await.is_err());
    let got = s
        .read_candles(&range("ABC", at(9, 0), at(15, 30), false))
        .await
        .unwrap();
    assert_eq!(got, Vec::<Candle>::new());
    assert_eq!(
        s.last_candle_ts("ABC", Interval::Min5, false)
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn oversized_symbol_and_batch_are_refused() {
    let s = SqliteStore::open_in_memory().await.unwrap();
    let one = [bar(at(9, 20), "100.00", 1, false)];
    for sym in [String::new(), "A".repeat(65)] {
        assert_eq!(s.write_candles(&sym, &one).await, Err(RepoError::TooLarge));
        assert_eq!(
            s.read_candles(&range(&sym, at(9, 0), at(15, 30), false))
                .await,
            Err(RepoError::TooLarge)
        );
    }
    let big = vec![one[0].clone(); 100_001];
    assert_eq!(s.write_candles("ABC", &big).await, Err(RepoError::TooLarge));
    assert_eq!(s.write_candles(&"A".repeat(64), &one).await, Ok(1));
}
