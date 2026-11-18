//! Ports: traits the domain owns and adapter crates implement. Core depends on no adapter.

use std::future::Future;

use chrono::NaiveDate;
use thiserror::Error;

use crate::instrument::{Exchange, Instrument, InstrumentType};

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
    /// Idempotent upsert of `rows` fetched on `fetched_on` (IST date). One transaction per
    /// call: the caller bounds `rows` (the parser caps a master at its `max_rows`).
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

    /// Earliest expiry on or after `on_or_after` among rows of `exchange` whose `name`
    /// (the underlying) equals `underlying` (trimmed, case-insensitive) and whose type is
    /// `kind`. Filtering by `kind` keeps monthly futures apart from option series.
    ///
    /// `on_or_after` is a date: the caller passes the calendar's next valid date once the
    /// session has closed, since the repo has no time of day.
    fn next_expiry(
        &self,
        exchange: &Exchange,
        underlying: &str,
        kind: &InstrumentType,
        on_or_after: NaiveDate,
    ) -> impl Future<Output = Result<Option<NaiveDate>, RepoError>> + Send;

    /// The `cash_fno` eligibility the calendar needs. Match key: `symbol` (trimmed,
    /// case-insensitive) is the `tradingsymbol` of a cash `Equity` row AND equals the
    /// `name` (underlying) of at least one derivative row. Both rows are required, so an
    /// index (no cash equity row) or a delisted equity (no derivative) is not eligible.
    fn is_cash_fno(&self, symbol: &str) -> impl Future<Output = Result<bool, RepoError>> + Send;

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
            e: &Exchange,
            u: &str,
            k: &InstrumentType,
            from: NaiveDate,
        ) -> Result<Option<NaiveDate>, RepoError> {
            let store = self.rows.lock().map_err(|_| RepoError::Other)?;
            Ok(store
                .iter()
                .filter(|r| &r.exchange == e && &r.instrument_type == k)
                .filter(|r| r.name.trim().eq_ignore_ascii_case(u.trim()))
                .filter_map(|r| r.expiry)
                .filter(|d| *d >= from)
                .min())
        }
        async fn is_cash_fno(&self, s: &str) -> Result<bool, RepoError> {
            let store = self.rows.lock().map_err(|_| RepoError::Other)?;
            let s = s.trim();
            let cash = store.iter().any(|r| {
                r.instrument_type == InstrumentType::Equity
                    && r.tradingsymbol.eq_ignore_ascii_case(s)
            });
            let deriv = store
                .iter()
                .any(|r| r.instrument_type.is_derivative() && r.name.eq_ignore_ascii_case(s));
            Ok(cash && deriv)
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
        let mut cash = fut("ABC", d(2026, 1, 1));
        cash.name = "ABC CORP LTD".into(); // company name differs from the symbol
        cash.exchange = Exchange::Nse;
        cash.instrument_type = InstrumentType::Equity;
        cash.expiry = None;
        let mut weekly = fut("ABC-W", d(2026, 2, 5));
        weekly.instrument_type = InstrumentType::Call;
        weekly.strike = Some(Decimal::new(100, 0));
        let mut index = fut("NIFTYF", d(2026, 1, 29));
        index.name = "NIFTY".into(); // no cash row named NIFTY
        let rows = [
            fut("ABC1", d(2026, 1, 29)),
            fut("ABC2", d(2026, 2, 26)),
            cash,
            weekly,
            index,
        ];
        assert_eq!(block(repo.last_fetched_on()), Ok(None));
        block(repo.upsert_instruments(&rows, d(2026, 1, 5))).unwrap();
        block(repo.upsert_instruments(&rows, d(2026, 1, 6))).unwrap();
        assert_eq!(repo.rows.lock().unwrap().len(), 5);
        assert_eq!(block(repo.last_fetched_on()), Ok(Some(d(2026, 1, 6))));
        let nfo = Exchange::Nfo;
        let fut_k = InstrumentType::Future;
        // futures skip the weekly option expiry; kind and exchange are filters
        assert_eq!(
            block(repo.next_expiry(&nfo, "abc", &fut_k, d(2026, 1, 30))),
            Ok(Some(d(2026, 2, 26)))
        );
        assert_eq!(
            block(repo.next_expiry(&nfo, "ABC", &InstrumentType::Call, d(2026, 1, 30))),
            Ok(Some(d(2026, 2, 5)))
        );
        assert_eq!(
            block(repo.next_expiry(&nfo, "ABC", &fut_k, d(2026, 3, 1))),
            Ok(None)
        );
        assert_eq!(
            block(repo.next_expiry(&Exchange::Bfo, "ABC", &fut_k, d(2026, 1, 1))),
            Ok(None)
        );
        assert_eq!(block(repo.is_cash_fno("abc")), Ok(true));
        assert_eq!(block(repo.is_cash_fno("NIFTY")), Ok(false)); // derivative only
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
