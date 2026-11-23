//! Clock-tick driver (BZ-511 S2): feeds the paper executor, expires stale approvals, raises the
//! `square_off_blocked` alarm (RISK-119-6) and supplies the breaker's P&L from the ledger.
//!
//! A blocked MIS square-off leaves a position open, so it is loud: an alarm on `GET /status` and
//! an error log on every tick until a later tick fills it (owner decision: no Telegram).

use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use bazaar_core::clock::Clock;
use bazaar_core::ports::{PaperLedgerRepo, SourceError};
use bazaar_execution::breaker::{DayPnl, PnlSource};
use bazaar_execution::pipeline::PipelineHandle;
use bazaar_execution::types::IntentId;
use bazaar_paper::account::LedgerKind;
use bazaar_paper::executor::{ClockOutcome, TickOutcome};
use bazaar_paper::fill::Tick;
use bazaar_store::SqliteStore;
use rust_decimal::Decimal;
use tokio_util::sync::CancellationToken;

use super::Paper;
use crate::state::AlarmBoard;

/// Alarm code shown on `GET /status` while a square-off is blocked.
pub const ALARM_SQUARE_OFF: &str = "square_off_blocked";
const ALARM_TEXT: &str = "MIS square-off blocked; a position stays open until it fills";

/// Day P&L for the breaker, derived from the current-epoch ledger. Fail closed: with any open
/// position the P&L needs a mark price that is not available, so it is `Unavailable`; so is any
/// unreadable ledger line or a non-positive opening equity. Never a guessed zero.
#[derive(Clone)]
pub struct LedgerPnl {
    store: SqliteStore,
    paper: Arc<Paper>,
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
    pub fn new(store: SqliteStore, paper: Arc<Paper>, clock: Arc<dyn Clock>) -> Self {
        Self {
            store,
            paper,
            clock,
        }
    }

    async fn compute(&self) -> Result<DayPnl, SourceError> {
        if self.paper.has_open_positions() {
            return Err(SourceError::Unavailable);
        }
        let today = self.clock.now_ist().date_naive().to_string();
        let lines = PaperLedgerRepo::current_epoch(&self.store)
            .await
            .map_err(|_| SourceError::Unavailable)?;
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

/// Drives the paper executor and the approval timer.
pub struct Driver {
    paper: Arc<Paper>,
    handle: PipelineHandle,
    alarms: AlarmBoard,
    clock: Arc<dyn Clock>,
    /// Square-off orders whose booking failed and that have not filled since.
    blocked: Mutex<HashSet<IntentId>>,
    /// The last clock call could not queue a square-off for some symbol.
    queue_failed: Mutex<bool>,
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
            blocked: Mutex::new(HashSet::new()),
            queue_failed: Mutex::new(false),
        }
    }

    fn refresh_alarm(&self) {
        let blocked = self
            .blocked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        let queue = *self
            .queue_failed
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if blocked > 0 || queue {
            tracing::error!(
                blocked,
                queue_failed = queue,
                "MIS square-off still blocked"
            );
            self.alarms.raise(
                ALARM_SQUARE_OFF,
                ALARM_TEXT,
                &self.clock.now_ist().to_rfc3339(),
            );
        } else {
            self.alarms.clear(ALARM_SQUARE_OFF);
        }
    }

    /// Feed one market tick (the quote feed calls this when one is wired).
    pub async fn on_tick(&self, tick: &Tick) {
        let out = self.paper.on_tick(tick).await;
        self.apply_tick(&out);
    }

    fn apply_tick(&self, out: &TickOutcome) {
        {
            let mut b = self.blocked.lock().unwrap_or_else(PoisonError::into_inner);
            for f in &out.fills {
                b.remove(&f.id);
            }
            for (id, _) in &out.blocked_square_offs {
                b.insert(*id);
            }
        }
        self.refresh_alarm();
    }

    /// One timer pass: clock work (expiry, square-off), then stale approvals.
    pub async fn on_timer(&self) {
        let out = self.paper.on_clock().await;
        self.apply_clock(&out);
        if let Err(e) = self.handle.expire_stale().await {
            tracing::error!(error = %e, "approval expiry pass failed");
        }
    }

    fn apply_clock(&self, out: &ClockOutcome) {
        *self
            .queue_failed
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = !out.square_off.failed.is_empty();
        self.refresh_alarm();
    }

    /// Run until `cancel`; one last pass runs on the way out so nothing is left half-handled.
    pub async fn run(self, every: Duration, cancel: CancellationToken) {
        let mut t = tokio::time::interval(every.max(Duration::from_millis(100)));
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                _ = t.tick() => self.on_timer().await,
            }
        }
        self.on_timer().await;
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
    use bazaar_execution::types::Product;
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
        let r = start(&dir.path().join("x.db"), RISK, c.clone(), cal)
            .await
            .unwrap();
        (r, dir)
    }

    fn driver(r: &Runtime, c: &Arc<FakeClock>) -> (Driver, AlarmBoard) {
        let alarms = AlarmBoard::new();
        let d = Driver::new(r.paper.clone(), r.handle.clone(), alarms.clone(), c.clone());
        (d, alarms)
    }

    fn fill(id: u128, c: &FakeClock) -> SimFill {
        SimFill {
            id: IntentId(id),
            symbol: "ABC".into(),
            side: Side::Sell,
            qty: 1,
            price: Decimal::from(50),
            product: Product::Mis,
            at: c.now_ist(),
            complete: true,
            reason: FillReason::SquareOff,
        }
    }

    fn blocked(id: u128) -> TickOutcome {
        TickOutcome {
            blocked_square_offs: vec![(IntentId(id), ExecError::Storage)],
            ..TickOutcome::default()
        }
    }

    /// RISK-119-6: a blocked square-off raises the alarm and only that order filling clears it.
    #[tokio::test]
    async fn a_blocked_square_off_alarms_until_that_order_fills() {
        let c = clock();
        let (r, _d) = rt(&c).await;
        let (d, alarms) = driver(&r, &c);
        d.apply_tick(&blocked(5));
        let a = alarms.snapshot();
        assert_eq!(a.len(), 1);
        assert_eq!(
            (a[0].code.as_str(), a[0].message.as_str()),
            (ALARM_SQUARE_OFF, ALARM_TEXT)
        );
        // A different order filling does not clear it.
        d.apply_tick(&TickOutcome {
            fills: vec![fill(6, &c)],
            ..TickOutcome::default()
        });
        assert_eq!(alarms.snapshot().len(), 1);
        // The blocked one filling does.
        d.apply_tick(&TickOutcome {
            fills: vec![fill(5, &c)],
            ..TickOutcome::default()
        });
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

    #[tokio::test]
    async fn the_timer_pass_runs_and_run_stops_on_cancel() {
        let c = clock();
        let (r, _d) = rt(&c).await;
        let (d, alarms) = driver(&r, &c);
        d.on_timer().await;
        assert_eq!(alarms.snapshot(), vec![]);
        let cancel = CancellationToken::new();
        let task = tokio::spawn(d.run(Duration::from_millis(100), cancel.clone()));
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("driver stops")
            .unwrap();
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

    #[tokio::test]
    async fn ledger_pnl_splits_today_from_the_opening_base() {
        let c = clock();
        let (r, _d) = rt(&c).await;
        let p = LedgerPnl::new(r.store.clone(), r.paper.clone(), c.clone());
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
        let p = LedgerPnl::new(r.store.clone(), r.paper.clone(), c.clone());
        line(&r, "charge", -1, "2026-11-19T11:00:00+00:00".into()).await;
        assert_eq!(p.day_pnl().await, Err(SourceError::Unavailable));
    }
}
