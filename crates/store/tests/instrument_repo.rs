//! Public-API check of the SQLite `InstrumentRepo`: temp file, import twice, reopen.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::str::FromStr;

use bazaar_core::instrument::{Exchange, Instrument, InstrumentType};
use bazaar_core::ports::InstrumentRepo;
use bazaar_store::SqliteStore;
use chrono::NaiveDate;
use rust_decimal::Decimal;

#[tokio::test]
async fn import_twice_then_reopen_gives_identical_rows() {
    let row = Instrument {
        instrument_token: 11,
        exchange_token: 22,
        tradingsymbol: "ABC".to_owned(),
        name: "ABC".to_owned(),
        exchange: Exchange::Nse,
        instrument_type: InstrumentType::Equity,
        segment: "SEG".to_owned(),
        expiry: None,
        strike: None,
        tick_size: Decimal::from_str("0.010").unwrap(),
        lot_size: 1,
    };
    let day = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("i.db");
    let store = SqliteStore::open(&path).await.unwrap();
    store
        .upsert_instruments(std::slice::from_ref(&row), day)
        .await
        .unwrap();
    store
        .upsert_instruments(std::slice::from_ref(&row), day)
        .await
        .unwrap();
    drop(store);
    let again = SqliteStore::open(&path).await.unwrap();
    assert_eq!(again.get(&Exchange::Nse, "ABC").await.unwrap(), Some(row));
    assert_eq!(again.last_fetched_on().await.unwrap(), Some(day));
}
