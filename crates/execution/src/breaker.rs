//! Daily-loss breaker. Trips when the day's P&L reaches `-(equity_base * max_daily_loss_pct / 100)`
//! and stays tripped until the next trading day, even if P&L recovers. An unreadable P&L source,
//! a non-positive equity base or a calendar error counts as tripped (fail closed).
//!
//! The latch is in-process (lost on restart; the trip audit row and, when configured, the kill
//! switch are what persist). The trip is latched before its audit row is written, so an audit
//! failure cannot un-trip it.

use std::future::Future;
use std::sync::{Arc, Mutex};

use bazaar_core::calendar::MarketCalendar;
use bazaar_core::ports::{AuditDraft, AuditRepo, SourceError};
use bazaar_core::segment::Segment;
use chrono::{DateTime, NaiveDate};
use chrono_tz::Tz;
use rust_decimal::Decimal;

use crate::audit::chain_hash;

/// Today's realised plus open P&L and the equity it is measured against.
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
}

impl<P: PnlSource, A: AuditRepo> std::fmt::Debug for DailyLossBreaker<P, A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DailyLossBreaker").finish_non_exhaustive()
    }
}

impl<P: PnlSource, A: AuditRepo> DailyLossBreaker<P, A> {
    /// `max_loss_pct` is `limits.max_daily_loss_pct`.
    pub const fn new(
        src: P,
        audit: Arc<A>,
        calendar: MarketCalendar,
        max_loss_pct: Decimal,
    ) -> Self {
        Self {
            src,
            audit,
            calendar,
            max_loss_pct,
            until: Mutex::new(None),
        }
    }

    fn latched(&self, today: NaiveDate) -> bool {
        self.until
            .lock()
            .map_or(true, |u| u.is_some_and(|d| today < d))
    }

    async fn over_limit(&self) -> bool {
        let Ok(p) = self.src.day_pnl().await else {
            return true;
        };
        let limit = p
            .equity_base
            .checked_mul(self.max_loss_pct)
            .and_then(|v| v.checked_div(Decimal::ONE_HUNDRED));
        match limit {
            Some(l) if p.equity_base > Decimal::ZERO => p.pnl <= -l,
            _ => true,
        }
    }

    /// `None` clear; `Some(newly)` tripped (`newly` on the check that tripped it).
    pub async fn tripped(&self, now: DateTime<Tz>) -> Option<bool> {
        let today = now.date_naive();
        if self.latched(today) {
            return Some(false);
        }
        if !self.over_limit().await {
            return None;
        }
        let until = self
            .calendar
            .next_trading_day(today, Segment::Equity)
            .unwrap_or(NaiveDate::MAX);
        if let Ok(mut u) = self.until.lock() {
            *u = Some(until);
        }
        let draft = AuditDraft {
            ts_ist_rfc3339: now.to_rfc3339(),
            actor: "breaker".to_owned(),
            kind: "breaker_tripped".to_owned(),
            payload: serde_json::json!({ "day": today.to_string() }).to_string(),
        };
        if let Err(e) = self.audit.append(draft, chain_hash).await {
            tracing::warn!(category = e.safe_reason(), "breaker trip audit failed");
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
}

#[cfg(test)]
mod tests {
    use super::fakes::*;
    use super::*;
    use crate::killswitch::fakes::MemRepo;
    use chrono::TimeZone;
    use chrono_tz::Asia::Kolkata;

    fn day(d: u32, h: u32) -> DateTime<Tz> {
        Kolkata.with_ymd_and_hms(2026, 11, d, h, 0, 0).unwrap()
    }

    // 2 % of 100000 = 2000.
    fn br(p: FakePnl) -> (DailyLossBreaker<FakePnl, MemRepo>, Arc<MemRepo>) {
        let r = Arc::new(MemRepo::default());
        (
            DailyLossBreaker::new(p, r.clone(), cal(), Decimal::from(2)),
            r,
        )
    }

    #[tokio::test]
    async fn boundary_one_paisa_inside_passes_equal_trips() {
        assert_eq!(
            br(FakePnl::set(-199_999, true))
                .0
                .tripped(day(19, 10))
                .await,
            None
        );
        let (b, r) = br(FakePnl::set(-2000, false));
        assert_eq!(b.tripped(day(19, 10)).await, Some(true));
        assert_eq!(r.rows.lock().unwrap().len(), 1);
        assert_eq!(r.rows.lock().unwrap()[0].draft.actor, "breaker");
    }

    #[tokio::test]
    async fn latches_through_recovery_and_clears_next_trading_day() {
        let (b, r) = br(FakePnl::set(-3000, false));
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
        let (b, _) = br(FakePnl::set(0, false));
        *b.src.0.lock().unwrap() = None;
        assert_eq!(b.tripped(day(19, 10)).await, Some(true));
        let (b, _) = br(FakePnl::set(0, false));
        *b.src.0.lock().unwrap() = Some(DayPnl {
            pnl: Decimal::ZERO,
            equity_base: Decimal::ZERO,
        });
        assert_eq!(b.tripped(day(19, 10)).await, Some(true));
    }

    #[tokio::test]
    async fn audit_failure_keeps_it_tripped() {
        let (b, r) = br(FakePnl::set(-3000, false));
        r.fail_write
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(b.tripped(day(19, 10)).await, Some(true));
        assert_eq!(b.tripped(day(19, 11)).await, Some(false));
    }
}
