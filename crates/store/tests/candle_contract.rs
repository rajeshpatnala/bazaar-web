//! Runs the shared contract (`common::run_contract`) against an in-memory fake to prove the suite.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::missing_panics_doc,
    clippy::too_many_lines
)]

mod common;

use std::future::Future;
use std::sync::Mutex;

use bazaar_core::candle::{Candle, Interval};
use bazaar_core::ports::{CandleRange, CandleSink, CandleSource, RepoError};
use chrono::DateTime;
use chrono_tz::Tz;
use common::{at, bar, range, run_contract};

/// In-memory reference adapter; keyed so overlapping writes replace.
#[derive(Default)]
struct MemStore(Mutex<Vec<(String, Candle)>>);

impl MemStore {
    fn put(&self, symbol: &str, candles: &[Candle]) -> Result<u64, RepoError> {
        let mixed = candles.first().is_some_and(|f| {
            candles
                .iter()
                .any(|c| c.interval() != f.interval() || c.adjusted() != f.adjusted())
        });
        if mixed {
            return Err(RepoError::Conflict);
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
    let min1 = Candle::new(at(9, 25), Interval::Min1, [common::d("1"); 4], 1, false).unwrap();
    assert!(!r.contains(&min1));
}
