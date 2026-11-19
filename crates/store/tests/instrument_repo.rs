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

fn row(token: u64, kind: InstrumentType, ex: Exchange, expiry: Option<NaiveDate>) -> Instrument {
    Instrument {
        instrument_token: token,
        exchange_token: token + 1000,
        tradingsymbol: format!("TESTU{token}"),
        name: "TESTU".to_owned(),
        exchange: ex,
        instrument_type: kind,
        segment: "SEG".to_owned(),
        expiry,
        strike: None,
        tick_size: Decimal::from_str("0.05").unwrap(),
        lot_size: 50,
    }
}

/// No weekday is hardcoded: moving the fixture expiry through all seven weekdays
/// makes `next_expiry` return exactly that date and `is_cash_fno` stay true.
#[tokio::test]
async fn next_expiry_and_cash_fno_follow_the_expiry_weekday() {
    let base = NaiveDate::from_ymd_opt(2031, 1, 27).unwrap();
    let fetched = NaiveDate::from_ymd_opt(2031, 1, 1).unwrap();
    for shift in 0..7 {
        let expiry = base + chrono::Days::new(shift);
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(&dir.path().join("w.db")).await.unwrap();
        let mut eq = row(1, InstrumentType::Equity, Exchange::Nse, None);
        eq.tradingsymbol = "TESTU".to_owned();
        let fut = row(2, InstrumentType::Future, Exchange::Nfo, Some(expiry));
        store.upsert_instruments(&[eq, fut], fetched).await.unwrap();
        let q = |from| store.next_expiry(&Exchange::Nfo, "TESTU", &InstrumentType::Future, from);
        assert_eq!(q(fetched).await.unwrap(), Some(expiry), "shift {shift}");
        assert!(store.is_cash_fno("TESTU").await.unwrap(), "shift {shift}");
        assert_eq!(q(expiry + chrono::Days::new(1)).await.unwrap(), None);
    }
}
