//! `PaperExecutor`: the paper `Executor`. `place` validates like a broker (lot, tick, freeze,
//! position, funds) and puts the order in the working book; it never fills. Fills come only from
//! `on_tick` / `on_clock`, driven by the same ticks the UI sees, and are booked in the paper
//! ledger with charges. A refused order leaves the ledger untouched.
//!
//! Limits (recorded in the F10 record): MIS and NRML use full cash (no margin, no leverage) and
//! intraday short selling is refused (`InvalidInput`), because no margin source exists yet; the
//! funds check at `place` is an estimate (reference = limit price or last tick) and the real
//! check repeats at every fill, where a failure drops the order. AMO is not modelled: outside
//! the normal session nothing fills, and a DAY order expires on the next IST date.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use bazaar_core::clock::Clock;
use bazaar_core::ports::{AuditDraft, AuditRepo, PaperLedgerRepo};
use bazaar_execution::audit::chain_hash;
use bazaar_execution::executor::{ApprovedOrder, ExecAck, ExecError, Executor};
use bazaar_execution::types::{IntentId, OrderIntent, OrderKind, Side};
use chrono::DateTime;
use chrono_tz::Tz;
use rust_decimal::Decimal;

use crate::account::{AccountError, ChargeSource, FillBooking, PaperAccount};
use crate::fill::{
    FillReason, FillSimulator, InstrumentSpec, SimConfig, SimFill, Tick, WorkingBook, WorkingOrder,
};
use crate::squareoff::{SquareOff, SquareOffRun};

/// Instrument master lookup (lot, tick, freeze, segment).
pub trait SpecSource: Send + Sync {
    fn spec(&self, symbol: &str) -> Option<InstrumentSpec>;
}

/// What one tick did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TickOutcome {
    pub fills: Vec<SimFill>,
    /// Orders dropped because the account refused or the audit write failed (reason attached).
    pub rejected: Vec<(IntentId, ExecError)>,
}

/// What one clock call did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClockOutcome {
    pub square_off: SquareOffRun,
    pub expired: Vec<IntentId>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// The paper executor.
pub struct PaperExecutor<R, C, A, S> {
    account: Arc<PaperAccount<R, C>>,
    audit: A,
    specs: S,
    clock: Arc<dyn Clock>,
    sim: Mutex<FillSimulator>,
    book: Mutex<WorkingBook>,
    square: Mutex<SquareOff>,
    /// Serialises tick and clock handling so fills book in order.
    driver: tokio::sync::Mutex<()>,
}

impl<R, C, A, S> std::fmt::Debug for PaperExecutor<R, C, A, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaperExecutor").finish_non_exhaustive()
    }
}

impl<R: PaperLedgerRepo, C: ChargeSource, A: AuditRepo, S: SpecSource> PaperExecutor<R, C, A, S> {
    pub fn new(
        account: Arc<PaperAccount<R, C>>,
        audit: A,
        specs: S,
        clock: Arc<dyn Clock>,
        cfg: SimConfig,
        square_off_at: chrono::NaiveTime,
    ) -> Self {
        Self {
            account,
            audit,
            specs,
            clock,
            sim: Mutex::new(FillSimulator::new(cfg)),
            book: Mutex::new(WorkingBook::default()),
            square: Mutex::new(SquareOff::new(square_off_at)),
            driver: tokio::sync::Mutex::new(()),
        }
    }

    /// Net position for a symbol and product label, as the simulated broker holds it.
    #[must_use]
    pub fn position(&self, symbol: &str, product: bazaar_execution::types::Product) -> i64 {
        lock(&self.book).position(symbol, product)
    }

    /// Number of orders still working.
    #[must_use]
    pub fn working_count(&self) -> usize {
        lock(&self.book).orders.len()
    }

    /// Validate and accept one order into the working book (no fill). `place` calls this; it is
    /// public so the paper crate can be driven in tests without an `ApprovedOrder`.
    ///
    /// # Errors
    /// `InvalidInput` (lot, tick, freeze, quantity, position, unknown instrument, duplicate),
    /// `MarginUnavailable` (no reference price or no charge estimate), `InsufficientFunds`,
    /// `Storage`.
    pub async fn accept(&self, i: &OrderIntent) -> Result<ExecAck, ExecError> {
        let spec = self
            .specs
            .spec(&i.symbol)
            .ok_or(ExecError::InvalidInput("unknown instrument"))?;
        let reference = self.validate(i, &spec)?;
        if i.side == Side::Buy {
            let b = FillBooking {
                ref_id: i.id.0,
                symbol: i.symbol.clone(),
                is_buy: true,
                qty: i.qty,
                price: reference,
                product: i.product.as_str().to_owned(),
                held_qty: 0,
            };
            self.account.check_funds(&b).await.map_err(map_account)?;
        }
        let now = self.clock.now_ist();
        let mut book = lock(&self.book);
        if book.orders.iter().any(|o| o.id == i.id) {
            return Err(ExecError::InvalidInput("duplicate order"));
        }
        book.specs.insert(i.symbol.clone(), spec.clone());
        book.orders.push(WorkingOrder {
            id: i.id,
            symbol: i.symbol.clone(),
            side: i.side,
            qty: i.qty,
            filled: 0,
            kind: i.kind,
            product: i.product,
            spec,
            accepted_at: now,
            stale_flagged: false,
            square_off: false,
        });
        Ok(ExecAck {
            intent_id: i.id,
            external_id: format!("paper-{}", i.id),
            filled_qty: 0,
            avg_price: None,
        })
    }

    /// Broker-style rejects; returns the reference price for the funds estimate.
    fn validate(&self, i: &OrderIntent, spec: &InstrumentSpec) -> Result<Decimal, ExecError> {
        if i.qty == 0 || spec.lot_size == 0 || !i.qty.is_multiple_of(spec.lot_size) {
            return Err(ExecError::InvalidInput("lot size"));
        }
        if spec.freeze_qty.is_some_and(|f| i.qty > f) {
            return Err(ExecError::InvalidInput("freeze quantity"));
        }
        let book = lock(&self.book);
        let reference = match i.kind {
            OrderKind::Limit(p) => {
                if p <= Decimal::ZERO
                    || spec.tick_size <= Decimal::ZERO
                    || p % spec.tick_size != Decimal::ZERO
                {
                    return Err(ExecError::InvalidInput("tick size"));
                }
                p
            }
            OrderKind::Market => book
                .last_price
                .get(&i.symbol)
                .copied()
                .ok_or(ExecError::MarginUnavailable)?,
        };
        if i.side == Side::Sell {
            let held = book.position(&i.symbol, i.product).max(0).unsigned_abs();
            if u64::from(i.qty) + book.working_sell_qty(&i.symbol) > held {
                return Err(ExecError::InvalidInput("no position to sell"));
            }
        }
        Ok(reference)
    }

    /// Feed one tick; books the fills it produces.
    pub async fn on_tick(&self, tick: &Tick) -> TickOutcome {
        let _d = self.driver.lock().await;
        let now = self.clock.now_ist();
        let fills = {
            let mut book = lock(&self.book);
            lock(&self.sim).on_tick(tick, now, &mut book)
        };
        let mut out = TickOutcome::default();
        for f in fills {
            match self.book_fill(&f, now).await {
                Ok(()) => out.fills.push(f),
                Err(e) => out.rejected.push((f.id, e)),
            }
        }
        out
    }

    /// Feed the clock: MIS square-off and DAY expiry.
    pub async fn on_clock(&self) -> ClockOutcome {
        let _d = self.driver.lock().await;
        let now = self.clock.now_ist();
        let mut book = lock(&self.book);
        let expired = lock(&self.sim).on_clock(now, &mut book);
        let square_off = lock(&self.square).on_clock(now, &mut book);
        ClockOutcome {
            square_off,
            expired,
        }
    }

    /// Audit (square-off only, write-ahead), then ledger, then position. On failure the order is
    /// dropped (account refused) or the fill is undone (audit failed; the order keeps working).
    async fn book_fill(&self, f: &SimFill, now: DateTime<Tz>) -> Result<(), ExecError> {
        if f.reason == FillReason::SquareOff {
            let draft = AuditDraft {
                ts_ist_rfc3339: now.to_rfc3339(),
                actor: "paper-broker".to_owned(),
                kind: "auto_square_off".to_owned(),
                payload: format!(
                    "{{\"id\":\"{}\",\"price\":\"{}\",\"qty\":{},\"side\":\"{}\",\"symbol\":\"{}\"}}",
                    f.id,
                    f.price,
                    f.qty,
                    if f.side == Side::Buy { "buy" } else { "sell" },
                    esc(&f.symbol)
                ),
            };
            if self.audit.append(draft, chain_hash).await.is_err() {
                if let Some(o) = lock(&self.book).orders.iter_mut().find(|o| o.id == f.id) {
                    o.filled -= f.qty;
                }
                return Err(ExecError::Storage);
            }
        }
        let held = lock(&self.book).position(&f.symbol, f.product).max(0);
        let booking = FillBooking {
            ref_id: f.id.0,
            symbol: f.symbol.clone(),
            is_buy: f.side == Side::Buy,
            qty: f.qty,
            price: f.price,
            product: f.product.as_str().to_owned(),
            held_qty: u32::try_from(held).unwrap_or(u32::MAX),
        };
        match self.account.apply_fill(&booking).await {
            Ok(_) => {
                lock(&self.book).record(f);
                Ok(())
            }
            Err(e) => {
                lock(&self.book).drop_order(f.id);
                Err(map_account(e))
            }
        }
    }
}

const fn map_account(e: AccountError) -> ExecError {
    match e {
        AccountError::InsufficientFunds | AccountError::NotOpened => ExecError::InsufficientFunds,
        AccountError::NoPosition => ExecError::InvalidInput("no position to sell"),
        AccountError::InvalidFill => ExecError::InvalidInput("invalid fill"),
        AccountError::ChargesUnavailable | AccountError::BadCharge => ExecError::MarginUnavailable,
        AccountError::Ledger(_) => ExecError::Storage,
    }
}

impl<R: PaperLedgerRepo, C: ChargeSource, A: AuditRepo, S: SpecSource> Executor
    for PaperExecutor<R, C, A, S>
{
    async fn place(&self, order: &ApprovedOrder) -> Result<ExecAck, ExecError> {
        self.accept(order.intent()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fill::tests::{at, cfg, d, tick};
    use bazaar_core::ports::{AuditRow, LedgerEntry, RepoError, SourceError};
    use bazaar_core::segment::{InstrumentKind, Segment};
    use bazaar_core::testutil::FakeClock;
    use bazaar_execution::audit::verify_chain;
    use bazaar_execution::types::Product;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[derive(Default, Clone)]
    struct Ledger(Arc<Mutex<Vec<Vec<LedgerEntry>>>>);
    impl PaperLedgerRepo for Ledger {
        async fn append(&self, e: LedgerEntry) -> Result<(), RepoError> {
            self.append_many(vec![e]).await
        }
        async fn append_many(&self, es: Vec<LedgerEntry>) -> Result<(), RepoError> {
            lock(&self.0)
                .last_mut()
                .ok_or(RepoError::Conflict)?
                .extend(es);
            Ok(())
        }
        async fn current_epoch(&self) -> Result<Vec<LedgerEntry>, RepoError> {
            Ok(lock(&self.0).last().cloned().unwrap_or_default())
        }
        async fn new_epoch(&self, o: LedgerEntry) -> Result<u32, RepoError> {
            let mut g = lock(&self.0);
            g.push(vec![o]);
            Ok(u32::try_from(g.len()).unwrap())
        }
    }

    struct Flat;
    impl ChargeSource for Flat {
        fn charge(
            &self,
            _: &str,
            _: bool,
            _: u32,
            _: Decimal,
            _: &str,
        ) -> Result<Decimal, SourceError> {
            Ok(d("1"))
        }
    }

    #[derive(Default, Clone)]
    struct Audit {
        rows: Arc<Mutex<Vec<AuditRow>>>,
        fail: Arc<AtomicBool>,
    }
    impl AuditRepo for Audit {
        async fn append(
            &self,
            draft: AuditDraft,
            chain: bazaar_core::ports::ChainFn,
        ) -> Result<AuditRow, RepoError> {
            if self.fail.load(Ordering::SeqCst) {
                return Err(RepoError::Unavailable);
            }
            let mut g = lock(&self.rows);
            let prev = g.last().map_or([0; 32], |r| r.hash);
            let seq = g.len() as u64 + 1;
            let row = AuditRow {
                seq,
                prev_hash: prev,
                hash: chain(&prev, seq, &draft),
                draft,
            };
            g.push(row.clone());
            Ok(row)
        }
        async fn read_all(&self) -> Result<Vec<AuditRow>, RepoError> {
            Ok(lock(&self.rows).clone())
        }
    }

    struct Specs;
    impl SpecSource for Specs {
        fn spec(&self, s: &str) -> Option<InstrumentSpec> {
            (s == "ABC").then(|| InstrumentSpec {
                segment: Segment::Equity,
                kind: InstrumentKind::CashNonFno,
                lot_size: 5,
                tick_size: d("0.05"),
                freeze_qty: Some(100),
            })
        }
    }

    type Ex = PaperExecutor<Ledger, Flat, Audit, Specs>;

    struct Rig {
        ex: Ex,
        ledger: Ledger,
        audit: Audit,
        clock: Arc<FakeClock>,
    }

    async fn rig(cash: &str) -> Rig {
        let ledger = Ledger::default();
        let audit = Audit::default();
        let clock = Arc::new(FakeClock::at(at(10, 0, 0)));
        let acct = Arc::new(PaperAccount::new(
            ledger.clone(),
            Flat,
            clock.clone(),
            d(cash),
        ));
        acct.reset().await.unwrap();
        let ex = PaperExecutor::new(
            acct,
            audit.clone(),
            Specs,
            clock.clone(),
            cfg(),
            chrono::NaiveTime::from_hms_opt(13, 30, 0).unwrap(),
        );
        Rig {
            ex,
            ledger,
            audit,
            clock,
        }
    }

    fn intent(id: u128, side: Side, qty: u32, kind: OrderKind, product: Product) -> OrderIntent {
        OrderIntent {
            id: IntentId(id),
            symbol: "ABC".into(),
            side,
            qty,
            kind,
            product,
            reduce_only: false,
            created_at: at(10, 0, 0),
        }
    }

    fn cnt(r: &Rig) -> usize {
        lock(&r.ledger.0).last().unwrap().len()
    }

    async fn tick_at(r: &Rig, price: &str, vol: u64, s: u32) -> TickOutcome {
        let t = tick(price, vol, s);
        r.clock.set(t.at);
        r.ex.on_tick(&t).await
    }

    #[tokio::test]
    async fn place_never_fills_and_a_later_tick_does_with_charge_and_cash() {
        let r = rig("100000").await;
        tick_at(&r, "100", 1000, 0).await; // reference price only
        let ack =
            r.ex.accept(&intent(1, Side::Buy, 10, OrderKind::Market, Product::Cnc))
                .await
                .unwrap();
        assert_eq!((ack.filled_qty, r.ex.working_count()), (0, 1));
        assert_eq!(cnt(&r), 1, "only the opening line");
        let o = tick_at(&r, "100", 1000, 1).await;
        assert_eq!(o.fills.len(), 1);
        assert_eq!(r.ex.position("ABC", Product::Cnc), 10);
        // 10 x 100.10 (100 * 1.001 aligned up) + charge 1
        let bal = r.ex.account.balances().await.unwrap();
        assert_eq!(bal.cash, d("100000") - d("1001.00") - d("1"));
        assert_eq!(r.ex.working_count(), 0);
    }

    #[tokio::test]
    async fn broker_style_rejects_leave_the_ledger_untouched() {
        let r = rig("100000").await;
        tick_at(&r, "100", 1000, 0).await;
        let m = |q| intent(1, Side::Buy, q, OrderKind::Market, Product::Cnc);
        assert_eq!(
            r.ex.accept(&m(7)).await,
            Err(ExecError::InvalidInput("lot size"))
        );
        assert_eq!(
            r.ex.accept(&m(0)).await,
            Err(ExecError::InvalidInput("lot size"))
        );
        assert_eq!(
            r.ex.accept(&m(105)).await,
            Err(ExecError::InvalidInput("freeze quantity"))
        );
        let l = intent(1, Side::Buy, 5, OrderKind::Limit(d("99.99")), Product::Cnc);
        assert_eq!(
            r.ex.accept(&l).await,
            Err(ExecError::InvalidInput("tick size"))
        );
        let mut other = m(5);
        other.symbol = "ZZZ".into();
        assert_eq!(
            r.ex.accept(&other).await,
            Err(ExecError::InvalidInput("unknown instrument"))
        );
        let sell = intent(2, Side::Sell, 5, OrderKind::Market, Product::Cnc);
        assert_eq!(
            r.ex.accept(&sell).await,
            Err(ExecError::InvalidInput("no position to sell"))
        );
        assert_eq!((cnt(&r), r.ex.working_count()), (1, 0));
    }

    #[tokio::test]
    async fn market_without_a_reference_price_is_margin_unavailable() {
        let r = rig("100000").await;
        let e =
            r.ex.accept(&intent(1, Side::Buy, 5, OrderKind::Market, Product::Cnc))
                .await;
        assert_eq!(e, Err(ExecError::MarginUnavailable));
    }

    #[tokio::test]
    async fn insufficient_funds_blocks_and_duplicate_is_refused() {
        let r = rig("100").await;
        let big = intent(1, Side::Buy, 5, OrderKind::Limit(d("100")), Product::Cnc);
        assert_eq!(r.ex.accept(&big).await, Err(ExecError::InsufficientFunds));
        let ok = intent(2, Side::Buy, 5, OrderKind::Limit(d("10")), Product::Cnc);
        r.ex.accept(&ok).await.unwrap();
        assert_eq!(
            r.ex.accept(&ok).await,
            Err(ExecError::InvalidInput("duplicate order"))
        );
    }

    #[tokio::test]
    async fn fill_time_funds_failure_drops_the_order_without_ledger_change() {
        let r = rig("600").await;
        let a = intent(1, Side::Buy, 5, OrderKind::Limit(d("100")), Product::Cnc);
        let b = intent(2, Side::Buy, 5, OrderKind::Limit(d("100")), Product::Cnc);
        r.ex.accept(&a).await.unwrap();
        r.ex.accept(&b).await.unwrap();
        let o = tick_at(&r, "99.95", 1000, 1).await;
        // each fill is 5 x 100 + 1: the first leaves 99, the second is refused and dropped
        assert_eq!((o.fills.len(), o.rejected.len()), (1, 1));
        assert_eq!(o.rejected[0].1, ExecError::InsufficientFunds);
        assert_eq!(r.ex.account.balances().await.unwrap().cash, d("99"));
        assert_eq!(r.ex.working_count(), 0);
    }

    #[tokio::test]
    async fn partial_fills_accumulate_across_ticks() {
        let r = rig("100000").await;
        let l = intent(1, Side::Buy, 50, OrderKind::Limit(d("100")), Product::Cnc);
        r.ex.accept(&l).await.unwrap();
        let mut total = 0;
        for s in 1..40 {
            total += tick_at(&r, "99.90", 100, s)
                .await
                .fills
                .iter()
                .map(|f| f.qty)
                .sum::<u32>();
        }
        assert_eq!(total, 50);
        assert_eq!(r.ex.position("ABC", Product::Cnc), 50);
    }

    #[tokio::test]
    async fn sell_cannot_exceed_position_net_of_working_sells() {
        let r = rig("100000").await;
        let b = intent(1, Side::Buy, 10, OrderKind::Limit(d("100")), Product::Cnc);
        r.ex.accept(&b).await.unwrap();
        tick_at(&r, "99.90", 10_000, 1).await;
        let s1 = intent(2, Side::Sell, 10, OrderKind::Limit(d("120")), Product::Cnc);
        r.ex.accept(&s1).await.unwrap();
        let s2 = intent(3, Side::Sell, 5, OrderKind::Limit(d("120")), Product::Cnc);
        assert_eq!(
            r.ex.accept(&s2).await,
            Err(ExecError::InvalidInput("no position to sell"))
        );
    }

    #[tokio::test]
    async fn mis_square_off_closes_position_audits_and_chain_verifies() {
        let r = rig("100000").await;
        tick_at(&r, "100", 1000, 0).await;
        r.ex.accept(&intent(1, Side::Buy, 10, OrderKind::Market, Product::Mis))
            .await
            .unwrap();
        tick_at(&r, "100", 10_000, 1).await;
        assert_eq!(r.ex.position("ABC", Product::Mis), 10);
        r.clock.set(at(13, 30, 0));
        let c = r.ex.on_clock().await;
        assert_eq!(c.square_off.queued.len(), 1);
        let mut t = tick("100", 10_000, 0);
        t.at = at(13, 30, 1);
        r.clock.set(t.at);
        let o = r.ex.on_tick(&t).await;
        assert_eq!(o.fills[0].reason, FillReason::SquareOff);
        assert_eq!(r.ex.position("ABC", Product::Mis), 0);
        let rows = lock(&r.audit.rows).clone();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].draft.kind, "auto_square_off");
        verify_chain(&rows).unwrap();
    }

    #[tokio::test]
    async fn audit_failure_blocks_the_square_off_fill_and_it_retries() {
        let r = rig("100000").await;
        tick_at(&r, "100", 1000, 0).await;
        r.ex.accept(&intent(1, Side::Buy, 10, OrderKind::Market, Product::Mis))
            .await
            .unwrap();
        tick_at(&r, "100", 10_000, 1).await;
        r.clock.set(at(13, 30, 0));
        r.ex.on_clock().await;
        r.audit.fail.store(true, Ordering::SeqCst);
        let before = cnt(&r);
        let mut t = tick("100", 10_000, 0);
        t.at = at(13, 30, 1);
        r.clock.set(t.at);
        let o = r.ex.on_tick(&t).await;
        assert_eq!(
            (o.fills.len(), o.rejected[0].1.clone()),
            (0, ExecError::Storage)
        );
        assert_eq!((cnt(&r), r.ex.position("ABC", Product::Mis)), (before, 10));
        r.audit.fail.store(false, Ordering::SeqCst);
        t.at = at(13, 30, 2);
        r.clock.set(t.at);
        assert_eq!(r.ex.on_tick(&t).await.fills.len(), 1);
        assert_eq!(r.ex.position("ABC", Product::Mis), 0);
    }

    #[tokio::test]
    async fn stale_tick_does_not_fill() {
        let r = rig("100000").await;
        tick_at(&r, "100", 1000, 0).await;
        r.ex.accept(&intent(1, Side::Buy, 5, OrderKind::Market, Product::Cnc))
            .await
            .unwrap();
        let t = tick("100", 1000, 1);
        r.clock.set(t.at + chrono::Duration::seconds(10));
        assert_eq!(r.ex.on_tick(&t).await.fills.len(), 0);
        assert_eq!(r.ex.working_count(), 1);
    }

    #[tokio::test]
    async fn day_order_expires_on_the_next_date() {
        let r = rig("100000").await;
        r.ex.accept(&intent(
            1,
            Side::Buy,
            5,
            OrderKind::Limit(d("50")),
            Product::Cnc,
        ))
        .await
        .unwrap();
        r.clock.set(at(10, 0, 0) + chrono::Duration::days(1));
        assert_eq!(r.ex.on_clock().await.expired, vec![IntentId(1)]);
        assert_eq!(r.ex.working_count(), 0);
    }

    #[test]
    fn account_errors_map_to_exec_errors() {
        assert_eq!(
            map_account(AccountError::NoPosition),
            ExecError::InvalidInput("no position to sell")
        );
        assert_eq!(
            map_account(AccountError::BadCharge),
            ExecError::MarginUnavailable
        );
        assert_eq!(
            map_account(AccountError::Ledger(RepoError::Unavailable)),
            ExecError::Storage
        );
        assert_eq!(esc("a\"b"), "a\\\"b");
    }
}
