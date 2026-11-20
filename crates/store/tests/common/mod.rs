//! Shared candle contract suite: every adapter test file does `mod common;` and calls
//! `run_contract` with a factory for a fresh, empty store.
#![allow(
    dead_code,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::missing_panics_doc,
    clippy::too_many_lines
)]
use std::str::FromStr;

use bazaar_core::candle::{Candle, Interval};
use bazaar_core::ports::{CandleRange, CandleSink, CandleSource, RepoError};
use chrono::{DateTime, TimeZone};
use chrono_tz::{Asia::Kolkata, Tz};
use rust_decimal::Decimal;

pub fn at(h: u32, m: u32) -> DateTime<Tz> {
    Kolkata.with_ymd_and_hms(2026, 1, 5, h, m, 0).unwrap()
}

pub fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

pub fn bar(t: DateTime<Tz>, close: &str, vol: u64, adjusted: bool) -> Candle {
    Candle::new(
        t,
        Interval::Min5,
        [d("100.10"), d("101.55"), d("99.90"), d(close)],
        vol,
        adjusted,
    )
    .unwrap()
}

/// Five-minute fixture bars, oldest first, exact decimals preserved.
pub fn fixture_candles() -> Vec<Candle> {
    (0..4u32)
        .map(|i| {
            bar(
                at(9, 20 + 5 * i),
                &format!("100.{i}5"),
                1000 + u64::from(i),
                false,
            )
        })
        .collect()
}

pub fn range(symbol: &str, from: DateTime<Tz>, to: DateTime<Tz>, adjusted: bool) -> CandleRange {
    CandleRange {
        symbol: symbol.to_owned(),
        interval: Interval::Min5,
        from,
        to,
        adjusted,
    }
}

/// The contract. `make` yields a fresh, empty adapter each call.
pub async fn run_contract<S, F>(make: F)
where
    S: CandleSource + CandleSink,
    F: Fn() -> S,
{
    // round trip, oldest first, exact (written out of order)
    let s = make();
    let mut shuffled = fixture_candles();
    shuffled.reverse();
    assert_eq!(s.write_candles("ABC", &shuffled).await.unwrap(), 4);
    let got = s
        .read_candles(&range("ABC", at(9, 0), at(15, 30), false))
        .await
        .unwrap();
    assert_eq!(got, fixture_candles());
    assert_eq!(got[0].close_time().timezone(), Kolkata);
    assert_eq!(got[1].close().to_string(), "100.15");

    // inclusive bounds
    let got = s
        .read_candles(&range("ABC", at(9, 25), at(9, 30), false))
        .await
        .unwrap();
    assert_eq!(got.len(), 2);

    // unknown symbol, empty and inverted range: empty, not an error
    let empty = s
        .read_candles(&range("NOPE", at(9, 0), at(15, 30), false))
        .await
        .unwrap();
    assert_eq!(empty, Vec::<Candle>::new());
    let empty = s
        .read_candles(&range("ABC", at(16, 0), at(17, 0), false))
        .await
        .unwrap();
    assert_eq!(empty, Vec::<Candle>::new());
    let empty = s
        .read_candles(&range("ABC", at(15, 0), at(9, 0), false))
        .await
        .unwrap();
    assert_eq!(empty, Vec::<Candle>::new());

    // overlapping write: new row wins, no duplicate
    let newer = bar(at(9, 25), "100.50", 7777, false);
    s.write_candles("ABC", std::slice::from_ref(&newer))
        .await
        .unwrap();
    let got = s
        .read_candles(&range("ABC", at(9, 0), at(15, 30), false))
        .await
        .unwrap();
    assert_eq!(got.len(), 4);
    assert_eq!(got[1], newer);

    // adjusted is part of the key: stored apart, never mixed in a read
    s.write_candles("ABC", &[bar(at(9, 25), "100.25", 1, true)])
        .await
        .unwrap();
    let adj = s
        .read_candles(&range("ABC", at(9, 0), at(15, 30), true))
        .await
        .unwrap();
    assert_eq!(adj.len(), 1);
    assert!(adj[0].adjusted());
    let raw = s
        .read_candles(&range("ABC", at(9, 0), at(15, 30), false))
        .await
        .unwrap();
    assert!(raw.iter().all(|c| !c.adjusted()) && raw.len() == 4);

    // symbols are independent
    s.write_candles("XYZ", &[bar(at(9, 20), "100.00", 5, false)])
        .await
        .unwrap();
    assert_eq!(
        s.read_candles(&range("XYZ", at(9, 0), at(15, 30), false))
            .await
            .unwrap()
            .len(),
        1
    );

    // freshness marker follows writes
    assert_eq!(
        s.last_candle_ts("ABC", Interval::Min5, false)
            .await
            .unwrap(),
        Some(at(9, 35))
    );
    assert_eq!(
        s.last_candle_ts("NOPE", Interval::Min5, false)
            .await
            .unwrap(),
        None
    );

    // a mixed batch is refused and writes nothing
    let s = make();
    let mixed = [
        bar(at(9, 20), "100.00", 1, false),
        bar(at(9, 25), "100.00", 1, true),
    ];
    assert_eq!(
        s.write_candles("ABC", &mixed).await,
        Err(RepoError::Conflict)
    );
    let empty = s
        .read_candles(&range("ABC", at(9, 0), at(15, 30), false))
        .await
        .unwrap();
    assert_eq!(empty, Vec::<Candle>::new());
    assert_eq!(
        s.last_candle_ts("ABC", Interval::Min5, false)
            .await
            .unwrap(),
        None
    );
    let other = Candle::new(at(9, 30), Interval::Min1, [d("1"); 4], 1, false).unwrap();
    let mixed = [bar(at(9, 20), "100.00", 1, false), other];
    assert_eq!(
        s.write_candles("ABC", &mixed).await,
        Err(RepoError::Conflict)
    );

    // the refused batch left nothing behind, in either interval
    let min1 = CandleRange::new("ABC", Interval::Min1, at(9, 0), at(15, 30), false);
    let none = s.read_candles(&min1).await.unwrap();
    assert_eq!(none, Vec::<Candle>::new());
    let none = s
        .read_candles(&range("ABC", at(9, 0), at(15, 30), false))
        .await
        .unwrap();
    assert_eq!(none, Vec::<Candle>::new());

    // duplicate close_time inside one batch: the last one wins, one row remains
    let dup = [
        bar(at(9, 20), "100.00", 1, false),
        bar(at(9, 20), "100.30", 2, false),
    ];
    s.write_candles("DUP", &dup).await.unwrap();
    let got = s
        .read_candles(&range("DUP", at(9, 0), at(15, 30), false))
        .await
        .unwrap();
    assert_eq!(got, vec![dup[1].clone()]);

    // empty batch is a no-op
    assert_eq!(s.write_candles("ABC", &[]).await.unwrap(), 0);
}
