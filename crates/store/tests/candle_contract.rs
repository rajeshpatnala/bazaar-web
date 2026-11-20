//! Shared contract suite for `CandleSource` + `CandleSink`. Every adapter (SQLite, Parquet/DuckDB)
//! calls `run_contract` with a factory for a fresh empty store; the assertions are identical.
//! Slice 0 runs it against an in-memory fake to prove the suite itself.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::future::Future;
use std::str::FromStr;
use std::sync::Mutex;

use bazaar_core::candle::{Candle, Interval};
use bazaar_core::ports::{CandleRange, CandleSink, CandleSource, RepoError};
use chrono::{DateTime, TimeZone};
use chrono_tz::{Asia::Kolkata, Tz};
use rust_decimal::Decimal;

fn at(h: u32, m: u32) -> DateTime<Tz> {
    Kolkata.with_ymd_and_hms(2026, 1, 5, h, m, 0).unwrap()
}

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn bar(t: DateTime<Tz>, close: &str, vol: u64, adjusted: bool) -> Candle {
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

fn range(symbol: &str, from: DateTime<Tz>, to: DateTime<Tz>, adjusted: bool) -> CandleRange {
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
    assert!(
        s.read_candles(&range("NOPE", at(9, 0), at(15, 30), false))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        s.read_candles(&range("ABC", at(16, 0), at(17, 0), false))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        s.read_candles(&range("ABC", at(15, 0), at(9, 0), false))
            .await
            .unwrap()
            .is_empty()
    );

    // overlapping write: new row wins, no duplicate
    let newer = bar(at(9, 25), "100.50", 7777, false);
    s.write_candles("ABC", &[newer.clone()]).await.unwrap();
    let got = s
        .read_candles(&range("ABC", at(9, 0), at(15, 30), false))
        .await
        .unwrap();
    assert_eq!(got.len(), 4);
    assert_eq!(got[1], newer);

    // adjusted is part of the key: stored apart, never mixed in a read
    s.write_candles("ABC", &[bar(at(9, 25), "50.25", 1, true)])
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
    assert!(
        s.read_candles(&range("ABC", at(9, 0), at(15, 30), false))
            .await
            .unwrap()
            .is_empty()
    );
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

    // empty batch is a no-op
    assert_eq!(s.write_candles("ABC", &[]).await.unwrap(), 0);
}

/// In-memory reference adapter; keyed so overlapping writes replace.
#[derive(Default)]
struct MemStore(Mutex<Vec<(String, Candle)>>);

impl MemStore {
    fn put(&self, symbol: &str, candles: &[Candle]) -> Result<u64, RepoError> {
        if let Some(first) = candles.first() {
            if candles
                .iter()
                .any(|c| c.interval() != first.interval() || c.adjusted() != first.adjusted())
            {
                return Err(RepoError::Conflict);
            }
        }
        let mut rows = self.0.lock().map_err(|_| RepoError::Other)?;
        for c in candles {
            rows.retain(|(s, o)| {
                !(s == symbol
                    && o.interval() == c.interval()
                    && o.adjusted() == c.adjusted()
                    && o.close_time() == c.close_time())
            });
            rows.push((symbol.to_owned(), c.clone()));
        }
        Ok(candles.len() as u64)
    }
}

impl CandleSink for MemStore {
    fn write_candles(
        &self,
        symbol: &str,
        candles: &[Candle],
    ) -> impl Future<Output = Result<u64, RepoError>> + Send {
        let res = self.put(symbol, candles);
        async move { res }
    }
}

impl CandleSource for MemStore {
    fn read_candles(
        &self,
        range: &CandleRange,
    ) -> impl Future<Output = Result<Vec<Candle>, RepoError>> + Send {
        let res = self.0.lock().map_err(|_| RepoError::Other).map(|rows| {
            let mut v: Vec<Candle> = rows
                .iter()
                .filter(|(s, c)| *s == range.symbol && range.contains(c))
                .map(|(_, c)| c.clone())
                .collect();
            v.sort_by_key(Candle::close_time);
            v
        });
        async move { res }
    }

    fn last_candle_ts(
        &self,
        symbol: &str,
        interval: Interval,
        adjusted: bool,
    ) -> impl Future<Output = Result<Option<DateTime<Tz>>, RepoError>> + Send {
        let res = self.0.lock().map_err(|_| RepoError::Other).map(|rows| {
            rows.iter()
                .filter(|(s, c)| {
                    s == symbol && c.interval() == interval && c.adjusted() == adjusted
                })
                .map(|(_, c)| c.close_time())
                .max()
        });
        async move { res }
    }
}

#[tokio::test]
async fn contract_holds_for_in_memory_reference_adapter() {
    run_contract(MemStore::default).await;
}

#[test]
fn range_contains_respects_interval_adjusted_and_bounds() {
    let r = range("ABC", at(9, 20), at(9, 30), false);
    assert!(r.contains(&bar(at(9, 20), "100.00", 1, false)));
    assert!(r.contains(&bar(at(9, 30), "100.00", 1, false)));
    assert!(!r.contains(&bar(at(9, 35), "100.00", 1, false)));
    assert!(!r.contains(&bar(at(9, 25), "100.00", 1, true)));
}
