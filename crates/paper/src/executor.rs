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

use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use bazaar_core::calendar::MarketCalendar;
use bazaar_core::clock::Clock;
use bazaar_core::ports::{AuditDraft, AuditRepo, PaperLedgerRepo};
use bazaar_execution::audit::chain_hash;
use bazaar_execution::executor::{ApprovedOrder, ExecAck, ExecError, Executor};
use bazaar_execution::types::{IntentId, OrderIntent, OrderKind, Product, Side};
use chrono::DateTime;
use chrono_tz::Tz;
use rust_decimal::Decimal;

use crate::account::{AccountError, ChargeSource, FillBooking, PaperAccount};
use crate::fill::{
    FillReason, FillSimulator, InstrumentSpec, SimConfig, SimFill, Tick, WorkingBook, WorkingOrder,
};
use crate::squareoff::{SquareOff, SquareOffRun, ValidatedSquareOffTime};

/// Instrument master lookup (lot, tick, freeze, segment).
pub trait SpecSource: Send + Sync {
    fn spec(&self, symbol: &str) -> Option<InstrumentSpec>;
}

/// What one tick did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TickOutcome {
    pub fills: Vec<SimFill>,
    /// User orders dropped because the account refused (reason attached).
    pub rejected: Vec<(IntentId, ExecError)>,
    /// Square-off fills that could not be booked (audit write failed, or the ledger or charge
    /// source refused). Fail closed: the order stays working and retries on the next tick, so a
    /// position stays open until this clears. The caller must alarm on any entry here; each one
    /// is also logged at error level on every tick.
    pub blocked_square_offs: Vec<(IntentId, ExecError)>,
}

/// What one clock call did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClockOutcome {
    pub square_off: SquareOffRun,
    pub expired: Vec<IntentId>,
    /// Cancelled or expired orders whose audit row could not be written.
    pub audit_failed: Vec<IntentId>,
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
    cal: Arc<MarketCalendar>,
    sim: Mutex<FillSimulator>,
    book: Mutex<WorkingBook>,
    square: Mutex<SquareOff>,
    /// Orders whose failure was already audited (one row per failure, not one per retry).
    failed_audited: Mutex<HashSet<IntentId>>,
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
        cal: Arc<MarketCalendar>,
        cfg: SimConfig,
        square_off_at: ValidatedSquareOffTime,
    ) -> Self {
        Self {
            account,
            audit,
            specs,
            clock,
            cal,
            sim: Mutex::new(FillSimulator::new(cfg)),
            book: Mutex::new(WorkingBook::default()),
            square: Mutex::new(SquareOff::new(square_off_at)),
            failed_audited: Mutex::new(HashSet::new()),
            driver: tokio::sync::Mutex::new(()),
        }
    }

    /// Net position for a symbol and product label, as the simulated broker holds it.
    #[must_use]
    pub fn position(&self, symbol: &str, product: Product) -> i64 {
        lock(&self.book).position(symbol, product)
    }

    /// True when any non-flat position is held (any product); used to refuse a P&L figure that
    /// would need a mark price.
    #[must_use]
    pub fn has_open_positions(&self) -> bool {
        let b = lock(&self.book);
        [Product::Mis, Product::Cnc, Product::Nrml]
            .into_iter()
            .any(|p| !b.open_positions(p).is_empty())
    }

    /// Number of orders still working.
    #[must_use]
    pub fn working_count(&self) -> usize {
        lock(&self.book).orders.len()
    }

    /// Validate and accept one order into the working book (no fill). Crate-private on purpose:
    /// the only public entry is `Executor::place(&ApprovedOrder)`, so nothing outside this crate
    /// can put an order on the paper book without the gate and the approval.
    ///
    /// # Errors
    /// `InvalidInput` (lot, tick, freeze, quantity, position, unknown instrument, duplicate),
    /// `MarginUnavailable` (no reference price or no charge estimate), `InsufficientFunds`,
    /// `Storage`.
    pub(crate) async fn accept(&self, i: &OrderIntent) -> Result<ExecAck, ExecError> {
        // One driver at a time: validate, funds check and push are atomic against other `place`
        // calls and against ticks, so two orders cannot both pass the position or reserve check.
        let _d = self.driver.lock().await;
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
            // Funds already promised to other working buys are not available (a broker blocks
            // them at placement); released automatically when those orders fill or end.
            let reserved = self.reserved_for_working_buys();
            let need = reserved.checked_add(
                reference
                    .checked_mul(Decimal::from(i.qty))
                    .ok_or(ExecError::MarginUnavailable)?,
            );
            let cash = self.account.balances().await.map_err(map_account)?.cash;
            if need.is_none_or(|n| n > cash) {
                return Err(ExecError::InsufficientFunds);
            }
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

    /// Notional of the unfilled part of every working buy (limit price, else the last tick).
    fn reserved_for_working_buys(&self) -> Decimal {
        let book = lock(&self.book);
        book.orders
            .iter()
            .filter(|o| o.side == Side::Buy)
            .filter_map(|o| {
                let px = match o.kind {
                    OrderKind::Limit(p) => p,
                    // A working market buy always has a last price (its spec and a reference
                    // price were required at accept), so this skip is unreachable in practice.
                    OrderKind::Market => *book.last_price.get(&o.symbol)?,
                };
                px.checked_mul(Decimal::from(o.qty - o.filled))
            })
            .fold(Decimal::ZERO, |a, n| {
                a.checked_add(n).unwrap_or(Decimal::MAX)
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

    /// Feed one tick; books the fills it produces. Whether the session is open is decided here
    /// from the calendar (a calendar error is closed), never by the caller.
    pub async fn on_tick(&self, tick: &Tick) -> TickOutcome {
        let _d = self.driver.lock().await;
        let now = self.clock.now_ist();
        let fills = {
            let mut book = lock(&self.book);
            lock(&self.sim).on_tick(tick, now, &self.cal, &mut book)
        };
        let mut out = TickOutcome::default();
        for f in fills {
            let square_off = f.reason == FillReason::SquareOff;
            match self.book_fill(&f, now).await {
                Ok(()) => out.fills.push(f),
                Err(e) if square_off => {
                    tracing::error!(
                        id = %f.id, symbol = %f.symbol, error = ?e,
                        "MIS square-off blocked; position stays open and the fill retries"
                    );
                    out.blocked_square_offs.push((f.id, e));
                }
                Err(e) => out.rejected.push((f.id, e)),
            }
        }
        out
    }

    /// Feed the clock: DAY expiry and MIS square-off. Cancelled and expired orders are audited
    /// (after the book change: they move no money); square-off fills are audited write-ahead.
    pub async fn on_clock(&self) -> ClockOutcome {
        let _d = self.driver.lock().await;
        let now = self.clock.now_ist();
        let (expired, square_off) = {
            let mut book = lock(&self.book);
            let expired = lock(&self.sim).on_clock(now, &mut book);
            (expired, lock(&self.square).on_clock(now, &mut book))
        };
        for s in &square_off.failed {
            tracing::error!(symbol = %s, "MIS square-off could not be queued; retrying on the next clock call");
        }
        let mut audit_failed = Vec::new();
        let events = square_off
            .cancelled
            .iter()
            .map(|id| (*id, "paper_order_cancelled", "mis_square_off"))
            .chain(
                expired
                    .iter()
                    .map(|id| (*id, "paper_order_expired", "day_validity")),
            );
        for (id, kind, reason) in events {
            let payload = format!("{{\"id\":\"{id}\",\"reason\":\"{reason}\"}}");
            if self.audit_event(kind, payload, now).await.is_err() {
                tracing::error!(id = %id, kind, "audit write failed for a paper order change");
                audit_failed.push(id);
            }
        }
        ClockOutcome {
            square_off,
            expired,
            audit_failed,
        }
    }

    async fn audit_event(
        &self,
        kind: &str,
        payload: String,
        now: DateTime<Tz>,
    ) -> Result<(), ExecError> {
        let draft = AuditDraft {
            ts_ist_rfc3339: now.to_rfc3339(),
            actor: "paper-broker".to_owned(),
            kind: kind.to_owned(),
            payload,
        };
        self.audit
            .append(draft, chain_hash)
            .await
            .map(|_| ())
            .map_err(|_| ExecError::Storage)
    }

    /// Undo the simulator's fill count so the order keeps working and retries.
    fn undo_fill(&self, f: &SimFill) {
        if let Some(o) = lock(&self.book).orders.iter_mut().find(|o| o.id == f.id) {
            o.filled = o.filled.saturating_sub(f.qty);
        }
    }

    /// Audit (square-off only, write-ahead), then ledger, then position. A square-off is never
    /// dropped: on any failure the fill is undone and retried. A user order the account refuses
    /// is dropped. The first failure per order is audited (`auto_square_off_failed` /
    /// `paper_order_dropped`). Ordinary fills are audited by the pipeline, not here.
    async fn book_fill(&self, f: &SimFill, now: DateTime<Tz>) -> Result<(), ExecError> {
        let square_off = f.reason == FillReason::SquareOff;
        if square_off {
            let payload = format!(
                "{{\"id\":\"{}\",\"price\":\"{}\",\"qty\":{},\"side\":\"{}\",\"symbol\":\"{}\"}}",
                f.id,
                f.price,
                f.qty,
                if f.side == Side::Buy { "buy" } else { "sell" },
                esc(&f.symbol)
            );
            if let Err(e) = self.audit_event("auto_square_off", payload, now).await {
                self.undo_fill(f);
                return Err(e);
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
                lock(&self.failed_audited).remove(&f.id);
                lock(&self.book).record(f);
                Ok(())
            }
            Err(e) => {
                if square_off {
                    self.undo_fill(f);
                } else {
                    lock(&self.book).drop_order(f.id);
                }
                let first = lock(&self.failed_audited).insert(f.id);
                if first {
                    let kind = if square_off {
                        "auto_square_off_failed"
                    } else {
                        "paper_order_dropped"
                    };
                    let payload = format!(
                        "{{\"id\":\"{}\",\"reason\":\"{}\",\"symbol\":\"{}\"}}",
                        f.id,
                        reason_str(e),
                        esc(&f.symbol)
                    );
                    if self.audit_event(kind, payload, now).await.is_err() {
                        lock(&self.failed_audited).remove(&f.id);
                    }
                }
                Err(map_account(e))
            }
        }
    }
}

/// Stable audit reason per account error (never the wrapped repo error text).
const fn reason_str(e: AccountError) -> &'static str {
    match e {
        AccountError::InsufficientFunds => "insufficient_funds",
        AccountError::NoPosition => "no_position",
        AccountError::InvalidFill => "invalid_fill",
        AccountError::ChargesUnavailable => "charges_unavailable",
        AccountError::BadCharge => "bad_charge",
        AccountError::NotOpened => "not_opened",
        AccountError::Ledger(_) => "ledger_error",
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
    use crate::fill::tests::{at, cal, cfg, d, tick};
    use bazaar_core::ports::{AuditRow, LedgerEntry, RepoError, SourceError};
    use bazaar_core::segment::{InstrumentKind, Segment};
    use bazaar_core::testutil::FakeClock;
    use bazaar_execution::audit::verify_chain;
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

    #[derive(Default, Clone)]
    struct Flat(Arc<AtomicBool>);
    impl ChargeSource for Flat {
        fn charge(
            &self,
            _: &str,
            _: bool,
            _: u32,
            _: Decimal,
            _: &str,
        ) -> Result<Decimal, SourceError> {
            if self.0.load(Ordering::SeqCst) {
                return Err(SourceError::Unavailable);
            }
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
        charge_fail: Arc<AtomicBool>,
    }

    async fn rig(cash: &str) -> Rig {
        let ledger = Ledger::default();
        let audit = Audit::default();
        let clock = Arc::new(FakeClock::at(at(10, 0, 0)));
        let flat = Flat::default();
        let charge_fail = flat.0.clone();
        let acct = Arc::new(PaperAccount::new(
            ledger.clone(),
            flat,
            clock.clone(),
            d(cash),
        ));
        acct.reset().await.unwrap();
        let ex = PaperExecutor::new(
            acct,
            audit.clone(),
            Specs,
            clock.clone(),
            Arc::new(cal()),
            cfg(),
            ValidatedSquareOffTime::new(
                &cal(),
                at(10, 0, 0).date_naive(),
                chrono::NaiveTime::from_hms_opt(13, 30, 0).unwrap(),
                &[(Segment::Equity, InstrumentKind::CashNonFno)],
            )
            .unwrap(),
        );
        Rig {
            ex,
            ledger,
            audit,
            clock,
            charge_fail,
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
        assert!(r.ex.has_open_positions());
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
    async fn working_buys_reserve_funds_at_placement() {
        let r = rig("600").await;
        let a = intent(1, Side::Buy, 5, OrderKind::Limit(d("100")), Product::Cnc);
        let b = intent(2, Side::Buy, 5, OrderKind::Limit(d("100")), Product::Cnc);
        r.ex.accept(&a).await.unwrap();
        assert_eq!(r.ex.accept(&b).await, Err(ExecError::InsufficientFunds));
        assert_eq!(r.ex.working_count(), 1);
        // the reservation is released once the first order fills or ends
        tick_at(&r, "99.95", 1000, 1).await;
        let c = intent(3, Side::Buy, 5, OrderKind::Limit(d("10")), Product::Cnc);
        r.ex.accept(&c).await.unwrap();
    }

    #[tokio::test]
    async fn concurrent_placements_cannot_both_reserve_the_same_funds() {
        let rig = rig("600").await;
        let first = intent(1, Side::Buy, 5, OrderKind::Limit(d("100")), Product::Cnc);
        let second = intent(2, Side::Buy, 5, OrderKind::Limit(d("100")), Product::Cnc);
        let (one, two) = tokio::join!(rig.ex.accept(&first), rig.ex.accept(&second));
        assert_eq!(usize::from(one.is_ok()) + usize::from(two.is_ok()), 1);
        assert_eq!(rig.ex.working_count(), 1);
    }

    #[tokio::test]
    async fn fill_time_funds_failure_drops_the_order_audits_it_and_leaves_the_ledger() {
        let r = rig("501").await;
        tick_at(&r, "100", 1000, 0).await;
        // placement estimate 5 x 100 + 1 = 501 passes; the fill slips to 100.10 and is refused
        r.ex.accept(&intent(1, Side::Buy, 5, OrderKind::Market, Product::Cnc))
            .await
            .unwrap();
        let o = tick_at(&r, "100", 1000, 1).await;
        assert_eq!((o.fills.len(), o.rejected.len()), (0, 1));
        assert_eq!(o.rejected[0].1, ExecError::InsufficientFunds);
        assert_eq!(r.ex.account.balances().await.unwrap().cash, d("501"));
        assert_eq!(r.ex.working_count(), 0);
        let rows = lock(&r.audit.rows).clone();
        assert_eq!(rows[0].draft.kind, "paper_order_dropped");
        assert!(
            rows[0]
                .draft
                .payload
                .contains("\"reason\":\"insufficient_funds\"")
        );
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
        assert!(!r.ex.has_open_positions());
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
            (
                o.fills.len(),
                o.blocked_square_offs[0].1.clone(),
                o.rejected.len()
            ),
            (0, ExecError::Storage, 0)
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

    async fn open_mis_and_reach_square_off(r: &Rig) {
        tick_at(r, "100", 1000, 0).await;
        r.ex.accept(&intent(1, Side::Buy, 10, OrderKind::Market, Product::Mis))
            .await
            .unwrap();
        tick_at(r, "100", 10_000, 1).await;
        r.clock.set(at(13, 30, 0));
        r.ex.on_clock().await;
    }

    fn so_tick(r: &Rig, s: u32) -> Tick {
        let mut t = tick("100", 10_000, 0);
        t.at = at(13, 30, s);
        r.clock.set(t.at);
        t
    }

    #[tokio::test]
    async fn square_off_is_never_dropped_on_a_ledger_or_charge_error_and_retries() {
        let r = rig("100000").await;
        open_mis_and_reach_square_off(&r).await;
        r.charge_fail.store(true, Ordering::SeqCst);
        for s in 1..4 {
            let o = r.ex.on_tick(&so_tick(&r, s)).await;
            assert_eq!((o.fills.len(), o.blocked_square_offs.len()), (0, 1));
            assert_eq!(o.blocked_square_offs[0].1, ExecError::MarginUnavailable);
            assert_eq!(r.ex.position("ABC", Product::Mis), 10);
            assert_eq!(r.ex.working_count(), 1, "still working");
        }
        let kinds: Vec<String> = lock(&r.audit.rows)
            .iter()
            .map(|x| x.draft.kind.clone())
            .collect();
        assert_eq!(
            kinds
                .iter()
                .filter(|k| *k == "auto_square_off_failed")
                .count(),
            1,
            "failure audited once, not per retry"
        );
        r.charge_fail.store(false, Ordering::SeqCst);
        let o = r.ex.on_tick(&so_tick(&r, 5)).await;
        assert_eq!(o.fills.len(), 1);
        assert_eq!(r.ex.position("ABC", Product::Mis), 0);
        verify_chain(&lock(&r.audit.rows).clone()).unwrap();
    }

    #[tokio::test]
    async fn blocked_square_off_by_audit_outage_fills_once_audit_recovers() {
        let r = rig("100000").await;
        open_mis_and_reach_square_off(&r).await;
        r.audit.fail.store(true, Ordering::SeqCst);
        let o = r.ex.on_tick(&so_tick(&r, 1)).await;
        assert_eq!(o.blocked_square_offs.len(), 1);
        assert_eq!(r.ex.position("ABC", Product::Mis), 10);
        r.audit.fail.store(false, Ordering::SeqCst);
        let o = r.ex.on_tick(&so_tick(&r, 2)).await;
        assert_eq!((o.fills.len(), o.blocked_square_offs.len()), (1, 0));
        assert_eq!(r.ex.position("ABC", Product::Mis), 0);
    }

    /// RISK-142-7: the watch over a real executor with an audit outage: outstanding while the
    /// square-off is blocked, cleared when the retried fill completes.
    #[tokio::test]
    async fn watch_follows_a_real_blocked_square_off_until_it_fills() {
        let r = rig("100000").await;
        let mut w = crate::watch::SquareOffWatch::default();
        open_mis_and_reach_square_off(&r).await;
        r.audit.fail.store(true, Ordering::SeqCst);
        w.on_tick(&r.ex.on_tick(&so_tick(&r, 1)).await);
        assert_eq!(w.outstanding(), 1);
        w.on_tick(&r.ex.on_tick(&so_tick(&r, 2)).await);
        assert_eq!(w.outstanding(), 1, "still blocked on the retry");
        r.audit.fail.store(false, Ordering::SeqCst);
        w.on_tick(&r.ex.on_tick(&so_tick(&r, 3)).await);
        assert_eq!(w.outstanding(), 0);
        assert_eq!(r.ex.position("ABC", Product::Mis), 0);
    }

    #[test]
    fn watch_ignores_partial_fills_and_other_orders() {
        use crate::fill::{FillReason, SimFill};
        let fill = |id: u128, complete: bool| SimFill {
            id: IntentId(id),
            symbol: "ABC".into(),
            side: Side::Sell,
            qty: 1,
            price: d("50"),
            product: Product::Mis,
            at: at(13, 30, 1),
            complete,
            reason: FillReason::SquareOff,
        };
        let mut w = crate::watch::SquareOffWatch::default();
        w.on_tick(&TickOutcome {
            blocked_square_offs: vec![(IntentId(5), ExecError::Storage)],
            ..TickOutcome::default()
        });
        for f in [fill(5, false), fill(6, true)] {
            w.on_tick(&TickOutcome {
                fills: vec![f],
                ..TickOutcome::default()
            });
            assert_eq!(w.outstanding(), 1);
        }
        w.on_tick(&TickOutcome {
            fills: vec![fill(5, true)],
            ..TickOutcome::default()
        });
        assert_eq!(w.outstanding(), 0);
        let mut c = ClockOutcome::default();
        c.square_off.failed.push("ABC".into());
        w.on_clock(&c);
        assert_eq!(w.outstanding(), 1);
        w.on_clock(&ClockOutcome::default());
        assert_eq!(w.outstanding(), 0);
    }

    #[tokio::test]
    async fn out_of_session_ticks_never_fill_even_when_the_caller_is_wrong() {
        let r = rig("100000").await;
        tick_at(&r, "100", 1000, 0).await;
        r.ex.accept(&intent(1, Side::Buy, 5, OrderKind::Market, Product::Cnc))
            .await
            .unwrap();
        let mut t = tick("100", 1000, 0);
        t.at = at(20, 0, 0);
        r.clock.set(t.at);
        assert_eq!(r.ex.on_tick(&t).await.fills.len(), 0);
        assert_eq!(r.ex.working_count(), 1);
    }

    #[tokio::test]
    async fn clock_audits_mis_cancellation_and_day_expiry() {
        let r = rig("100000").await;
        r.ex.accept(&intent(
            1,
            Side::Buy,
            5,
            OrderKind::Limit(d("50")),
            Product::Mis,
        ))
        .await
        .unwrap();
        r.ex.accept(&intent(
            2,
            Side::Buy,
            5,
            OrderKind::Limit(d("50")),
            Product::Cnc,
        ))
        .await
        .unwrap();
        r.clock.set(at(13, 30, 0));
        let c = r.ex.on_clock().await;
        assert_eq!(c.square_off.cancelled, vec![IntentId(1)]);
        r.clock.set(at(10, 0, 0) + chrono::Duration::days(1));
        let c = r.ex.on_clock().await;
        assert_eq!(c.expired, vec![IntentId(2)]);
        assert_eq!(c.audit_failed.len(), 0);
        let rows = lock(&r.audit.rows).clone();
        let kinds: Vec<&str> = rows.iter().map(|x| x.draft.kind.as_str()).collect();
        assert_eq!(kinds, vec!["paper_order_cancelled", "paper_order_expired"]);
        verify_chain(&rows).unwrap();
        r.audit.fail.store(true, Ordering::SeqCst);
        r.ex.accept(&intent(
            3,
            Side::Buy,
            5,
            OrderKind::Limit(d("50")),
            Product::Cnc,
        ))
        .await
        .unwrap();
        r.clock.set(at(10, 0, 0) + chrono::Duration::days(2));
        assert_eq!(r.ex.on_clock().await.audit_failed, vec![IntentId(3)]);
    }
}
