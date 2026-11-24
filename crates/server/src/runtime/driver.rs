//! Clock-tick driver (BZ-511 S2): feeds the paper executor, expires stale approvals, raises the
//! `square_off_blocked` alarm (RISK-119-6) and supplies the breaker's P&L from the ledger.
//!
//! A blocked MIS square-off leaves a position open, so it is loud: an alarm on `GET /status` and
//! an error log (first pass, then every [`LOG_EVERY`] passes; the paper executor itself logs on
//! every tick) until a tick completes it (owner decision: no Telegram).

use std::future::Future;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use bazaar_core::clock::Clock;
use bazaar_core::ports::{PaperLedgerRepo, SourceError};
use bazaar_execution::breaker::{DayPnl, PnlSource};
use bazaar_execution::pipeline::PipelineHandle;
use bazaar_paper::account::LedgerKind;
use bazaar_paper::executor::{ClockOutcome, TickOutcome};
use bazaar_paper::fill::Tick;
use bazaar_paper::watch::SquareOffWatch;
use bazaar_store::SqliteStore;
use rust_decimal::Decimal;
use thiserror::Error;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::Paper;
use crate::state::AlarmBoard;

/// Alarm code shown on `GET /status` while a square-off is blocked.
pub const ALARM_SQUARE_OFF: &str = "square_off_blocked";
/// Alarm code while the audit anchor write is failing (ADR 0003): rows after the last good anchor
/// write could be truncated undetected until it is repaired.
const ALARM_AUDIT_ANCHOR: &str = "audit_anchor_lag";
const ANCHOR_TEXT: &str = "audit anchor write failing; the audit tail is not protected";
const ALARM_TEXT: &str = "MIS square-off blocked; a position stays open until it fills";
/// While an alarm is up, the driver's own error log repeats every this many passes.
const LOG_EVERY: u32 = 30;
/// Bound on the last pass after cancel (RISK-142-6): a hung store cannot hang shutdown.
const FINAL_PASS_TIMEOUT: Duration = Duration::from_secs(5);
/// Bound on waiting for the driver task to finish after cancel.
pub const STOP_WAIT: Duration = Duration::from_secs(10);
/// Floor on the pass interval (a zero interval would spin).
const MIN_EVERY: Duration = Duration::from_millis(100);

/// Whether any position is open (the P&L would need a mark price).
type HasOpenPositions = Arc<dyn Fn() -> bool + Send + Sync>;

/// Day P&L for the breaker, derived from the current-epoch ledger. Fail closed: with any open
/// position the P&L needs a mark price that is not available, so it is `Unavailable`; so is any
/// unreadable ledger line or a non-positive opening equity. Never a guessed zero.
#[derive(Clone)]
pub struct LedgerPnl {
    store: SqliteStore,
    has_open: HasOpenPositions,
    clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for LedgerPnl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LedgerPnl").finish_non_exhaustive()
    }
}

impl std::fmt::Debug for Driver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Driver").finish_non_exhaustive()
    }
}

impl LedgerPnl {
    #[must_use]
    pub fn new(store: SqliteStore, has_open: HasOpenPositions, clock: Arc<dyn Clock>) -> Self {
        Self {
            store,
            has_open,
            clock,
        }
    }

    async fn compute(&self) -> Result<DayPnl, SourceError> {
        let today = self.clock.now_ist().date_naive().to_string();
        let lines = PaperLedgerRepo::current_epoch(&self.store)
            .await
            .map_err(|_| SourceError::Unavailable)?;
        // Checked AFTER the ledger read: a fill booked in between shows as a loss (a false trip),
        // never as a hidden one.
        if (self.has_open)() {
            return Err(SourceError::Unavailable);
        }
        let (mut base, mut pnl) = (Decimal::ZERO, Decimal::ZERO);
        for l in &lines {
            let kind = LedgerKind::parse(&l.kind).ok_or(SourceError::Unavailable)?;
            // Ledger lines are stamped with the IST clock (`+05:30`); anything else is refused.
            let day = l
                .ts_ist_rfc3339
                .strip_suffix("+05:30")
                .and_then(|t| t.get(..10))
                .ok_or(SourceError::Unavailable)?;
            let slot = if day < today.as_str() || kind == LedgerKind::Opening {
                &mut base
            } else {
                &mut pnl
            };
            *slot = slot.checked_add(l.amount).ok_or(SourceError::Unavailable)?;
        }
        if base <= Decimal::ZERO {
            return Err(SourceError::Unavailable);
        }
        Ok(DayPnl {
            pnl,
            equity_base: base,
        })
    }
}

impl PnlSource for LedgerPnl {
    fn day_pnl(&self) -> impl Future<Output = Result<DayPnl, SourceError>> + Send {
        self.compute()
    }
}

/// Drives the paper executor and the approval timer. Held in an `Arc`: `run` takes one clone and
/// the quote feed slice calls [`Driver::on_tick`] on another (HARD CONDITION in the F10 record).
pub struct Driver {
    paper: Arc<Paper>,
    handle: PipelineHandle,
    alarms: AlarmBoard,
    clock: Arc<dyn Clock>,
    watch: Mutex<SquareOffWatch>,
    /// The store whose audit anchor health is watched (`None`: nothing watched).
    anchor: Option<SqliteStore>,
    /// Passes since the alarm went up (for the log rate limit).
    up_passes: AtomicU32,
}

impl Driver {
    #[must_use]
    pub fn new(
        paper: Arc<Paper>,
        handle: PipelineHandle,
        alarms: AlarmBoard,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            paper,
            handle,
            alarms,
            clock,
            watch: Mutex::new(SquareOffWatch::default()),
            anchor: None,
            up_passes: AtomicU32::new(0),
        }
    }

    /// Watch the audit anchor of `store` each timer pass: a lagging anchor raises
    /// [`ALARM_AUDIT_ANCHOR`] (after one retry) and a repaired one clears it.
    #[must_use]
    pub fn with_anchor(mut self, store: SqliteStore) -> Self {
        self.anchor = Some(store);
        self
    }

    /// Retry a lagging anchor write once, then raise or clear the alarm from the status.
    async fn check_anchor(&self) {
        let Some(store) = &self.anchor else { return };
        let mut st = store.anchor_status().await;
        if st.is_some_and(|s| s.lag) {
            if store.anchor_flush().await.is_err() {
                tracing::warn!(category = "anchor_write", "audit anchor still failing");
            }
            st = store.anchor_status().await;
        }
        // No status at all means no anchor on a store that must have one: alarm, never "fine".
        if st.is_none_or(|s| s.lag) {
            self.alarms.raise(
                ALARM_AUDIT_ANCHOR,
                ANCHOR_TEXT,
                &self.clock.now_ist().to_rfc3339(),
            );
        } else {
            self.alarms.clear(ALARM_AUDIT_ANCHOR);
        }
    }

    fn refresh_alarm(&self) {
        let outstanding = self
            .watch
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .outstanding();
        if outstanding > 0 {
            if self
                .up_passes
                .fetch_add(1, Ordering::Relaxed)
                .is_multiple_of(LOG_EVERY)
            {
                tracing::error!(outstanding, "MIS square-off still blocked");
            }
            self.alarms.raise(
                ALARM_SQUARE_OFF,
                ALARM_TEXT,
                &self.clock.now_ist().to_rfc3339(),
            );
        } else {
            self.up_passes.store(0, Ordering::Relaxed);
            self.alarms.clear(ALARM_SQUARE_OFF);
        }
    }

    /// Feed one market tick (the quote feed calls this when one is wired).
    pub async fn on_tick(&self, tick: &Tick) {
        let out = self.paper.on_tick(tick).await;
        self.apply_tick(&out);
    }

    fn apply_tick(&self, out: &TickOutcome) {
        self.watch
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .on_tick(out);
        self.refresh_alarm();
    }

    /// One timer pass: clock work (expiry, square-off), then stale approvals.
    pub async fn on_timer(&self) {
        let out = self.paper.on_clock().await;
        self.apply_clock(&out);
        if let Err(e) = self.handle.expire_stale().await {
            tracing::error!(error = %e, "approval expiry pass failed");
        }
        self.check_anchor().await;
    }

    fn apply_clock(&self, out: &ClockOutcome) {
        self.watch
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .on_clock(out);
        self.refresh_alarm();
    }

    /// Run until `cancel`, then one last bounded pass.
    pub async fn run(self: Arc<Self>, every: Duration, cancel: CancellationToken) {
        let pass = || self.on_timer();
        if !run_loop(every, cancel, FINAL_PASS_TIMEOUT, pass).await {
            tracing::error!("final tick-driver pass did not finish in time");
        }
    }
}

/// Loop `pass` every `every` until `cancel`, then run it once more bounded by `final_timeout`;
/// returns whether that last pass finished in time.
async fn run_loop<F, Fut>(
    every: Duration,
    cancel: CancellationToken,
    final_timeout: Duration,
    pass: F,
) -> bool
where
    F: Fn() -> Fut,
    Fut: Future<Output = ()>,
{
    let mut t = tokio::time::interval(every.max(MIN_EVERY));
    loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => break,
            _ = t.tick() => pass().await,
        }
    }
    tokio::time::timeout(final_timeout, pass()).await.is_ok()
}

/// The driver task ended while the server was still serving (RISK-142-1).
#[derive(Debug, Error)]
#[error("tick driver stopped before shutdown: {0}")]
pub struct DriverStopped(pub String);

/// Run `serve` while watching the driver task: if the task ends or panics first, the server
/// future is dropped (it stops serving) and the error is returned, so a dead safety loop never
/// runs silently. `task` must not be awaited again after an `Err`.
///
/// # Errors
/// [`DriverStopped`] when the task finished before `serve`.
pub async fn serve_while_driven<T>(
    serve: impl Future<Output = T>,
    task: &mut JoinHandle<()>,
) -> Result<T, DriverStopped> {
    tokio::select! {
        out = serve => Ok(out),
        done = &mut *task => Err(DriverStopped(match done {
            Ok(()) => "task returned".to_owned(),
            Err(e) if e.is_panic() => "task panicked".to_owned(),
            Err(e) => format!("task failed: {e}"),
        })),
    }
}

/// After cancel: wait up to `wait` for the task; abort it on timeout. True on a clean finish.
pub async fn stop(task: &mut JoinHandle<()>, wait: Duration) -> bool {
    match tokio::time::timeout(wait, &mut *task).await {
        Ok(Ok(())) => true,
        Ok(Err(_)) => false,
        Err(_) => {
            task.abort();
            false
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::path::Path;

    use bazaar_core::calendar::MarketCalendar;
    use bazaar_core::config::{CalendarConfig, load};
    use bazaar_core::ports::LedgerEntry;
    use bazaar_core::testutil::FakeClock;
    use bazaar_execution::executor::ExecError;
    use bazaar_execution::types::{IntentId, Product};
    use bazaar_paper::fill::{FillReason, SimFill};
    use bazaar_risk::types::Side;
    use chrono::TimeZone;
    use chrono_tz::Asia::Kolkata;

    use super::*;
    use crate::runtime::{Runtime, start};

    const RISK: &str = include_str!("../../../../fixtures/config/risk.toml");

    fn clock() -> Arc<FakeClock> {
        Arc::new(FakeClock::at(
            Kolkata.with_ymd_and_hms(2026, 11, 19, 11, 0, 0).unwrap(),
        ))
    }

    async fn rt(c: &Arc<FakeClock>) -> (Runtime, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = concat!(env!("CARGO_MANIFEST_DIR"), "/../../config");
        let cal = MarketCalendar::from_config(&load::<CalendarConfig>(Path::new(cfg)).unwrap());
        let r = start(
            &dir.path().join("x.db"),
            RISK,
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config"),
            c.clone(),
            cal,
        )
        .await
        .unwrap();
        (r, dir)
    }

    fn driver(r: &Runtime, c: &Arc<FakeClock>) -> (Driver, AlarmBoard) {
        let alarms = AlarmBoard::new();
        let d = Driver::new(r.paper.clone(), r.handle.clone(), alarms.clone(), c.clone());
        (d, alarms)
    }

    fn fill(id: u128, complete: bool, c: &FakeClock) -> SimFill {
        SimFill {
            id: IntentId(id),
            symbol: "ABC".into(),
            side: Side::Sell,
            qty: 1,
            price: Decimal::from(50),
            product: Product::Mis,
            at: c.now_ist(),
            complete,
            reason: FillReason::SquareOff,
        }
    }

    fn filled(f: SimFill) -> TickOutcome {
        TickOutcome {
            fills: vec![f],
            ..TickOutcome::default()
        }
    }

    /// RISK-119-6: the alarm is up while a square-off is blocked and goes only when that order's
    /// COMPLETING fill is booked (RISK-142-3). The booking glue itself is tested in `paper`.
    #[tokio::test]
    async fn the_alarm_follows_the_watch_and_a_partial_fill_does_not_clear_it() {
        let c = clock();
        let (r, _d) = rt(&c).await;
        let (d, alarms) = driver(&r, &c);
        d.apply_tick(&TickOutcome {
            blocked_square_offs: vec![(IntentId(5), ExecError::Storage)],
            ..TickOutcome::default()
        });
        let a = alarms.snapshot();
        assert_eq!(a.len(), 1);
        assert_eq!(
            (a[0].code.as_str(), a[0].message.as_str()),
            (ALARM_SQUARE_OFF, ALARM_TEXT)
        );
        d.apply_tick(&filled(fill(5, false, &c)));
        assert_eq!(alarms.snapshot().len(), 1, "partial fill keeps the alarm");
        d.apply_tick(&filled(fill(5, true, &c)));
        assert_eq!(alarms.snapshot(), vec![]);
    }

    #[tokio::test]
    async fn a_square_off_that_cannot_be_queued_alarms_until_a_clock_call_queues_it() {
        let c = clock();
        let (r, _d) = rt(&c).await;
        let (d, alarms) = driver(&r, &c);
        let mut out = ClockOutcome::default();
        out.square_off.failed.push("ABC".into());
        d.apply_clock(&out);
        assert_eq!(alarms.snapshot()[0].code, ALARM_SQUARE_OFF);
        d.apply_clock(&ClockOutcome::default());
        assert_eq!(alarms.snapshot(), vec![]);
    }

    /// RISK-142-6: a pre-cancelled loop still makes the final pass, and a stalled one is cut off.
    #[tokio::test]
    async fn the_final_pass_runs_after_cancel_and_is_bounded() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        let runs = AtomicU32::new(0);
        let ok = run_loop(
            Duration::from_secs(3600),
            cancel.clone(),
            Duration::from_secs(5),
            || {
                runs.fetch_add(1, Ordering::SeqCst);
                std::future::ready(())
            },
        )
        .await;
        assert!(ok);
        assert_eq!(runs.load(Ordering::SeqCst), 1, "exactly the final pass");
        let stalled = run_loop(
            Duration::from_secs(3600),
            cancel,
            Duration::from_millis(50),
            std::future::pending::<()>,
        )
        .await;
        assert!(!stalled);
    }

    /// A failing anchor write raises the alarm on a timer pass; a repaired one clears it.
    #[tokio::test]
    async fn a_lagging_audit_anchor_raises_the_alarm_and_repair_clears_it() {
        let c = clock();
        let (r, dir) = rt(&c).await;
        let alarms = AlarmBoard::new();
        let d = Driver::new(r.paper.clone(), r.handle.clone(), alarms.clone(), c.clone())
            .with_anchor(r.store.clone());
        d.on_timer().await;
        assert_eq!(alarms.snapshot(), vec![], "healthy anchor: no alarm");
        // The first audit row creates the anchor file; a second one then has to rewrite it.
        r.handle
            .set_kill_switch(true, "owner", "first")
            .await
            .unwrap();
        r.store.anchor_flush().await.unwrap();
        // A directory where the anchor file belongs makes every write fail.
        let ap = dir.path().join("x.db.anchor");
        std::fs::remove_file(&ap).unwrap();
        std::fs::create_dir(&ap).unwrap();
        r.handle
            .set_kill_switch(false, "owner", "RELEASE KILL SWITCH")
            .await
            .unwrap();
        r.store.anchor_flush().await.unwrap_err();
        d.on_timer().await;
        let a = alarms.snapshot();
        assert_eq!(a.len(), 1, "{a:?}");
        assert_eq!(
            (a[0].code.as_str(), a[0].message.as_str()),
            (ALARM_AUDIT_ANCHOR, ANCHOR_TEXT)
        );
        // Repair: the next pass retries the write, finds it good and clears the alarm.
        std::fs::remove_dir(&ap).unwrap();
        d.on_timer().await;
        assert_eq!(alarms.snapshot(), vec![]);
        assert!(!r.store.anchor_status().await.unwrap().lag);
    }

    /// A driver that watches no store never raises the anchor alarm.
    #[tokio::test]
    async fn a_driver_without_an_anchor_store_never_raises_the_anchor_alarm() {
        let c = clock();
        let (r, _d) = rt(&c).await;
        let (d, alarms) = driver(&r, &c);
        d.on_timer().await;
        assert_eq!(alarms.snapshot(), vec![]);
    }

    #[tokio::test]
    async fn run_stops_on_cancel_through_the_public_entry() {
        let c = clock();
        let (r, _d) = rt(&c).await;
        let (d, _a) = driver(&r, &c);
        let cancel = CancellationToken::new();
        let mut task = tokio::spawn(Arc::new(d).run(Duration::from_millis(100), cancel.clone()));
        cancel.cancel();
        assert!(stop(&mut task, Duration::from_secs(5)).await);
    }

    /// RISK-142-1 / SEC-142-1: a dead driver ends serving instead of running silently.
    #[tokio::test]
    async fn a_panicking_or_returning_driver_ends_the_server_future() {
        let mut t = tokio::spawn(async { panic!("boom") });
        let e = serve_while_driven(std::future::pending::<()>(), &mut t)
            .await
            .unwrap_err();
        assert_eq!(e.0, "task panicked");
        let mut t = tokio::spawn(async {});
        let e = serve_while_driven(std::future::pending::<()>(), &mut t)
            .await
            .unwrap_err();
        assert_eq!(e.0, "task returned");
        // The server finishing first is the normal path.
        let mut t = tokio::spawn(std::future::pending::<()>());
        assert_eq!(serve_while_driven(async { 7 }, &mut t).await.unwrap(), 7);
        t.abort();
    }

    #[tokio::test]
    async fn stop_aborts_a_task_that_does_not_finish() {
        let mut t = tokio::spawn(std::future::pending::<()>());
        assert!(!stop(&mut t, Duration::from_millis(50)).await);
        assert!(t.await.unwrap_err().is_cancelled());
    }

    async fn line(r: &Runtime, kind: &str, amount: i64, ts: String) {
        let epoch = PaperLedgerRepo::current_epoch(&r.store).await.unwrap()[0].epoch;
        PaperLedgerRepo::append(
            &r.store,
            LedgerEntry {
                epoch,
                kind: kind.into(),
                amount: Decimal::from(amount),
                ref_id: Some(1),
                ts_ist_rfc3339: ts,
            },
        )
        .await
        .unwrap();
    }

    fn pnl(r: &Runtime, c: &Arc<FakeClock>, open: bool) -> LedgerPnl {
        LedgerPnl::new(r.store.clone(), Arc::new(move || open), c.clone())
    }

    #[tokio::test]
    async fn ledger_pnl_splits_today_from_the_opening_base() {
        let c = clock();
        let (r, _d) = rt(&c).await;
        let p = pnl(&r, &c, false);
        let got = p.day_pnl().await.unwrap();
        assert_eq!(got.pnl, Decimal::ZERO);
        assert_eq!(got.equity_base, Decimal::from(250_000));
        // Yesterday's line moves the base, today's the P&L.
        line(&r, "sell", 1000, "2026-11-18T15:00:00+05:30".into()).await;
        line(&r, "charge", -30, c.now_ist().to_rfc3339()).await;
        let got = p.day_pnl().await.unwrap();
        assert_eq!(got.pnl, Decimal::from(-30));
        assert_eq!(got.equity_base, Decimal::from(251_000));
    }

    #[tokio::test]
    async fn ledger_pnl_is_unavailable_on_an_unreadable_line() {
        let c = clock();
        let (r, _d) = rt(&c).await;
        line(&r, "charge", -1, "2026-11-19T11:00:00+00:00".into()).await;
        assert_eq!(
            pnl(&r, &c, false).day_pnl().await,
            Err(SourceError::Unavailable)
        );
    }

    /// CR-142-1: the two other fail-closed branches.
    #[tokio::test]
    async fn ledger_pnl_is_unavailable_with_an_open_position_or_a_non_positive_base() {
        let c = clock();
        let (r, _d) = rt(&c).await;
        assert_eq!(
            pnl(&r, &c, true).day_pnl().await,
            Err(SourceError::Unavailable),
            "open position"
        );
        // A prior-day loss that wipes the base out.
        line(&r, "buy", -250_000, "2026-11-18T15:00:00+05:30".into()).await;
        assert_eq!(
            pnl(&r, &c, false).day_pnl().await,
            Err(SourceError::Unavailable),
            "base is zero"
        );
    }
}
