//! Daily-loss breaker. Trips when the day's P&L reaches `-(equity_base * max_daily_loss_pct / 100)`
//! and stays tripped until the next trading day, even if P&L recovers. An unreadable P&L source,
//! a non-positive equity base or a calendar error counts as tripped (fail closed).
//!
//! The latch survives a restart: `new` re-reads the audit log and restores `until` from the
//! `breaker_tripped` rows (RISK-105-1). The trip is latched before its audit row is written; a
//! failed append is kept and retried on the next check until it lands (`audit_pending`).
//!
//! Limits: the trading-day calendar uses `Segment::Equity` for every product (the intent carries
//! no segment). A calendar error latches in-process to `NaiveDate::MAX` and is logged; the audit
//! row records `until = "max"`. On the next start `new` re-derives the date from the row's `day`
//! with the (fixed) calendar, or holds one calendar day if the calendar still errors, so a restart
//! after the calendar is fixed (or the next day) clears it. No typed-phrase clear exists.
//! `DayPnl::equity_base` must be the start-of-day capital, not live equity.

use std::future::Future;
use std::sync::{Arc, Mutex};

use bazaar_core::calendar::MarketCalendar;
use bazaar_core::ports::{AuditDraft, AuditRepo, AuditRow, SourceError};
use bazaar_core::segment::Segment;
use chrono::{DateTime, NaiveDate};
use chrono_tz::Tz;
use rust_decimal::Decimal;

use crate::audit::chain_hash;
use crate::executor::ExecError;

const TRIP_KIND: &str = "breaker_tripped";

/// Today's realised plus open P&L and the equity it is measured against.
/// `equity_base` is the capital at the start of the day, so intraday profit cannot loosen the limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DayPnl {
    pub pnl: Decimal,
    pub equity_base: Decimal,
}

/// Supplies the day's P&L.
pub trait PnlSource: Send + Sync {
    fn day_pnl(&self) -> impl Future<Output = Result<DayPnl, SourceError>> + Send;
}

/// Breaker over a P&L source, with its trip audit row going to an `AuditRepo`.
pub struct DailyLossBreaker<P: PnlSource, A: AuditRepo> {
    src: P,
    audit: Arc<A>,
    calendar: MarketCalendar,
    max_loss_pct: Decimal,
    /// First date on which the breaker is clear again (set while tripped).
    until: Mutex<Option<NaiveDate>>,
    /// A trip audit row that failed to append; retried on the next check.
    pending: Mutex<Option<AuditDraft>>,
}

impl<P: PnlSource, A: AuditRepo> std::fmt::Debug for DailyLossBreaker<P, A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DailyLossBreaker").finish_non_exhaustive()
    }
}

fn parse_until(s: &str) -> Option<NaiveDate> {
    if s == "max" {
        return Some(NaiveDate::MAX);
    }
    s.parse().ok()
}

/// The latch as the audit log records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestoredLatch {
    /// First clear date when still latched on the asked day; `None` when clear.
    pub until: Option<NaiveDate>,
    /// `breaker_tripped` rows that could not be read. The breaker ignores them; a caller that
    /// must fail closed (the paper reset guard) treats any as unknown.
    pub unparsable: usize,
}

/// Restores the latch on `today` from the `breaker_tripped` rows (RISK-105-1). Pure; the one
/// reading of those rows, shared by [`DailyLossBreaker::new`] and the paper reset guard.
#[must_use]
pub fn restore_latch(
    rows: &[AuditRow],
    calendar: &MarketCalendar,
    today: NaiveDate,
) -> RestoredLatch {
    let mut out = RestoredLatch {
        until: None,
        unparsable: 0,
    };
    for r in rows.iter().filter(|r| r.draft.kind == TRIP_KIND) {
        match trip_until(&r.draft.payload, calendar) {
            Some(d) if today < d => out.until = out.until.max(Some(d)),
            Some(_) => {}
            None => out.unparsable += 1,
        }
    }
    out
}

/// The first clear date of one trip row; `None` when the row is unreadable.
fn trip_until(payload: &str, calendar: &MarketCalendar) -> Option<NaiveDate> {
    let v: serde_json::Value = serde_json::from_str(payload).ok()?;
    let until = parse_until(v.get("until")?.as_str()?)?;
    if until != NaiveDate::MAX {
        return Some(until);
    }
    // A calendar-error trip: re-derive from the trip day now that the calendar may be
    // fixed; if it still errors, hold one calendar day (RISK-105-10).
    let day: NaiveDate = v.get("day")?.as_str()?.parse().ok()?;
    Some(
        calendar
            .next_trading_day(day, Segment::Equity)
            .ok()
            .or_else(|| day.succ_opt())
            .unwrap_or(NaiveDate::MAX),
    )
}

impl<P: PnlSource, A: AuditRepo> DailyLossBreaker<P, A> {
    /// `max_loss_pct` is `limits.max_daily_loss_pct`; `hard_max_pct` is
    /// `hard_limits.max_daily_loss_pct`. Restores the latch from the audit log as of `now`.
    ///
    /// # Errors
    /// `InvalidInput` when `max_loss_pct` is not in `(0, hard_max_pct]`; `Storage` when the audit
    /// log cannot be read (the latch cannot be restored, so no breaker is built).
    pub async fn new(
        src: P,
        audit: Arc<A>,
        calendar: MarketCalendar,
        max_loss_pct: Decimal,
        hard_max_pct: Decimal,
        now: DateTime<Tz>,
    ) -> Result<Self, ExecError> {
        if max_loss_pct <= Decimal::ZERO || max_loss_pct > hard_max_pct {
            return Err(ExecError::InvalidInput(
                "max_loss_pct must be above 0 and within the hard ceiling",
            ));
        }
        let rows = audit.read_all().await.map_err(|e| {
            tracing::warn!(category = e.safe_reason(), "breaker latch not restorable");
            ExecError::Storage
        })?;
        let restored = restore_latch(&rows, &calendar, now.date_naive());
        if restored.unparsable > 0 {
            // The count only, never payload text.
            tracing::warn!(
                unparsable = restored.unparsable,
                "breaker latch restore ignored unreadable trip rows"
            );
        }
        Ok(Self {
            src,
            audit,
            calendar,
            max_loss_pct,
            until: Mutex::new(restored.until),
            pending: Mutex::new(None),
        })
    }

    fn latched(&self, today: NaiveDate) -> bool {
        self.until
            .lock()
            .map_or(true, |u| u.is_some_and(|d| today < d))
    }

    /// `Some(pnl)` when over the limit (`pnl` is `None` when the source or equity was unusable).
    async fn over_limit(&self) -> Option<Option<DayPnl>> {
        let Ok(p) = self.src.day_pnl().await else {
            return Some(None);
        };
        let limit = p
            .equity_base
            .checked_mul(self.max_loss_pct)
            .and_then(|v| v.checked_div(Decimal::ONE_HUNDRED));
        match limit {
            Some(l) if p.equity_base > Decimal::ZERO => (p.pnl <= -l).then_some(Some(p)),
            _ => Some(None),
        }
    }

    async fn flush_pending(&self) {
        let Some(draft) = self.pending.lock().ok().and_then(|mut g| g.take()) else {
            return;
        };
        if let Err(e) = self.audit.append(draft.clone(), chain_hash).await {
            tracing::warn!(
                category = e.safe_reason(),
                "breaker trip audit retry failed"
            );
            if let Ok(mut g) = self.pending.lock() {
                *g = Some(draft);
            }
        }
    }

    /// True while a trip audit row is waiting to be written.
    #[must_use]
    pub fn audit_pending(&self) -> bool {
        self.pending.lock().map_or(true, |g| g.is_some())
    }

    /// `None` clear; `Some(newly)` tripped (`newly` on the check that tripped it).
    pub async fn tripped(&self, now: DateTime<Tz>) -> Option<bool> {
        let today = now.date_naive();
        if self.latched(today) {
            self.flush_pending().await;
            return Some(false);
        }
        let pnl = self.over_limit().await?;
        let until = self
            .calendar
            .next_trading_day(today, Segment::Equity)
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "breaker: calendar error; latched to max");
                NaiveDate::MAX
            });
        // Compare-and-set under one lock: only one concurrent check trips and audits.
        {
            let Ok(mut u) = self.until.lock() else {
                return Some(false);
            };
            if u.is_some_and(|d| today < d) {
                return Some(false);
            }
            *u = Some(until);
        }
        let until_s = if until == NaiveDate::MAX {
            "max".to_owned()
        } else {
            until.to_string()
        };
        let payload = serde_json::json!({
            "day": today.to_string(),
            "until": until_s,
            "pnl": pnl.map(|p| p.pnl.to_string()),
            "equity_base": pnl.map(|p| p.equity_base.to_string()),
            "max_loss_pct": self.max_loss_pct.to_string(),
        })
        .to_string();
        let draft = AuditDraft {
            ts_ist_rfc3339: now.to_rfc3339(),
            actor: "breaker".to_owned(),
            kind: TRIP_KIND.to_owned(),
            payload,
        };
        if let Err(e) = self.audit.append(draft.clone(), chain_hash).await {
            tracing::warn!(
                category = e.safe_reason(),
                "breaker trip audit failed; will retry"
            );
            if let Ok(mut g) = self.pending.lock() {
                *g = Some(draft);
            }
        }
        Some(true)
    }
}

#[cfg(test)]
#[allow(clippy::unused_async_trait_impl)]
pub(crate) mod fakes {
    use super::*;
    use std::sync::Mutex;

    pub(crate) fn cal() -> MarketCalendar {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config");
        MarketCalendar::from_config(
            &bazaar_core::config::load::<bazaar_core::config::CalendarConfig>(&dir).unwrap(),
        )
    }

    /// P&L source returning a settable value (`None` = source down).
    pub(crate) struct FakePnl(pub Mutex<Option<DayPnl>>);
    impl FakePnl {
        pub(crate) fn set(pnl: i64, cents: bool) -> Self {
            let p = if cents {
                Decimal::new(pnl, 2)
            } else {
                Decimal::from(pnl)
            };
            Self(Mutex::new(Some(DayPnl {
                pnl: p,
                equity_base: Decimal::from(100_000),
            })))
        }
    }
    impl PnlSource for FakePnl {
        async fn day_pnl(&self) -> Result<DayPnl, SourceError> {
            self.0.lock().unwrap().ok_or(SourceError::Unavailable)
        }
    }

    /// Breaker over a fresh repo: 2 % of 100000 = 2000, hard ceiling 3 %, built on 2026-11-19.
    pub(crate) async fn breaker_on(
        p: FakePnl,
        r: &Arc<crate::killswitch::fakes::MemRepo>,
        now: DateTime<Tz>,
    ) -> DailyLossBreaker<FakePnl, crate::killswitch::fakes::MemRepo> {
        DailyLossBreaker::new(p, r.clone(), cal(), Decimal::from(2), Decimal::from(3), now)
            .await
            .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::fakes::*;
    use super::*;
    use crate::killswitch::fakes::MemRepo;
    use chrono::TimeZone;
    use chrono_tz::Asia::Kolkata;
    use std::sync::atomic::Ordering::SeqCst;

    fn day(d: u32, h: u32) -> DateTime<Tz> {
        Kolkata.with_ymd_and_hms(2026, 11, d, h, 0, 0).unwrap()
    }

    async fn br(p: FakePnl) -> (DailyLossBreaker<FakePnl, MemRepo>, Arc<MemRepo>) {
        let r = Arc::new(MemRepo::default());
        (breaker_on(p, &r, day(19, 9)).await, r)
    }

    #[tokio::test]
    async fn boundary_one_paisa_inside_passes_equal_trips() {
        assert_eq!(
            br(FakePnl::set(-199_999, true))
                .await
                .0
                .tripped(day(19, 10))
                .await,
            None
        );
        let (b, r) = br(FakePnl::set(-2000, false)).await;
        assert_eq!(b.tripped(day(19, 10)).await, Some(true));
        assert_eq!(r.rows.lock().unwrap().len(), 1);
        assert_eq!(r.rows.lock().unwrap()[0].draft.actor, "breaker");
    }

    #[tokio::test]
    async fn latches_through_recovery_and_clears_next_trading_day() {
        let (b, r) = br(FakePnl::set(-3000, false)).await;
        assert_eq!(b.tripped(day(20, 10)).await, Some(true)); // Friday
        *b.src.0.lock().unwrap() = Some(DayPnl {
            pnl: Decimal::ZERO,
            equity_base: Decimal::from(100_000),
        });
        assert_eq!(b.tripped(day(20, 14)).await, Some(false));
        assert_eq!(b.tripped(day(21, 10)).await, Some(false)); // weekend
        assert_eq!(b.tripped(day(23, 10)).await, None); // Monday
        assert_eq!(r.rows.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn source_error_and_bad_equity_trip() {
        let (b, _) = br(FakePnl::set(0, false)).await;
        *b.src.0.lock().unwrap() = None;
        assert_eq!(b.tripped(day(19, 10)).await, Some(true));
        let (b, _) = br(FakePnl::set(0, false)).await;
        *b.src.0.lock().unwrap() = Some(DayPnl {
            pnl: Decimal::ZERO,
            equity_base: Decimal::ZERO,
        });
        assert_eq!(b.tripped(day(19, 10)).await, Some(true));
    }

    #[tokio::test]
    async fn audit_failure_keeps_it_tripped_and_retries_until_it_lands() {
        let (b, r) = br(FakePnl::set(-3000, false)).await;
        r.fail_write.store(true, SeqCst);
        assert_eq!(b.tripped(day(19, 10)).await, Some(true));
        assert!(b.audit_pending());
        assert_eq!(b.tripped(day(19, 11)).await, Some(false));
        assert!(b.audit_pending());
        assert!(r.rows.lock().unwrap().is_empty());
        r.fail_write.store(false, SeqCst);
        assert_eq!(b.tripped(day(19, 12)).await, Some(false));
        assert!(!b.audit_pending());
        assert_eq!(r.rows.lock().unwrap().len(), 1);
        assert_eq!(b.tripped(day(19, 13)).await, Some(false));
        assert_eq!(r.rows.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn latch_survives_a_restart_over_the_same_repo() {
        let (b, r) = br(FakePnl::set(-3000, false)).await;
        assert_eq!(b.tripped(day(20, 10)).await, Some(true)); // Friday
        let b2 = breaker_on(FakePnl::set(0, false), &r, day(20, 15)).await; // P&L recovered
        assert_eq!(b2.tripped(day(20, 15)).await, Some(false));
        assert_eq!(b2.tripped(day(21, 10)).await, Some(false));
        assert_eq!(b2.tripped(day(23, 10)).await, None);
        assert_eq!(r.rows.lock().unwrap().len(), 1);
    }

    #[allow(clippy::unused_async_trait_impl)]
    #[tokio::test]
    async fn unreadable_audit_log_refuses_to_build() {
        use bazaar_core::ports::{AuditRow, ChainFn, RepoError};
        struct Down;
        impl AuditRepo for Down {
            async fn append(&self, _: AuditDraft, _: ChainFn) -> Result<AuditRow, RepoError> {
                Err(RepoError::Unavailable)
            }
            async fn read_all(&self) -> Result<Vec<AuditRow>, RepoError> {
                Err(RepoError::Unavailable)
            }
        }
        let r = DailyLossBreaker::new(
            FakePnl::set(0, false),
            Arc::new(Down),
            cal(),
            Decimal::from(2),
            Decimal::from(3),
            day(19, 9),
        )
        .await;
        assert_eq!(r.err(), Some(ExecError::Storage));
    }

    #[tokio::test]
    async fn pct_must_be_positive_and_within_the_hard_ceiling() {
        for pct in [Decimal::ZERO, Decimal::from(-1), Decimal::new(301, 2)] {
            let r = DailyLossBreaker::new(
                FakePnl::set(0, false),
                Arc::new(MemRepo::default()),
                cal(),
                pct,
                Decimal::from(3),
                day(19, 9),
            )
            .await;
            assert!(matches!(r, Err(ExecError::InvalidInput(_))), "{pct}");
        }
    }

    #[tokio::test]
    async fn calendar_error_trip_is_rederived_on_restart_not_latched_forever() {
        let r = Arc::new(MemRepo::default());
        let payload = serde_json::json!({"day": "2026-11-19", "until": "max"}).to_string();
        r.rows.lock().unwrap().push(bazaar_core::ports::AuditRow {
            seq: 1,
            draft: AuditDraft {
                ts_ist_rfc3339: day(19, 10).to_rfc3339(),
                actor: "breaker".into(),
                kind: TRIP_KIND.into(),
                payload,
            },
            prev_hash: [0; 32],
            hash: [0; 32],
        });
        // Same day: still tripped. Next trading day: clear.
        let b = breaker_on(FakePnl::set(0, false), &r, day(19, 15)).await;
        assert_eq!(b.tripped(day(19, 15)).await, Some(false));
        let b = breaker_on(FakePnl::set(0, false), &r, day(20, 10)).await;
        assert_eq!(b.tripped(day(20, 10)).await, None);
    }

    #[test]
    fn restore_latch_takes_the_latest_open_trip_and_counts_unreadable_rows() {
        let row = |kind: &str, payload: &str| AuditRow {
            seq: 1,
            draft: AuditDraft {
                ts_ist_rfc3339: day(19, 10).to_rfc3339(),
                actor: "breaker".into(),
                kind: kind.into(),
                payload: payload.into(),
            },
            prev_hash: [0; 32],
            hash: [0; 32],
        };
        let rows = vec![
            row(TRIP_KIND, r#"{"until":"2026-11-23"}"#),
            row(TRIP_KIND, r#"{"until":"2026-11-20"}"#),
            row("other", "garbage"),
        ];
        let today = day(19, 10).date_naive();
        let monday = day(23, 0).date_naive();
        let got = restore_latch(&rows, &cal(), today);
        assert_eq!((got.until, got.unparsable), (Some(monday), 0));
        // The latest wins, not the first read: the same rows oldest first (TS-179-1).
        let reversed: Vec<_> = rows.iter().rev().cloned().collect();
        let got = restore_latch(&reversed, &cal(), today);
        assert_eq!((got.until, got.unparsable), (Some(monday), 0));
        let expired = restore_latch(&rows, &cal(), monday);
        assert_eq!((expired.until, expired.unparsable), (None, 0));
        // An unreadable row does not stop the scan: a later open trip still latches.
        let mixed = [row(TRIP_KIND, "garbage"), rows[0].clone()];
        let got = restore_latch(&mixed, &cal(), today);
        assert_eq!((got.until, got.unparsable), (Some(monday), 1));
        // "max" on a day the calendar still cannot answer: readable, held one calendar day
        // (a Friday, so a working calendar would not give the Saturday).
        let uncovered = NaiveDate::from_ymd_opt(2035, 1, 5).unwrap();
        let next = cal().next_trading_day(uncovered, Segment::Equity);
        assert!(next.is_err(), "{next:?}");
        let held = row(TRIP_KIND, r#"{"until":"max","day":"2035-01-05"}"#);
        let got = restore_latch(&[held], &cal(), today);
        assert_eq!((got.until, got.unparsable), (uncovered.succ_opt(), 0));
        for bad in [
            "garbage",
            "{}",
            r#"{"until":7}"#,
            r#"{"until":"soon"}"#,
            r#"{"until":"max"}"#,
            r#"{"until":"max","day":"x"}"#,
        ] {
            let got = restore_latch(&[row(TRIP_KIND, bad)], &cal(), today);
            assert_eq!((got.until, got.unparsable), (None, 1), "{bad}");
        }
    }

    /// Collects every event's fields as one line each.
    struct Events(Arc<Mutex<Vec<String>>>);
    impl tracing::Subscriber for Events {
        fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            use std::fmt::Write as _;
            struct Line(String);
            impl tracing::field::Visit for Line {
                fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
                    let _ = write!(self.0, "{}={v:?};", f.name());
                }
            }
            let mut line = Line(format!("{};", event.metadata().level()));
            event.record(&mut line);
            self.0.lock().unwrap().push(line.0);
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }

    #[tokio::test]
    async fn a_restart_warns_with_the_count_of_unreadable_trip_rows_only() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let _capture = tracing::subscriber::set_default(Events(seen.clone()));
        let r = Arc::new(MemRepo::default());
        breaker_on(FakePnl::set(0, false), &r, day(19, 9)).await;
        assert!(seen.lock().unwrap().is_empty());
        for payload in [r#"{"until":"secret-text"}"#, "{}"] {
            r.rows.lock().unwrap().push(AuditRow {
                seq: 1,
                draft: AuditDraft {
                    ts_ist_rfc3339: day(19, 8).to_rfc3339(),
                    actor: "breaker".into(),
                    kind: TRIP_KIND.into(),
                    payload: payload.into(),
                },
                prev_hash: [0; 32],
                hash: [0; 32],
            });
        }
        let b = breaker_on(FakePnl::set(0, false), &r, day(19, 9)).await;
        assert_eq!(
            *seen.lock().unwrap(),
            ["WARN;message=breaker latch restore ignored unreadable trip rows;unparsable=2;"]
        );
        // Behaviour is unchanged: unreadable rows do not latch (BZ-F10-S10, RISK-179-1).
        assert_eq!(b.tripped(day(19, 10)).await, None);
    }

    #[tokio::test]
    async fn trip_payload_carries_pnl_base_and_limit() {
        let (b, r) = br(FakePnl::set(-3000, false)).await;
        b.tripped(day(19, 10)).await;
        let p = r.rows.lock().unwrap()[0].draft.payload.clone();
        let v: serde_json::Value = serde_json::from_str(&p).unwrap();
        assert_eq!(v["pnl"], "-3000");
        assert_eq!(v["equity_base"], "100000");
        assert_eq!(v["max_loss_pct"], "2");
        assert_eq!(v["until"], "2026-11-20");
    }

    #[tokio::test]
    async fn concurrent_first_checks_write_one_audit_row() {
        let (b, r) = br(FakePnl::set(-3000, false)).await;
        r.delay_ms.store(20, SeqCst);
        let (x, y) = tokio::join!(b.tripped(day(19, 10)), b.tripped(day(19, 10)));
        let newly = [x, y].iter().filter(|v| **v == Some(true)).count();
        assert_eq!(newly, 1, "exactly one newly-tripped");
        assert_eq!(r.rows.lock().unwrap().len(), 1);
    }

    #[allow(clippy::panic)]
    #[tokio::test]
    async fn poisoned_latch_lock_counts_as_tripped() {
        let (b, _) = br(FakePnl::set(0, false)).await;
        let b = Arc::new(b);
        let b2 = b.clone();
        let _ = std::thread::spawn(move || {
            let _g = b2.until.lock().unwrap();
            panic!("poison");
        })
        .join();
        assert_eq!(b.tripped(day(19, 10)).await, Some(false));
    }
}
