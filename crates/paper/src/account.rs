//! Ledger-based paper cash. Balances are derived by replaying the append-only ledger of the
//! current epoch; nothing but the ledger is stored. Every fill debits a charge obtained from a
//! [`ChargeSource`] (no rates live here). Cash never goes negative: a fill the cash cannot cover
//! is refused and leaves the ledger untouched. A sell above the held quantity is refused (no
//! cash from nothing). MIS/NRML margin and intraday shorting are the caller's concern (slice 5,
//! `MarginSource`); this account covers cash only.
//!
//! Single-writer rule: exactly one [`PaperAccount`] may exist per ledger. Its internal lock
//! serialises the funds check and the append within that one handle; a second handle or process
//! over the same ledger could interleave and overdraw. The wiring slice constructs it once.

use std::sync::Arc;

use rust_decimal::Decimal;
use thiserror::Error;
use tokio::sync::Mutex;

use bazaar_core::clock::Clock;
use bazaar_core::ports::{
    AuditDraft, LEDGER_OPENING_KIND, LedgerEntry, PAPER_RESET_AUDIT_KIND, PaperLedgerRepo,
    RepoError, SourceError,
};
use bazaar_execution::audit::chain_hash;

/// Ledger entry kinds; stored as their `as_str` label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerKind {
    /// First line of an epoch: the starting virtual cash.
    Opening,
    /// Cash paid for a buy fill (negative).
    Buy,
    /// Cash received for a sell fill (positive).
    Sell,
    /// Charges debited for a fill (negative).
    Charge,
}

impl LedgerKind {
    /// Stable storage label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Opening => LEDGER_OPENING_KIND,
            Self::Buy => "buy",
            Self::Sell => "sell",
            Self::Charge => "charge",
        }
    }

    /// Parse a stored label; `None` for anything unknown.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        [Self::Opening, Self::Buy, Self::Sell, Self::Charge]
            .into_iter()
            .find(|k| k.as_str() == s)
    }
}

/// Why a paper reset is requested. Typed on purpose: no free text reaches the audit row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetReason {
    /// First open of an empty ledger (the only reason that skips the guard).
    Initial,
    /// The owner asked for a fresh account.
    OwnerRequested,
}

impl ResetReason {
    /// Stable label stored in the audit row.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Initial => "initial",
            Self::OwnerRequested => "owner_requested",
        }
    }
}

/// A reset request. The opening amount is never part of it: it comes from config only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetRequest {
    pub reason: ResetReason,
    /// Who asked: 1..=64 chars of `[A-Za-z0-9_.:-]` (it lands in the audit JSON verbatim).
    pub actor: String,
}

/// The rule that refused a reset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetRule {
    /// The circuit breaker is latched.
    BreakerLatched,
    /// Net P&L of the current IST week (all epochs) is negative.
    WeeklyLoss,
    /// The guard could not read what it needs: unknown refuses.
    GuardUnavailable,
    /// `Initial` asked on a ledger that already has an epoch.
    InitialOnOpenedLedger,
}

/// Port: may the owner reset now? Implemented over the ledger week and the breaker latch by the
/// server wiring. An unreadable answer must be `GuardUnavailable`, never `Ok`.
pub trait ResetGuard: Send + Sync {
    /// # Errors
    /// The [`ResetRule`] that refuses.
    fn check(&self) -> impl Future<Output = Result<(), ResetRule>> + Send;
}

/// Guard that refuses every reset (fail-closed default until a real guard is wired).
#[derive(Debug, Clone, Copy)]
pub struct RefuseAll;

impl ResetGuard for RefuseAll {
    fn check(&self) -> impl Future<Output = Result<(), ResetRule>> + Send {
        std::future::ready(Err(ResetRule::GuardUnavailable))
    }
}

/// Port: total charges (brokerage, taxes, fees) for one fill, from the F9 cost model.
/// `Unavailable` means no estimate is made: the fill is refused. The app wiring must supply the
/// real cost model; a source returning zero for a non-zero notional is refused as `BadCharge`.
pub trait ChargeSource: Send + Sync {
    /// Charge in rupees for a fill of `qty` at `price`.
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
    /// Quantity of `symbol` the position book holds now; a sell above it is refused.
    pub held_qty: u32,
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
    #[error("sell above held quantity")]
    NoPosition,
    #[error("invalid fill (zero qty, non-positive price or overflow)")]
    InvalidFill,
    #[error("charges unavailable; fill refused")]
    ChargesUnavailable,
    #[error("charge from source is negative, or zero for a non-zero notional")]
    BadCharge,
    #[error("reset refused: {0:?}")]
    ResetRefused(ResetRule),
    #[error("reset actor must be 1..=64 chars of [A-Za-z0-9_.:-]")]
    BadActor,
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

/// Derive balances from ledger lines (pure replay). An unknown kind is a corrupt ledger.
///
/// # Errors
/// `InvalidFill` on decimal overflow; `Ledger(Corrupt)` on an unknown kind.
pub fn replay(entries: &[LedgerEntry]) -> Result<Balances, AccountError> {
    let mut cash = Decimal::ZERO;
    let mut charges = Decimal::ZERO;
    for e in entries {
        let kind = LedgerKind::parse(&e.kind).ok_or(AccountError::Ledger(RepoError::Corrupt))?;
        cash = cash
            .checked_add(e.amount)
            .ok_or(AccountError::InvalidFill)?;
        if kind == LedgerKind::Charge {
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

/// Paper cash over a [`PaperLedgerRepo`]. One instance per ledger (see the module docs).
pub struct PaperAccount<R, C> {
    repo: R,
    charges: C,
    clock: Arc<dyn Clock>,
    starting_cash: Decimal,
    write: Mutex<()>,
}

impl<R, C> std::fmt::Debug for PaperAccount<R, C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaperAccount").finish_non_exhaustive()
    }
}

impl<R: PaperLedgerRepo, C: ChargeSource> PaperAccount<R, C> {
    /// New account handle; the ledger itself lives in `repo`. `starting_cash` is
    /// `[execution] paper_starting_cash_inr`; it is the only opening amount a reset can use.
    pub fn new(repo: R, charges: C, clock: Arc<dyn Clock>, starting_cash: Decimal) -> Self {
        Self {
            repo,
            charges,
            clock,
            starting_cash,
            write: Mutex::new(()),
        }
    }

    fn entry(
        &self,
        epoch: u32,
        kind: LedgerKind,
        amount: Decimal,
        ref_id: Option<u128>,
    ) -> LedgerEntry {
        LedgerEntry {
            epoch,
            kind: kind.as_str().to_owned(),
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

    /// Audited reset: a new epoch with the configured starting cash, its opening line and an
    /// audit row (`paper_reset`: actor, epoch, opening, reason) in ONE store transaction. The
    /// old epoch's ledger is kept. `Initial` is allowed only on an empty ledger and skips the
    /// guard; every other reason needs `guard` to say yes (a refusal writes nothing).
    ///
    /// # Errors
    /// `ResetRefused(rule)`; `BadActor`; `InvalidFill` if the configured starting cash is not
    /// positive; ledger failure (`Conflict` when a racing reset won).
    pub async fn reset<G: ResetGuard>(
        &self,
        req: &ResetRequest,
        guard: &G,
    ) -> Result<u32, AccountError> {
        let ok = |c: char| c.is_ascii_alphanumeric() || "_.:-".contains(c);
        if req.actor.is_empty() || req.actor.len() > 64 || !req.actor.chars().all(ok) {
            return Err(AccountError::BadActor);
        }
        if self.starting_cash <= Decimal::ZERO {
            return Err(AccountError::InvalidFill);
        }
        let _g = self.write.lock().await;
        let epoch = self.balances().await?.epoch;
        match (req.reason, epoch) {
            (ResetReason::Initial, Some(_)) => {
                return Err(AccountError::ResetRefused(ResetRule::InitialOnOpenedLedger));
            }
            (ResetReason::Initial, None) => {}
            (ResetReason::OwnerRequested, _) => {
                guard.check().await.map_err(AccountError::ResetRefused)?;
            }
        }
        let next = epoch
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(AccountError::Ledger(RepoError::Other))?;
        let e = self.entry(next, LedgerKind::Opening, self.starting_cash, None);
        let audit = AuditDraft {
            ts_ist_rfc3339: e.ts_ist_rfc3339.clone(),
            actor: req.actor.clone(),
            kind: PAPER_RESET_AUDIT_KIND.to_owned(),
            // Canonical JSON: sorted keys, no whitespace; every value is typed or validated.
            payload: format!(
                "{{\"actor\":\"{}\",\"epoch\":{next},\"opening\":\"{}\",\"reason\":\"{}\"}}",
                req.actor,
                self.starting_cash,
                req.reason.as_str()
            ),
        };
        Ok(self.repo.new_epoch(e, audit, chain_hash).await?)
    }

    /// Signed trade leg and signed (negative) charge leg after validation.
    fn legs(&self, b: &FillBooking) -> Result<(Decimal, Decimal), AccountError> {
        if b.qty == 0 || b.price <= Decimal::ZERO {
            return Err(AccountError::InvalidFill);
        }
        if !b.is_buy && b.qty > b.held_qty {
            return Err(AccountError::NoPosition);
        }
        let notional = b
            .price
            .checked_mul(Decimal::from(b.qty))
            .ok_or(AccountError::InvalidFill)?;
        let charge = self
            .charges
            .charge(&b.symbol, b.is_buy, b.qty, b.price, &b.product)
            .map_err(|_| AccountError::ChargesUnavailable)?;
        if charge <= Decimal::ZERO {
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

    /// Book a fill: funds check, then the charge line and the trade line in one all-or-nothing
    /// `append_many`. A refused or failed fill leaves the ledger unchanged.
    ///
    /// # Errors
    /// `InsufficientFunds`, `NoPosition`, `InvalidFill`, `ChargesUnavailable`, `BadCharge`,
    /// `NotOpened`, `Ledger`.
    pub async fn apply_fill(&self, b: &FillBooking) -> Result<Balances, AccountError> {
        let _g = self.write.lock().await;
        let (epoch, trade, charge) = self.check_locked(b).await?;
        let tk = if b.is_buy {
            LedgerKind::Buy
        } else {
            LedgerKind::Sell
        };
        let lines = vec![
            self.entry(epoch, LedgerKind::Charge, charge, Some(b.ref_id)),
            self.entry(epoch, tk, trade, Some(b.ref_id)),
        ];
        self.repo.append_many(lines).await?;
        self.balances().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;
    use std::sync::atomic::{AtomicBool, Ordering};

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
        fail_next: AtomicBool,
        fail_audit: AtomicBool,
        fail_read: AtomicBool,
        audits: StdMutex<Vec<AuditDraft>>,
    }
    impl FakeRepo {
        fn lines(&self, epoch_idx: usize) -> Vec<LedgerEntry> {
            self.epochs.lock().unwrap()[epoch_idx].clone()
        }
    }
    struct Shared(Arc<FakeRepo>);
    impl PaperLedgerRepo for Shared {
        async fn append(&self, e: LedgerEntry) -> Result<(), RepoError> {
            self.append_many(vec![e]).await
        }
        async fn append_many(&self, es: Vec<LedgerEntry>) -> Result<(), RepoError> {
            if self.0.fail_next.swap(false, Ordering::SeqCst) {
                return Err(RepoError::Unavailable);
            }
            let mut g = self.0.epochs.lock().unwrap();
            g.last_mut().ok_or(RepoError::Conflict)?.extend(es);
            Ok(())
        }
        async fn current_epoch(&self) -> Result<Vec<LedgerEntry>, RepoError> {
            if self.0.fail_read.load(Ordering::SeqCst) {
                return Err(RepoError::Unavailable);
            }
            let snapshot = self
                .0
                .epochs
                .lock()
                .unwrap()
                .last()
                .cloned()
                .unwrap_or_default();
            // Like a real store: other tasks may run between this read and the caller's write.
            tokio::task::yield_now().await;
            Ok(snapshot)
        }
        async fn new_epoch(
            &self,
            opening: LedgerEntry,
            audit: AuditDraft,
            _: bazaar_core::ports::ChainFn,
        ) -> Result<u32, RepoError> {
            let mut g = self.0.epochs.lock().unwrap();
            let n = u32::try_from(g.len()).unwrap() + 1;
            if opening.epoch != n || self.0.fail_audit.load(Ordering::SeqCst) {
                return Err(RepoError::Conflict);
            }
            self.0.audits.lock().unwrap().push(audit);
            g.push(vec![opening]);
            Ok(n)
        }
    }

    /// Fake cost model: `base + per_unit * qty`, or unavailable.
    struct FakeCharges {
        base: Option<Decimal>,
        per_unit: Decimal,
    }
    impl ChargeSource for FakeCharges {
        fn charge(
            &self,
            _: &str,
            _: bool,
            q: u32,
            _: Decimal,
            _: &str,
        ) -> Result<Decimal, SourceError> {
            self.base
                .map(|b| b + self.per_unit * Decimal::from(q))
                .ok_or(SourceError::Unavailable)
        }
    }

    struct Allow;
    impl ResetGuard for Allow {
        async fn check(&self) -> Result<(), ResetRule> {
            Ok(())
        }
    }
    struct Deny(ResetRule);
    impl ResetGuard for Deny {
        async fn check(&self) -> Result<(), ResetRule> {
            Err(self.0)
        }
    }
    fn req(reason: ResetReason) -> ResetRequest {
        ResetRequest {
            reason,
            actor: "owner".into(),
        }
    }
    /// First open: the only reset that skips the guard.
    async fn open(a: &Acct) -> u32 {
        a.reset(&req(ResetReason::Initial), &RefuseAll)
            .await
            .unwrap()
    }

    type Acct = PaperAccount<Shared, FakeCharges>;

    fn clock() -> Arc<FakeClock> {
        Arc::new(FakeClock::at(
            Kolkata.with_ymd_and_hms(2026, 1, 5, 10, 0, 0).unwrap(),
        ))
    }

    fn acct_with(base: Option<Decimal>, per_unit: Decimal, start: &str) -> (Acct, Arc<FakeRepo>) {
        let repo = Arc::new(FakeRepo::default());
        let ch = FakeCharges { base, per_unit };
        (
            PaperAccount::new(Shared(repo.clone()), ch, clock(), d(start)),
            repo,
        )
    }

    fn acct(charge: Option<Decimal>, start: &str) -> (Acct, Arc<FakeRepo>) {
        acct_with(charge, Decimal::ZERO, start)
    }

    /// A fill with a position book that backs any sell (held far above qty).
    fn fill(buy: bool, qty: u32, price: &str) -> FillBooking {
        FillBooking {
            ref_id: 7,
            symbol: "AAA".into(),
            is_buy: buy,
            qty,
            price: d(price),
            product: "cnc".into(),
            held_qty: 1_000_000,
        }
    }

    #[tokio::test]
    async fn buy_debits_cost_and_charge_sell_credits() {
        let (a, _) = acct(Some(d("20")), "10000");
        open(&a).await;
        let b = a.apply_fill(&fill(true, 10, "100")).await.unwrap();
        assert_eq!(b.cash, d("8980"));
        assert_eq!(b.charges_paid, d("20"));
        let b = a.apply_fill(&fill(false, 10, "101")).await.unwrap();
        assert_eq!(b.cash, d("9970"));
        assert_eq!(b.charges_paid, d("40"));
    }

    #[tokio::test]
    async fn booked_lines_carry_kind_epoch_ref_and_ist_timestamp() {
        let (a, repo) = acct(Some(d("20")), "10000");
        open(&a).await;
        a.apply_fill(&fill(true, 10, "100")).await.unwrap();
        let l = repo.lines(0);
        assert_eq!(l.len(), 3);
        assert_eq!(l[0].kind, "opening");
        assert_eq!(
            (l[1].kind.as_str(), l[1].amount, l[1].ref_id),
            ("charge", d("-20"), Some(7))
        );
        assert_eq!(
            (l[2].kind.as_str(), l[2].amount, l[2].ref_id),
            ("buy", d("-1000"), Some(7))
        );
        for e in &l {
            assert_eq!(e.epoch, 1);
            assert_eq!(e.ts_ist_rfc3339, "2026-01-05T10:00:00+05:30");
        }
    }

    #[tokio::test]
    async fn exact_cash_is_accepted_one_rupee_less_is_not() {
        let (a, _) = acct(Some(d("20")), "1020");
        open(&a).await;
        a.check_funds(&fill(true, 10, "100")).await.unwrap();
        assert_eq!(
            a.apply_fill(&fill(true, 10, "100")).await.unwrap().cash,
            Decimal::ZERO
        );
        let (b, _) = acct(Some(d("20")), "1019");
        open(&b).await;
        assert_eq!(
            b.check_funds(&fill(true, 10, "100")).await,
            Err(AccountError::InsufficientFunds)
        );
    }

    #[tokio::test]
    async fn refuses_what_cash_cannot_cover_and_ledger_is_untouched() {
        let (a, repo) = acct(Some(d("20")), "1000");
        open(&a).await;
        assert_eq!(
            a.apply_fill(&fill(true, 10, "100")).await,
            Err(AccountError::InsufficientFunds)
        );
        assert_eq!(repo.lines(0).len(), 1);
        assert!(a.check_funds(&fill(true, 9, "100")).await.is_ok());
    }

    #[tokio::test]
    async fn sell_above_held_quantity_is_refused_and_creates_no_cash() {
        let (a, repo) = acct(Some(d("20")), "1000");
        open(&a).await;
        let mut s = fill(false, 10, "100");
        s.held_qty = 9;
        assert_eq!(a.apply_fill(&s).await, Err(AccountError::NoPosition));
        assert_eq!(a.check_funds(&s).await, Err(AccountError::NoPosition));
        s.held_qty = 0;
        assert_eq!(a.apply_fill(&s).await, Err(AccountError::NoPosition));
        assert_eq!(repo.lines(0).len(), 1);
        s.held_qty = 10;
        assert_eq!(a.apply_fill(&s).await.unwrap().cash, d("1980"));
    }

    #[tokio::test]
    async fn invalid_fills_and_bad_charges_are_refused_by_check_and_apply() {
        let (a, repo) = acct(Some(d("1")), "100");
        assert_eq!(
            a.apply_fill(&fill(true, 1, "1")).await,
            Err(AccountError::NotOpened)
        );
        assert_eq!(
            a.check_funds(&fill(true, 1, "1")).await,
            Err(AccountError::NotOpened)
        );
        open(&a).await;
        assert_eq!(
            a.apply_fill(&fill(true, 0, "1")).await,
            Err(AccountError::InvalidFill)
        );
        assert_eq!(
            a.apply_fill(&fill(true, 1, "0")).await,
            Err(AccountError::InvalidFill)
        );
        assert_eq!(repo.lines(0).len(), 1);
        for (c, want) in [
            (None, AccountError::ChargesUnavailable),
            (Some(d("-1")), AccountError::BadCharge),
            (Some(Decimal::ZERO), AccountError::BadCharge),
        ] {
            let (n, r) = acct(c, "100");
            open(&n).await;
            assert_eq!(n.apply_fill(&fill(true, 1, "1")).await, Err(want));
            assert_eq!(n.check_funds(&fill(true, 1, "1")).await, Err(want));
            assert_eq!(r.lines(0).len(), 1);
        }
    }

    #[tokio::test]
    async fn reset_uses_configured_cash_and_rejects_non_positive() {
        for start in ["0", "-5"] {
            let (a, repo) = acct(Some(d("1")), start);
            assert_eq!(
                a.reset(&req(ResetReason::Initial), &RefuseAll).await,
                Err(AccountError::InvalidFill)
            );
            assert!(repo.epochs.lock().unwrap().is_empty());
            assert!(repo.audits.lock().unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn reset_writes_epoch_opening_and_audit_row_and_keeps_old_ledger() {
        let (a, repo) = acct(Some(d("1")), "500");
        assert_eq!(open(&a).await, 1);
        a.apply_fill(&fill(true, 1, "100")).await.unwrap();
        let old = repo.lines(0);
        assert_eq!(
            a.reset(&req(ResetReason::OwnerRequested), &Allow).await,
            Ok(2)
        );
        let b = a.balances().await.unwrap();
        assert_eq!((b.epoch, b.cash, b.entries), (Some(2), d("500"), 1));
        assert_eq!(repo.lines(0), old, "old epoch untouched");
        let au = repo.audits.lock().unwrap().clone();
        assert_eq!(au.len(), 2);
        let (first, second) = (&au[0], &au[1]);
        assert_eq!(
            (first.kind.as_str(), first.actor.as_str()),
            ("paper_reset", "owner")
        );
        assert_eq!(
            first.payload,
            r#"{"actor":"owner","epoch":1,"opening":"500","reason":"initial"}"#
        );
        assert_eq!(
            second.payload,
            r#"{"actor":"owner","epoch":2,"opening":"500","reason":"owner_requested"}"#
        );
        assert_eq!(second.ts_ist_rfc3339, repo.lines(1)[0].ts_ist_rfc3339);
    }

    #[tokio::test]
    async fn guard_refusals_name_the_rule_and_write_nothing() {
        let (a, repo) = acct(Some(d("1")), "500");
        open(&a).await;
        for rule in [
            ResetRule::BreakerLatched,
            ResetRule::WeeklyLoss,
            ResetRule::GuardUnavailable,
        ] {
            assert_eq!(
                a.reset(&req(ResetReason::OwnerRequested), &Deny(rule))
                    .await,
                Err(AccountError::ResetRefused(rule))
            );
        }
        // The fail-closed default refuses too, and so does Initial on an opened ledger.
        assert_eq!(
            a.reset(&req(ResetReason::OwnerRequested), &RefuseAll).await,
            Err(AccountError::ResetRefused(ResetRule::GuardUnavailable))
        );
        assert_eq!(
            a.reset(&req(ResetReason::Initial), &Allow).await,
            Err(AccountError::ResetRefused(ResetRule::InitialOnOpenedLedger))
        );
        assert_eq!(repo.epochs.lock().unwrap().len(), 1);
        assert_eq!(repo.audits.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn owner_reset_of_an_empty_ledger_still_asks_the_guard() {
        let (a, repo) = acct(Some(d("1")), "500");
        assert_eq!(
            a.reset(&req(ResetReason::OwnerRequested), &RefuseAll).await,
            Err(AccountError::ResetRefused(ResetRule::GuardUnavailable))
        );
        assert!(repo.epochs.lock().unwrap().is_empty());
        assert!(repo.audits.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn reset_with_an_unreadable_ledger_writes_nothing() {
        let (a, repo) = acct(Some(d("1")), "500");
        repo.fail_read.store(true, Ordering::SeqCst);
        for reason in [ResetReason::Initial, ResetReason::OwnerRequested] {
            assert_eq!(
                a.reset(&req(reason), &Allow).await,
                Err(AccountError::Ledger(RepoError::Unavailable))
            );
        }
        assert!(repo.epochs.lock().unwrap().is_empty());
        assert!(repo.audits.lock().unwrap().is_empty());
    }

    /// The write lock makes the second reset see the first one's epoch; without it both would
    /// read an empty ledger and the loser would only fail at the store.
    #[tokio::test]
    async fn concurrent_initial_resets_on_one_account_are_serialised() {
        let (a, repo) = acct(Some(d("1")), "500");
        let r = req(ResetReason::Initial);
        let (first, second) = tokio::join!(a.reset(&r, &RefuseAll), a.reset(&r, &RefuseAll));
        assert_eq!(first, Ok(1));
        assert_eq!(
            second,
            Err(AccountError::ResetRefused(ResetRule::InitialOnOpenedLedger))
        );
        assert_eq!(repo.epochs.lock().unwrap().len(), 1);
        assert_eq!(repo.audits.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn failed_audit_means_no_new_epoch_and_bad_actor_is_refused() {
        let (a, repo) = acct(Some(d("1")), "500");
        open(&a).await;
        repo.fail_audit.store(true, Ordering::SeqCst);
        assert_eq!(
            a.reset(&req(ResetReason::OwnerRequested), &Allow).await,
            Err(AccountError::Ledger(RepoError::Conflict))
        );
        assert_eq!(a.balances().await.unwrap().epoch, Some(1));
        assert_eq!(repo.audits.lock().unwrap().len(), 1);
        for bad in ["", "a\"b", &"x".repeat(65)] {
            let r = ResetRequest {
                reason: ResetReason::OwnerRequested,
                actor: bad.into(),
            };
            assert_eq!(a.reset(&r, &Allow).await, Err(AccountError::BadActor));
        }
    }

    #[tokio::test]
    async fn failed_append_many_leaves_ledger_unchanged_and_returns_error() {
        let (a, repo) = acct(Some(d("5")), "1000");
        open(&a).await;
        repo.fail_next.store(true, Ordering::SeqCst);
        assert_eq!(
            a.apply_fill(&fill(true, 1, "100")).await,
            Err(AccountError::Ledger(RepoError::Unavailable))
        );
        let b = a.balances().await.unwrap();
        assert_eq!(
            (b.cash, b.charges_paid, b.entries),
            (d("1000"), Decimal::ZERO, 1)
        );
        // The next fill books normally.
        assert_eq!(
            a.apply_fill(&fill(true, 1, "100")).await.unwrap().cash,
            d("895")
        );
    }

    #[test]
    fn replay_overflow_and_unknown_kind_are_errors_not_panics() {
        let e = |kind: &str, amount| LedgerEntry {
            epoch: 1,
            kind: kind.into(),
            amount,
            ref_id: None,
            ts_ist_rfc3339: String::new(),
        };
        assert_eq!(
            replay(&[e("opening", Decimal::MAX), e("opening", Decimal::MAX)]),
            Err(AccountError::InvalidFill)
        );
        assert_eq!(
            replay(&[e("chrage", d("-1"))]),
            Err(AccountError::Ledger(RepoError::Corrupt))
        );
        assert_eq!(LedgerKind::parse("sell"), Some(LedgerKind::Sell));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_fills_cannot_overdraw_one_account() {
        let (a, _) = acct(Some(d("10")), "1000");
        open(&a).await;
        let a = Arc::new(a);
        // Each affordable alone (610 of 1000), not together.
        let tasks: Vec<_> = (0..8)
            .map(|_| {
                let a = a.clone();
                tokio::spawn(async move { a.apply_fill(&fill(true, 6, "100")).await })
            })
            .collect();
        let mut ok = 0;
        for t in tasks {
            match t.await.unwrap() {
                Ok(_) => ok += 1,
                Err(e) => assert_eq!(e, AccountError::InsufficientFunds),
            }
        }
        assert_eq!(ok, 1);
        assert_eq!(a.balances().await.unwrap().cash, d("390"));
    }

    proptest! {
        /// Random fills, resets and write failures: cash never negative, equals the sum of the
        /// current epoch's lines, refused or failed ops change nothing, and a fresh account over
        /// the same ledger replays identical balances. `held` models the position book.
        #[test]
        fn replay_reproduces_balances(
            per_unit in 0u32..3,
            ops in proptest::collection::vec((0u8..12, any::<bool>(), 1u32..50, 1u32..500), 0..60),
        ) {
            let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
            rt.block_on(async {
                let (a, repo) = acct_with(Some(d("5")), Decimal::from(per_unit), "20000");
                open(&a).await;
                let mut held = 0u32;
                for (op, buy, qty, price) in ops {
                    let before = a.balances().await.unwrap();
                    if op == 0 {
                        a.reset(&req(ResetReason::OwnerRequested), &Allow).await.unwrap();
                        continue;
                    }
                    let fail = op == 1;
                    repo.fail_next.store(fail, Ordering::SeqCst);
                    let mut f = fill(buy, qty, &price.to_string());
                    f.held_qty = held;
                    match a.apply_fill(&f).await {
                        Ok(after) => {
                            prop_assert!(!fail);
                            prop_assert!(after.cash >= Decimal::ZERO);
                            held = if buy { held + qty } else { held - qty };
                        }
                        Err(AccountError::InsufficientFunds | AccountError::NoPosition) => {
                            repo.fail_next.store(false, Ordering::SeqCst);
                            prop_assert_eq!(a.balances().await.unwrap(), before);
                        }
                        Err(AccountError::Ledger(_)) => {
                            prop_assert!(fail);
                            prop_assert_eq!(a.balances().await.unwrap(), before);
                        }
                        Err(e) => prop_assert!(false, "unexpected {e}"),
                    }
                    repo.fail_next.store(false, Ordering::SeqCst);
                }
                let live = a.balances().await.unwrap();
                let n = repo.epochs.lock().unwrap().len();
                let cur = repo.lines(n - 1);
                let sum: Decimal = cur.iter().map(|e| e.amount).sum();
                prop_assert_eq!(live.cash, sum);
                prop_assert!(live.cash >= Decimal::ZERO);
                let ch = FakeCharges { base: None, per_unit: Decimal::ZERO };
                let again = PaperAccount::new(Shared(repo.clone()), ch, clock(), d("1"));
                prop_assert_eq!(again.balances().await.unwrap(), live);
                Ok(())
            })?;
        }
    }
}
