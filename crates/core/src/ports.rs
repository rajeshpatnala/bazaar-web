//! Ports: traits the domain owns and adapter crates implement. Core depends on no adapter.

use std::future::Future;

use chrono::{DateTime, NaiveDate};
use chrono_tz::Tz;
use thiserror::Error;

use crate::candle::{Candle, Interval};
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
    /// A uniqueness or optimistic-version check failed (the stale writer loses).
    #[error("store conflict")]
    Conflict,
    /// Input or result over a size cap (symbol length, batch size, rows read).
    #[error("store input or result too large")]
    TooLarge,
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
            Self::Conflict => "conflict",
            Self::TooLarge => "too_large",
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

// ---- F10 execution ports. Plain data only: core knows no execution or risk type. ----

/// Injected id source (128-bit, time-ordered in production, a counter in tests).
pub trait IdGen: Send + Sync {
    /// Next unique id.
    fn next_id(&self) -> u128;
}

/// One persisted order: opaque state label and payload, owned by `bazaar-execution`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderRecord {
    pub intent_id: u128,
    /// Optimistic-lock counter; starts at 0 and rises by one per `advance`.
    pub version: u32,
    pub state: String,
    /// Canonical JSON of the order intent.
    pub payload: String,
}

/// An audit entry before the store assigns `seq`, `prev_hash` and `hash`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditDraft {
    pub ts_ist_rfc3339: String,
    pub actor: String,
    pub kind: String,
    /// Canonical JSON (keys sorted, no whitespace).
    pub payload: String,
}

/// A stored audit row. See `docs/adr/0001-audit-hash-chain.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRow {
    pub seq: u64,
    pub draft: AuditDraft,
    pub prev_hash: [u8; 32],
    pub hash: [u8; 32],
}

/// Pure hash function the caller passes in, so the store never depends on `execution`:
/// `(prev_hash, seq, draft) -> hash`.
pub type ChainFn = fn(&[u8; 32], u64, &AuditDraft) -> [u8; 32];

/// Append-only, hash-chained audit log. No update or delete path exists.
pub trait AuditRepo: Send + Sync {
    /// Append in one transaction; the store computes `hash` with `chain` against the last row.
    fn append(
        &self,
        draft: AuditDraft,
        chain: ChainFn,
    ) -> impl Future<Output = Result<AuditRow, RepoError>> + Send;
    /// All rows in `seq` order, for `verify_chain`.
    fn read_all(&self) -> impl Future<Output = Result<Vec<AuditRow>, RepoError>> + Send;
}

/// Order store. `insert` and `advance` write state, event and audit row in one transaction.
pub trait OrderRepo: Send + Sync {
    /// Insert a new order at version 0; a duplicate id is `RepoError::Conflict`.
    fn insert(
        &self,
        order: OrderRecord,
        audit: AuditDraft,
        chain: ChainFn,
    ) -> impl Future<Output = Result<(), RepoError>> + Send;
    /// One order by id.
    fn get(
        &self,
        intent_id: u128,
    ) -> impl Future<Output = Result<Option<OrderRecord>, RepoError>> + Send;
    /// Move to `new_state` and record `event_kind`; `RepoError::Conflict` when `expected_version`
    /// is stale (the stale writer loses). Returns the new version.
    fn advance(
        &self,
        intent_id: u128,
        expected_version: u32,
        new_state: String,
        event_kind: String,
        audit: AuditDraft,
        chain: ChainFn,
    ) -> impl Future<Output = Result<u32, RepoError>> + Send;
    /// Orders currently in `state`, oldest first, at most `limit`. More than `limit` matches is
    /// `RepoError::TooLarge` (never a silent truncation).
    fn list_in_state(
        &self,
        state: &str,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<OrderRecord>, RepoError>> + Send;
}

/// A user's decision on an approval request: a closed set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    Approve,
    Decline,
}

impl ApprovalDecision {
    /// Stable label stored in the database.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::Decline => "decline",
        }
    }

    /// Parse a stored label.
    ///
    /// # Errors
    /// `RepoError::Corrupt` for any value outside the closed set; stores use this on read.
    pub fn parse(s: &str) -> Result<Self, RepoError> {
        match s {
            "approve" => Ok(Self::Approve),
            "decline" => Ok(Self::Decline),
            _ => Err(RepoError::Corrupt),
        }
    }
}

/// Approval decisions, one per intent (first wins).
pub trait ApprovalRepo: Send + Sync {
    /// `Conflict` does NOT prove an Approve exists (it may be a decline, a stale version, a
    /// non-pending order or a lost race): callers must re-read the order state before acting on it.
    ///
    /// Record the decision, move the order to `new_state` with `event_kind`, and write the audit
    /// row in ONE transaction. The order must still be in `pending_approval` at `expected_version`
    /// (checked inside the transaction); otherwise, or when a decision already exists, the result
    /// is `RepoError::Conflict` and nothing is stored. Returns the new order version.
    #[allow(clippy::too_many_arguments)] // one atomic write: every part is needed
    fn decide_and_advance(
        &self,
        intent_id: u128,
        decision: ApprovalDecision,
        actor: &str,
        expected_version: u32,
        new_state: String,
        event_kind: String,
        audit: AuditDraft,
        chain: ChainFn,
    ) -> impl Future<Output = Result<u32, RepoError>> + Send;

    /// Startup guard, run before pending orders are reloaded: every order still in
    /// `pending_approval` that already has a decision row is inconsistent. Each is moved to
    /// `expired` with an audit row (one transaction per order) and its id returned; it is never
    /// re-presented for approval.
    fn reconcile_stray_decisions(
        &self,
        ts_ist_rfc3339: &str,
        chain: ChainFn,
    ) -> impl Future<Output = Result<Vec<u128>, RepoError>> + Send;
}

/// Persistent kill switch. A read error must be treated as engaged by the caller.
pub trait KillSwitchRepo: Send + Sync {
    /// Current state.
    fn is_engaged(&self) -> impl Future<Output = Result<bool, RepoError>> + Send;
    /// Engage or release, with its audit row in the same transaction. `reason` is the typed
    /// reason kept in the `kill_switch` row and its event history (RISK-129-1); it must not be
    /// empty.
    fn set(
        &self,
        engaged: bool,
        reason: &str,
        audit: AuditDraft,
        chain: ChainFn,
    ) -> impl Future<Output = Result<(), RepoError>> + Send;
}

/// Ledger line kind that opens an epoch. Only [`PaperLedgerRepo::new_epoch`] may store it.
pub const LEDGER_OPENING_KIND: &str = "opening";
/// Audit row kind that [`PaperLedgerRepo::new_epoch`] requires.
pub const PAPER_RESET_AUDIT_KIND: &str = "paper_reset";

/// One append-only paper ledger line; balances are derived by replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerEntry {
    pub epoch: u32,
    pub kind: String,
    /// Signed rupees (credit positive), exact decimal.
    pub amount: rust_decimal::Decimal,
    pub ref_id: Option<u128>,
    pub ts_ist_rfc3339: String,
}

/// Paper account ledger. Reset starts a new epoch and keeps the old entries.
pub trait PaperLedgerRepo: Send + Sync {
    /// Append one entry; same rules as [`Self::append_many`].
    fn append(&self, entry: LedgerEntry) -> impl Future<Output = Result<(), RepoError>> + Send;
    /// Append all `entries` in order, all-or-nothing: on error none of them is stored. Entries
    /// must target the current epoch, else `RepoError::Conflict`; an empty ledger has none, so
    /// every entry is a `Conflict` there. A [`LEDGER_OPENING_KIND`] entry is `RepoError::Other`.
    fn append_many(
        &self,
        entries: Vec<LedgerEntry>,
    ) -> impl Future<Output = Result<(), RepoError>> + Send;
    /// Entries of the current epoch in order.
    fn current_epoch(&self) -> impl Future<Output = Result<Vec<LedgerEntry>, RepoError>> + Send;
    /// Audited reset: in ONE transaction insert the opening line of epoch `opening.epoch` and the
    /// `audit` row; both or neither. `opening.epoch` must be exactly the current epoch + 1 (0 on
    /// an empty ledger counts as epoch 0), else `RepoError::Conflict` and nothing is stored (a
    /// racing second reset loses). `opening.kind` must be [`LEDGER_OPENING_KIND`] and
    /// `audit.kind` [`PAPER_RESET_AUDIT_KIND`], else `RepoError::Other`. This is the only way to
    /// start an epoch, so none starts without an audit row.
    fn new_epoch(
        &self,
        opening: LedgerEntry,
        audit: AuditDraft,
        chain: ChainFn,
    ) -> impl Future<Output = Result<u32, RepoError>> + Send;
}

/// What to read: one symbol and interval over a closed time window of bar close times.
///
/// `adjusted` is part of the key: adjusted and unadjusted bars are stored apart and never mixed
/// in one read. `from > to` is an empty range, not an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandleRange {
    pub symbol: String,
    pub interval: Interval,
    /// Inclusive lower bound on `close_time`.
    pub from: DateTime<Tz>,
    /// Inclusive upper bound on `close_time`.
    pub to: DateTime<Tz>,
    pub adjusted: bool,
}

impl CandleRange {
    /// Range over `[from, to]` (close times, inclusive) for one symbol, interval and adjusted flag.
    #[must_use]
    pub fn new(
        symbol: &str,
        interval: Interval,
        from: DateTime<Tz>,
        to: DateTime<Tz>,
        adjusted: bool,
    ) -> Self {
        Self {
            symbol: symbol.to_owned(),
            interval,
            from,
            to,
            adjusted,
        }
    }

    /// True when `c` belongs to this range (same interval and adjusted flag, close time inside).
    #[must_use]
    pub fn contains(&self, c: &Candle) -> bool {
        c.interval() == self.interval
            && c.adjusted() == self.adjusted
            && c.close_time() >= self.from
            && c.close_time() <= self.to
    }
}

/// Read closed candles. Adapters hide the storage engine; callers see only `Candle`s.
pub trait CandleSource: Send + Sync {
    /// Candles in `range`, oldest close time first. Unknown symbol or empty range is `Ok(vec![])`;
    /// a stored value that cannot be decoded is `RepoError::Corrupt`, never a zero or a panic.
    fn read_candles(
        &self,
        range: &CandleRange,
    ) -> impl Future<Output = Result<Vec<Candle>, RepoError>> + Send;

    /// Close time of the newest stored candle for the key (freshness marker), `None` if none.
    fn last_candle_ts(
        &self,
        symbol: &str,
        interval: Interval,
        adjusted: bool,
    ) -> impl Future<Output = Result<Option<DateTime<Tz>>, RepoError>> + Send;
}

/// Persist closed candles.
pub trait CandleSink: Send + Sync {
    /// Upsert `candles` for `symbol` keyed by `(symbol, interval, adjusted, close_time)`: the new
    /// row wins, no duplicate remains. All-or-nothing: on error earlier data is intact. A batch
    /// mixing intervals or adjusted flags is refused with `RepoError::Conflict` and writes
    /// nothing. Duplicate close times inside one batch: the last one wins. Updates the `last_candle_ts` marker. Returns the rows written.
    fn write_candles(
        &self,
        symbol: &str,
        candles: &[Candle],
    ) -> impl Future<Output = Result<u64, RepoError>> + Send;
}

/// Why a margin or quote source could not answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SourceError {
    /// No source is wired or it is down; callers block, never estimate.
    #[error("source unavailable")]
    Unavailable,
}

/// Required margin for an order, from the broker's own margin API (never hand-estimated).
pub trait MarginSource: Send + Sync {
    /// Margin in rupees.
    fn required_margin(
        &self,
        symbol: &str,
        qty: u32,
        is_buy: bool,
        product: &str,
    ) -> impl Future<Output = Result<rust_decimal::Decimal, SourceError>> + Send;
}

/// A fresh quote for the approval-time re-gate and paper fills.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuoteSnapshot {
    pub last: rust_decimal::Decimal,
    /// Circuit limits, when known; paper never fills at a circuit.
    pub lower_circuit: Option<rust_decimal::Decimal>,
    pub upper_circuit: Option<rust_decimal::Decimal>,
    /// Volume of the current bar, for `paper_max_participation`.
    pub bar_volume: Option<u64>,
    pub at_ist_rfc3339: String,
}

/// Quote supply.
pub trait QuoteSource: Send + Sync {
    /// Latest quote for `symbol`.
    fn quote(
        &self,
        symbol: &str,
    ) -> impl Future<Output = Result<QuoteSnapshot, SourceError>> + Send;
}

/// Why the token store failed: category only, never a path or the token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TokenError {
    /// The store could not be read or written (missing directory, permissions).
    #[error("token store unavailable")]
    Unavailable,
    /// The stored value could not be decoded; treated as "no token", never repaired.
    #[error("stored token is corrupt")]
    Corrupt,
}

/// Where the broker access token lives between runs (see ADR 0002).
///
/// `load` returns the token even if expired (the caller checks `is_valid_at` with the `Clock`).
/// Implementations write atomically with owner-only permissions and never log the value.
pub trait TokenStore: Send + Sync {
    /// The stored token, `None` if none was saved.
    fn load(
        &self,
    ) -> impl Future<Output = Result<Option<crate::secret::AccessToken>, TokenError>> + Send;

    /// Replace the stored token. All-or-nothing: on error the previous token is intact.
    fn save(
        &self,
        token: &crate::secret::AccessToken,
    ) -> impl Future<Output = Result<(), TokenError>> + Send;

    /// Remove the stored token (logout or after the broker rejects it). No token is not an error.
    fn clear(&self) -> impl Future<Output = Result<(), TokenError>> + Send;
}

#[cfg(test)]
#[allow(clippy::unused_async_trait_impl)] // the fake has nothing to await
mod token_store_tests {
    use super::tests::block;
    use super::*;
    use crate::secret::{AccessToken, Secret};
    use chrono::TimeZone;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemStore(Mutex<Option<(String, DateTime<Tz>)>>);

    impl TokenStore for MemStore {
        async fn load(&self) -> Result<Option<AccessToken>, TokenError> {
            let g = self.0.lock().map_err(|_| TokenError::Unavailable)?;
            Ok(g.as_ref()
                .map(|(s, e)| AccessToken::new(Secret::new(s.clone()), *e)))
        }
        async fn save(&self, t: &AccessToken) -> Result<(), TokenError> {
            *self.0.lock().map_err(|_| TokenError::Unavailable)? =
                Some((t.secret().expose().to_owned(), t.expires_at()));
            Ok(())
        }
        async fn clear(&self) -> Result<(), TokenError> {
            *self.0.lock().map_err(|_| TokenError::Unavailable)? = None;
            Ok(())
        }
    }

    #[test]
    fn save_load_clear_contract() {
        let store = MemStore::default();
        assert!(block(store.load()).unwrap().is_none());
        let exp = chrono_tz::Asia::Kolkata
            .with_ymd_and_hms(2026, 11, 22, 6, 0, 0)
            .unwrap();
        block(store.save(&AccessToken::new(Secret::new("t1".to_owned()), exp))).unwrap();
        let got = block(store.load()).unwrap().unwrap();
        assert_eq!(got.secret().expose(), "t1");
        assert_eq!(got.expires_at(), exp);
        block(store.clear()).unwrap();
        block(store.clear()).unwrap();
        assert!(block(store.load()).unwrap().is_none());
    }

    #[test]
    fn token_errors_carry_no_detail() {
        assert_eq!(TokenError::Corrupt.to_string(), "stored token is corrupt");
        assert_eq!(
            TokenError::Unavailable.to_string(),
            "token store unavailable"
        );
    }
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
    pub(super) fn block<T>(f: impl Future<Output = T>) -> T {
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
    fn approval_decision_round_trips_and_rejects_unknown() {
        for d in [ApprovalDecision::Approve, ApprovalDecision::Decline] {
            assert_eq!(ApprovalDecision::parse(d.as_str()), Ok(d));
        }
        assert_eq!(
            ApprovalDecision::parse("approve_all"),
            Err(RepoError::Corrupt)
        );
        assert_eq!(ApprovalDecision::parse(""), Err(RepoError::Corrupt));
    }

    #[test]
    fn repo_error_reasons_are_stable_and_carry_no_detail() {
        assert_eq!(RepoError::Unavailable.safe_reason(), "unavailable");
        assert_eq!(RepoError::Schema.safe_reason(), "schema");
        assert_eq!(RepoError::Corrupt.to_string(), "stored data is corrupt");
        assert_eq!(RepoError::Other.safe_reason(), "other");
    }
}
