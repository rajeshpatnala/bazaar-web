//! Ports: traits the domain owns and adapter crates implement. Core depends on no adapter.

use std::future::Future;

use chrono::NaiveDate;
use thiserror::Error;

use crate::instrument::{Exchange, Instrument};

/// Storage failure with a safe, category-only reason: no paths, SQL or row contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RepoError {
    /// The store could not be reached or opened.
    #[error("store unavailable")]
    Unavailable,
    /// A migration failed or the schema is not what the code expects.
    #[error("store schema error")]
    Schema,
    /// A stored value could not be decoded.
    #[error("stored data is corrupt")]
    Corrupt,
    /// Any other storage failure.
    #[error("store error")]
    Other,
}

impl RepoError {
    /// Stable category label, safe to print.
    #[must_use]
    pub const fn safe_reason(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::Schema => "schema",
            Self::Corrupt => "corrupt",
            Self::Other => "other",
        }
    }
}

/// Outcome of one import into the master.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UpsertSummary {
    /// Rows written (inserted or replaced with the same key).
    pub written: u64,
}

/// The instrument master: write once per fetch, query by data only.
///
/// Rows are keyed by `(exchange, tradingsymbol)`; importing the same rows twice leaves the
/// store identical. Each import records `fetched_on` and keeps a dated snapshot.
pub trait InstrumentRepo: Send + Sync {
    /// Idempotent upsert of `rows` fetched on `fetched_on` (IST date).
    fn upsert_instruments(
        &self,
        rows: &[Instrument],
        fetched_on: NaiveDate,
    ) -> impl Future<Output = Result<UpsertSummary, RepoError>> + Send;

    /// One instrument by key.
    fn get(
        &self,
        exchange: &Exchange,
        tradingsymbol: &str,
    ) -> impl Future<Output = Result<Option<Instrument>, RepoError>> + Send;

    /// Earliest derivative expiry of `underlying` on or after `on_or_after`.
    fn next_expiry(
        &self,
        underlying: &str,
        on_or_after: NaiveDate,
    ) -> impl Future<Output = Result<Option<NaiveDate>, RepoError>> + Send;

    /// True when a cash equity named `tradingsymbol` has a derivative in the master
    /// (the `cash_fno` eligibility the calendar needs).
    fn is_cash_fno(
        &self,
        tradingsymbol: &str,
    ) -> impl Future<Output = Result<bool, RepoError>> + Send;

    /// Date of the latest import, `None` for an empty master.
    fn last_fetched_on(&self) -> impl Future<Output = Result<Option<NaiveDate>, RepoError>> + Send;

    /// Cheap liveness check of the store.
    fn ping(&self) -> impl Future<Output = Result<(), RepoError>> + Send;
}

#[cfg(test)]
#[allow(clippy::unused_async_trait_impl)] // the fake has nothing to await
mod tests {
    use super::*;
    use crate::instrument::InstrumentType;
    use rust_decimal::Decimal;
    use std::sync::Mutex;

    /// Minimal fake showing the contract: keyed upsert, queries over the data.
    #[derive(Debug, Default)]
    struct Fake {
        rows: Mutex<Vec<Instrument>>,
        fetched: Mutex<Option<NaiveDate>>,
    }

    impl InstrumentRepo for Fake {
        async fn upsert_instruments(
            &self,
            rows: &[Instrument],
            fetched_on: NaiveDate,
        ) -> Result<UpsertSummary, RepoError> {
            let mut store = self.rows.lock().map_err(|_| RepoError::Other)?;
            for r in rows {
                store.retain(|s| !(s.exchange == r.exchange && s.tradingsymbol == r.tradingsymbol));
                store.push(r.clone());
            }
            *self.fetched.lock().map_err(|_| RepoError::Other)? = Some(fetched_on);
            Ok(UpsertSummary {
                written: rows.len() as u64,
            })
        }
        async fn get(&self, e: &Exchange, s: &str) -> Result<Option<Instrument>, RepoError> {
            let store = self.rows.lock().map_err(|_| RepoError::Other)?;
            Ok(store
                .iter()
                .find(|r| &r.exchange == e && r.tradingsymbol == s)
                .cloned())
        }
        async fn next_expiry(
            &self,
            u: &str,
            from: NaiveDate,
        ) -> Result<Option<NaiveDate>, RepoError> {
            let store = self.rows.lock().map_err(|_| RepoError::Other)?;
            Ok(store
                .iter()
                .filter(|r| r.name == u)
                .filter_map(|r| r.expiry)
                .filter(|d| *d >= from)
                .min())
        }
        async fn is_cash_fno(&self, s: &str) -> Result<bool, RepoError> {
            let store = self.rows.lock().map_err(|_| RepoError::Other)?;
            Ok(store
                .iter()
                .any(|r| r.name == s && r.instrument_type.is_derivative()))
        }
        async fn last_fetched_on(&self) -> Result<Option<NaiveDate>, RepoError> {
            Ok(*self.fetched.lock().map_err(|_| RepoError::Other)?)
        }
        async fn ping(&self) -> Result<(), RepoError> {
            Ok(())
        }
    }

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn fut(sym: &str, expiry: NaiveDate) -> Instrument {
        Instrument {
            instrument_token: 1,
            exchange_token: 1,
            tradingsymbol: sym.into(),
            name: "ABC".into(),
            exchange: Exchange::Nfo,
            instrument_type: InstrumentType::Future,
            segment: "NFO-FUT".into(),
            expiry: Some(expiry),
            strike: None,
            tick_size: Decimal::new(5, 2),
            lot_size: 10,
        }
    }

    /// Drive a future to completion without a runtime (the fake never pends).
    fn block<T>(f: impl Future<Output = T>) -> T {
        let mut f = std::pin::pin!(f);
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        match f.as_mut().poll(&mut cx) {
            std::task::Poll::Ready(v) => v,
            std::task::Poll::Pending => unreachable!("fake never pends"),
        }
    }

    #[test]
    fn port_contract_upsert_is_keyed_and_queries_follow_data() {
        let repo = Fake::default();
        let rows = [fut("ABC1", d(2026, 1, 29)), fut("ABC2", d(2026, 2, 26))];
        assert_eq!(block(repo.last_fetched_on()), Ok(None));
        block(repo.upsert_instruments(&rows, d(2026, 1, 5))).unwrap();
        block(repo.upsert_instruments(&rows, d(2026, 1, 6))).unwrap();
        assert_eq!(repo.rows.lock().unwrap().len(), 2);
        assert_eq!(block(repo.last_fetched_on()), Ok(Some(d(2026, 1, 6))));
        assert_eq!(
            block(repo.next_expiry("ABC", d(2026, 1, 30))),
            Ok(Some(d(2026, 2, 26)))
        );
        assert_eq!(block(repo.next_expiry("ABC", d(2026, 3, 1))), Ok(None));
        assert_eq!(block(repo.is_cash_fno("ABC")), Ok(true));
        assert_eq!(block(repo.is_cash_fno("XYZ")), Ok(false));
        assert!(block(repo.get(&Exchange::Nfo, "ABC1")).unwrap().is_some());
        assert!(block(repo.get(&Exchange::Nse, "ABC1")).unwrap().is_none());
        assert_eq!(block(repo.ping()), Ok(()));
    }

    #[test]
    fn repo_error_reasons_are_stable_and_carry_no_detail() {
        assert_eq!(RepoError::Unavailable.safe_reason(), "unavailable");
        assert_eq!(RepoError::Schema.safe_reason(), "schema");
        assert_eq!(RepoError::Corrupt.to_string(), "stored data is corrupt");
        assert_eq!(RepoError::Other.safe_reason(), "other");
    }
}
