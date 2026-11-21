//! Ledger-based paper cash. Balances are derived by replaying the append-only ledger of the
//! current epoch; nothing but the ledger is stored. Every fill debits a charge obtained from a
//! [`ChargeSource`] (no rates live here). Cash never goes negative: a fill the cash cannot cover
//! is refused and leaves the ledger untouched. MIS/NRML margin is the caller's concern
//! (`MarginSource`); this account covers cash only.

use std::sync::Arc;

use rust_decimal::Decimal;
use thiserror::Error;
use tokio::sync::Mutex;

use bazaar_core::clock::Clock;
use bazaar_core::ports::{LedgerEntry, PaperLedgerRepo, RepoError, SourceError};

/// Ledger entry kinds (stored as these labels).
pub mod kind {
    /// First line of an epoch: the starting virtual cash.
    pub const OPENING: &str = "opening";
    /// Cash paid for a buy fill (negative).
    pub const BUY: &str = "buy";
    /// Cash received for a sell fill (positive).
    pub const SELL: &str = "sell";
    /// Charges debited for a fill (negative).
    pub const CHARGE: &str = "charge";
    /// Compensation when a fill's trade line failed to write after its charge line.
    pub const REVERSAL: &str = "reversal";
}

/// Port: total charges (brokerage, taxes, fees) for one fill, from the F9 cost model.
/// `Unavailable` means no estimate is made: the fill is refused.
pub trait ChargeSource: Send + Sync {
    /// Charge in rupees (never negative) for a fill of `qty` at `price`.
    ///
    /// # Errors
    /// `SourceError::Unavailable` when no cost model is wired.
    fn charge(
        &self,
        symbol: &str,
        is_buy: bool,
        qty: u32,
        price: Decimal,
        product: &str,
    ) -> Result<Decimal, SourceError>;
}

/// One simulated fill to book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FillBooking {
    /// Intent id (as u128) the fill belongs to.
    pub ref_id: u128,
    pub symbol: String,
    pub is_buy: bool,
    pub qty: u32,
    pub price: Decimal,
    /// Product label for the cost model (`Product::as_str`).
    pub product: String,
}

/// Balances derived from the current epoch's ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Balances {
    pub epoch: Option<u32>,
    /// Virtual cash available (sum of all entries).
    pub cash: Decimal,
    /// Total charges paid (positive number).
    pub charges_paid: Decimal,
    pub entries: usize,
}

/// Why the account refused or failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AccountError {
    #[error("insufficient paper cash")]
    InsufficientFunds,
    #[error("invalid fill (zero qty, non-positive price or overflow)")]
    InvalidFill,
    #[error("charges unavailable; fill refused")]
    ChargesUnavailable,
    #[error("negative charge from source")]
    BadCharge,
    #[error("no opening balance; reset the account first")]
    NotOpened,
    #[error("ledger: {0}")]
    Ledger(RepoError),
}

impl From<RepoError> for AccountError {
    fn from(e: RepoError) -> Self {
        Self::Ledger(e)
    }
}

/// Derive balances from ledger lines (pure replay).
///
/// # Errors
/// `InvalidFill` on decimal overflow.
pub fn replay(entries: &[LedgerEntry]) -> Result<Balances, AccountError> {
    let mut cash = Decimal::ZERO;
    let mut charges = Decimal::ZERO;
    for e in entries {
        cash = cash
            .checked_add(e.amount)
            .ok_or(AccountError::InvalidFill)?;
        if e.kind == kind::CHARGE {
            charges = charges
                .checked_sub(e.amount)
                .ok_or(AccountError::InvalidFill)?;
        }
    }
    Ok(Balances {
        epoch: entries.first().map(|e| e.epoch),
        cash,
        charges_paid: charges,
        entries: entries.len(),
    })
}

/// Paper cash over a [`PaperLedgerRepo`]. Writes are serialised by an internal lock so the funds
/// check and the append cannot interleave within one account.
pub struct PaperAccount<R, C> {
    repo: R,
    charges: C,
    clock: Arc<dyn Clock>,
    write: Mutex<()>,
}

impl<R, C> std::fmt::Debug for PaperAccount<R, C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaperAccount").finish_non_exhaustive()
    }
}

impl<R: PaperLedgerRepo, C: ChargeSource> PaperAccount<R, C> {
    /// New account handle; the ledger itself lives in `repo`.
    pub fn new(repo: R, charges: C, clock: Arc<dyn Clock>) -> Self {
        Self {
            repo,
            charges,
            clock,
            write: Mutex::new(()),
        }
    }

    fn entry(&self, epoch: u32, kind: &str, amount: Decimal, ref_id: Option<u128>) -> LedgerEntry {
        LedgerEntry {
            epoch,
            kind: kind.to_owned(),
            amount,
            ref_id,
            ts_ist_rfc3339: self.clock.now_ist().to_rfc3339(),
        }
    }

    /// Balances replayed from the current epoch.
    ///
    /// # Errors
    /// Ledger read failure.
    pub async fn balances(&self) -> Result<Balances, AccountError> {
        replay(&self.repo.current_epoch().await?)
    }

    /// Start a new epoch with `opening` cash; the old epoch's ledger is kept. Returns the epoch.
    ///
    /// # Errors
    /// `InvalidFill` if `opening` is negative; ledger failure.
    pub async fn reset(&self, opening: Decimal) -> Result<u32, AccountError> {
        if opening < Decimal::ZERO {
            return Err(AccountError::InvalidFill);
        }
        let _g = self.write.lock().await;
        let e = self.entry(0, kind::OPENING, opening, None);
        Ok(self.repo.new_epoch(e).await?)
    }

    /// Signed trade leg and signed (negative) charge leg after validation.
    fn legs(&self, b: &FillBooking) -> Result<(Decimal, Decimal), AccountError> {
        if b.qty == 0 || b.price <= Decimal::ZERO {
            return Err(AccountError::InvalidFill);
        }
        let notional = b
            .price
            .checked_mul(Decimal::from(b.qty))
            .ok_or(AccountError::InvalidFill)?;
        let charge = self
            .charges
            .charge(&b.symbol, b.is_buy, b.qty, b.price, &b.product)
            .map_err(|_| AccountError::ChargesUnavailable)?;
        if charge < Decimal::ZERO {
            return Err(AccountError::BadCharge);
        }
        Ok((if b.is_buy { -notional } else { notional }, -charge))
    }

    /// Funds check without booking: would cash cover this fill and its charges?
    ///
    /// # Errors
    /// `InsufficientFunds` and the validation errors of [`Self::apply_fill`].
    pub async fn check_funds(&self, b: &FillBooking) -> Result<(), AccountError> {
        let _g = self.write.lock().await;
        self.check_locked(b).await.map(|_| ())
    }

    async fn check_locked(&self, b: &FillBooking) -> Result<(u32, Decimal, Decimal), AccountError> {
        let (trade, charge) = self.legs(b)?;
        let bal = self.balances().await?;
        let epoch = bal.epoch.ok_or(AccountError::NotOpened)?;
        let after = bal
            .cash
            .checked_add(trade)
            .and_then(|c| c.checked_add(charge))
            .ok_or(AccountError::InvalidFill)?;
        if after < Decimal::ZERO {
            return Err(AccountError::InsufficientFunds);
        }
        Ok((epoch, trade, charge))
    }

    /// Book a fill: funds check, then a charge line and a trade line. A refused fill writes
    /// nothing. If the trade line fails after the charge line, a reversal line is attempted.
    ///
    /// # Errors
    /// `InsufficientFunds`, `InvalidFill`, `ChargesUnavailable`, `BadCharge`, `NotOpened`, `Ledger`.
    pub async fn apply_fill(&self, b: &FillBooking) -> Result<Balances, AccountError> {
        let _g = self.write.lock().await;
        let (epoch, trade, charge) = self.check_locked(b).await?;
        let tk = if b.is_buy { kind::BUY } else { kind::SELL };
        self.repo
            .append(self.entry(epoch, kind::CHARGE, charge, Some(b.ref_id)))
            .await?;
        if let Err(e) = self
            .repo
            .append(self.entry(epoch, tk, trade, Some(b.ref_id)))
            .await
        {
            // Best effort: undo the charge so the ledger shows no half fill.
            let _ = self
                .repo
                .append(self.entry(epoch, kind::REVERSAL, -charge, Some(b.ref_id)))
                .await;
            return Err(e.into());
        }
        self.balances().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    use bazaar_core::testutil::FakeClock;
    use chrono::TimeZone;
    use chrono_tz::Asia::Kolkata;
    use proptest::prelude::*;

    fn d(s: &str) -> Decimal {
        s.parse().expect("decimal literal")
    }

    #[derive(Default)]
    struct FakeRepo {
        epochs: StdMutex<Vec<Vec<LedgerEntry>>>,
        fail_kind: StdMutex<Option<&'static str>>,
    }
    struct Shared(Arc<FakeRepo>);
    impl PaperLedgerRepo for Shared {
        async fn append(&self, e: LedgerEntry) -> Result<(), RepoError> {
            self.0.append(e).await
        }
        async fn current_epoch(&self) -> Result<Vec<LedgerEntry>, RepoError> {
            self.0.current_epoch().await
        }
        async fn new_epoch(&self, o: LedgerEntry) -> Result<u32, RepoError> {
            self.0.new_epoch(o).await
        }
    }
    impl FakeRepo {
        async fn append(&self, e: LedgerEntry) -> Result<(), RepoError> {
            if self.fail_kind.lock().unwrap().is_some_and(|k| k == e.kind) {
                return Err(RepoError::Unavailable);
            }
            let mut g = self.epochs.lock().unwrap();
            g.last_mut().ok_or(RepoError::Conflict)?.push(e);
            Ok(())
        }
        async fn current_epoch(&self) -> Result<Vec<LedgerEntry>, RepoError> {
            Ok(self
                .epochs
                .lock()
                .unwrap()
                .last()
                .cloned()
                .unwrap_or_default())
        }
        async fn new_epoch(&self, mut opening: LedgerEntry) -> Result<u32, RepoError> {
            let mut g = self.epochs.lock().unwrap();
            let n = u32::try_from(g.len()).unwrap() + 1;
            opening.epoch = n;
            g.push(vec![opening]);
            Ok(n)
        }
    }

    /// Fake cost model: a fixed per-fill amount, or unavailable.
    struct FakeCharges(Option<Decimal>);
    impl ChargeSource for FakeCharges {
        fn charge(
            &self,
            _: &str,
            _: bool,
            _: u32,
            _: Decimal,
            _: &str,
        ) -> Result<Decimal, SourceError> {
            self.0.ok_or(SourceError::Unavailable)
        }
    }

    type Acct = PaperAccount<Shared, FakeCharges>;

    fn acct(charge: Option<Decimal>) -> (Acct, Arc<FakeRepo>) {
        let repo = Arc::new(FakeRepo::default());
        let clock = Arc::new(FakeClock::at(
            Kolkata.with_ymd_and_hms(2026, 1, 5, 10, 0, 0).unwrap(),
        ));
        (
            PaperAccount::new(Shared(repo.clone()), FakeCharges(charge), clock),
            repo,
        )
    }

    fn fill(buy: bool, qty: u32, price: &str) -> FillBooking {
        FillBooking {
            ref_id: 7,
            symbol: "AAA".into(),
            is_buy: buy,
            qty,
            price: d(price),
            product: "cnc".into(),
        }
    }

    #[tokio::test]
    async fn buy_debits_cost_and_charge_sell_credits() {
        let (a, _) = acct(Some(d("20")));
        a.reset(d("10000")).await.unwrap();
        let b = a.apply_fill(&fill(true, 10, "100")).await.unwrap();
        assert_eq!(b.cash, d("8980"));
        assert_eq!(b.charges_paid, d("20"));
        let b = a.apply_fill(&fill(false, 10, "101")).await.unwrap();
        assert_eq!(b.cash, d("9970"));
        assert_eq!(b.charges_paid, d("40"));
    }

    #[tokio::test]
    async fn refuses_what_cash_cannot_cover_and_ledger_is_untouched() {
        let (a, repo) = acct(Some(d("20")));
        a.reset(d("1000")).await.unwrap();
        // 1000 notional + 20 charge > 1000 cash.
        assert_eq!(
            a.apply_fill(&fill(true, 10, "100")).await,
            Err(AccountError::InsufficientFunds)
        );
        assert_eq!(
            a.check_funds(&fill(true, 10, "100")).await,
            Err(AccountError::InsufficientFunds)
        );
        assert_eq!(repo.epochs.lock().unwrap()[0].len(), 1);
        assert!(a.check_funds(&fill(true, 9, "100")).await.is_ok());
    }

    #[tokio::test]
    async fn invalid_fills_and_unavailable_charges_are_refused() {
        let (a, repo) = acct(Some(d("1")));
        assert_eq!(
            a.apply_fill(&fill(true, 1, "1")).await,
            Err(AccountError::NotOpened)
        );
        a.reset(d("100")).await.unwrap();
        assert_eq!(
            a.apply_fill(&fill(true, 0, "1")).await,
            Err(AccountError::InvalidFill)
        );
        assert_eq!(
            a.apply_fill(&fill(true, 1, "0")).await,
            Err(AccountError::InvalidFill)
        );
        assert_eq!(a.reset(d("-1")).await, Err(AccountError::InvalidFill));
        assert_eq!(repo.epochs.lock().unwrap()[0].len(), 1);
        let (n, _) = acct(None);
        n.reset(d("100")).await.unwrap();
        assert_eq!(
            n.apply_fill(&fill(true, 1, "1")).await,
            Err(AccountError::ChargesUnavailable)
        );
        let (neg, _) = acct(Some(d("-1")));
        neg.reset(d("100")).await.unwrap();
        assert_eq!(
            neg.apply_fill(&fill(true, 1, "1")).await,
            Err(AccountError::BadCharge)
        );
    }

    #[tokio::test]
    async fn reset_starts_new_epoch_and_keeps_old_ledger() {
        let (a, repo) = acct(Some(d("1")));
        assert_eq!(a.reset(d("500")).await.unwrap(), 1);
        a.apply_fill(&fill(true, 1, "100")).await.unwrap();
        assert_eq!(a.reset(d("200")).await.unwrap(), 2);
        let b = a.balances().await.unwrap();
        assert_eq!((b.epoch, b.cash, b.entries), (Some(2), d("200"), 1));
        assert_eq!(repo.epochs.lock().unwrap()[0].len(), 3);
    }

    #[tokio::test]
    async fn failed_trade_line_is_reversed() {
        let (a, repo) = acct(Some(d("5")));
        a.reset(d("1000")).await.unwrap();
        *repo.fail_kind.lock().unwrap() = Some(kind::BUY);
        assert_eq!(
            a.apply_fill(&fill(true, 1, "100")).await,
            Err(AccountError::Ledger(RepoError::Unavailable))
        );
        assert_eq!(a.balances().await.unwrap().cash, d("1000"));
    }

    #[test]
    fn replay_overflow_is_an_error_not_a_panic() {
        let e = |amount| LedgerEntry {
            epoch: 1,
            kind: kind::OPENING.into(),
            amount,
            ref_id: None,
            ts_ist_rfc3339: String::new(),
        };
        assert_eq!(
            replay(&[e(Decimal::MAX), e(Decimal::MAX)]),
            Err(AccountError::InvalidFill)
        );
    }

    proptest! {
        /// Random fills: cash never negative, equals the sum of ledger lines, a refused fill
        /// changes nothing, and a fresh account over the same ledger replays identical balances.
        #[test]
        fn replay_reproduces_balances(
            opening in 0u32..100_000,
            charge in 0u32..50,
            ops in proptest::collection::vec((any::<bool>(), 1u32..50, 1u32..500), 0..40),
        ) {
            let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
            rt.block_on(async {
                let (a, repo) = acct(Some(Decimal::from(charge)));
                a.reset(Decimal::from(opening)).await.unwrap();
                for (buy, qty, price) in ops {
                    let before = a.balances().await.unwrap();
                    match a.apply_fill(&fill(buy, qty, &price.to_string())).await {
                        Ok(after) => prop_assert!(after.cash >= Decimal::ZERO),
                        Err(AccountError::InsufficientFunds) => {
                            prop_assert_eq!(a.balances().await.unwrap(), before);
                        }
                        Err(e) => prop_assert!(false, "unexpected {e}"),
                    }
                }
                let live = a.balances().await.unwrap();
                let sum: Decimal = repo.epochs.lock().unwrap()[0].iter().map(|e| e.amount).sum();
                prop_assert_eq!(live.cash, sum);
                let (fresh, _) = acct(None);
                let again = PaperAccount::new(Shared(repo.clone()), FakeCharges(None), fresh.clock);
                prop_assert_eq!(again.balances().await.unwrap(), live);
                Ok(())
            })?;
        }
    }
}
