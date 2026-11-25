//! [`LedgerResetGuard`]: the one real [`ResetGuard`] (F10 slice 9e). NOT wired: production still
//! uses `RefuseAll`; the stopped-server reset command is BZ-F10-S10.
//!
//! A reset passes only if the breaker latch is clear, the net ledger P&L of today and of this
//! ISO week (IST, all epochs) is not negative, the executor book is flat with no working order
//! and the current epoch has no buy or sell line. Anything unreadable is `GuardUnavailable`.
//! Precedence: `GuardUnavailable`, `BreakerLatched`, `DailyLoss`, `WeeklyLoss`, `BookNotFlat`,
//! `OpenLots`.
//!
//! The P&L is the signed sum of `charge`, `buy` and `sell` lines (never `opening`): cash flow,
//! equal to realised P&L only when flat at both ends of the window, so `OpenLots` must pass too.
//!
//! Limits: the latch is read from the audit log only, so a live trip whose row is still unwritten
//! (`audit_pending`) is not seen; the ledger scan and the audit read grow with their logs, and
//! once they outrun the deadline every reset is refused.

use std::sync::Arc;
use std::time::Duration;

use bazaar_core::calendar::MarketCalendar;
use bazaar_core::clock::Clock;
use bazaar_core::ports::{AuditRepo, LedgerEntry, PaperLedgerRepo, RepoError};
use bazaar_execution::breaker::restore_latch;
use chrono::{DateTime, Datelike, Days, NaiveDate};
use rust_decimal::Decimal;

use crate::account::{LedgerKind, ResetGuard, ResetRule};

/// What the executor holds right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookState {
    /// No position and no working order.
    Flat,
    /// A position or a working order exists.
    NotFlat,
    /// Not knowable now: refuses.
    Unknown,
}

/// Port: the executor book, as the reset guard sees it.
pub trait BookProvider: Send + Sync {
    /// Positions AND working orders.
    ///
    /// Contract: never block the thread, and never wait on the paper executor's `driver` lock
    /// without a timeout of your own; a busy lock is [`BookState::Unknown`]. `PaperAccount::reset`
    /// holds the account write lock while the guard runs, and `PaperExecutor::accept` holds
    /// `driver` while it waits for that account lock, so an unbounded wait here deadlocks until
    /// the guard's deadline.
    fn book_state(&self) -> impl Future<Output = BookState> + Send;
}

/// The real reset guard. Every input is required; there is no default.
pub struct LedgerResetGuard<L, A, B> {
    ledger: L,
    audit: A,
    calendar: MarketCalendar,
    clock: Arc<dyn Clock>,
    timeout: Duration,
    book: B,
}

impl<L, A, B> std::fmt::Debug for LedgerResetGuard<L, A, B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LedgerResetGuard").finish_non_exhaustive()
    }
}

/// Logs the category only (never row contents) and refuses.
fn unavailable(cause: &'static str) -> ResetRule {
    tracing::warn!(cause, "paper reset guard: input unavailable; refusing");
    ResetRule::GuardUnavailable
}

fn read_failed(source: &'static str, e: RepoError) -> ResetRule {
    tracing::warn!(source, category = e.safe_reason(), "paper reset guard read");
    ResetRule::GuardUnavailable
}

/// The kind of a line; an unknown kind refuses.
fn kind_of(line: &LedgerEntry) -> Result<LedgerKind, ResetRule> {
    LedgerKind::parse(&line.kind).ok_or_else(|| unavailable("line kind"))
}

/// Net (today, this week) cash flow of the trade and charge lines. Every line must be readable
/// and none may be dated after `today` (a clock that stepped back, or foreign data).
fn net_pnl(
    lines: &[LedgerEntry],
    today: NaiveDate,
    monday: NaiveDate,
) -> Result<(Decimal, Decimal), ResetRule> {
    let (mut day, mut week) = (Decimal::ZERO, Decimal::ZERO);
    for line in lines {
        let kind = kind_of(line)?;
        let ts = &line.ts_ist_rfc3339;
        let date = DateTime::parse_from_rfc3339(ts)
            .ok()
            .filter(|_| ts.ends_with("+05:30"))
            .ok_or_else(|| unavailable("line timestamp"))?
            .date_naive();
        if date > today {
            return Err(unavailable("line timestamp"));
        }
        if kind == LedgerKind::Opening {
            continue;
        }
        let add = |sum: Decimal| {
            sum.checked_add(line.amount)
                .ok_or_else(|| unavailable("overflow"))
        };
        if date >= monday {
            week = add(week)?;
        }
        if date >= today {
            day = add(day)?;
        }
    }
    Ok((day, week))
}

/// True when a buy or sell line exists.
fn has_trade_line(lines: &[LedgerEntry]) -> Result<bool, ResetRule> {
    let mut found = false;
    for line in lines {
        found |= matches!(kind_of(line)?, LedgerKind::Buy | LedgerKind::Sell);
    }
    Ok(found)
}

impl<L: PaperLedgerRepo, A: AuditRepo, B: BookProvider> LedgerResetGuard<L, A, B> {
    /// `ledger` and `audit` must be the stores the account and the breaker write to; `timeout`
    /// is `[execution] io_timeout_secs` and bounds the whole check. The audit chain is not
    /// verified here: the caller opens the store with `open_anchored`, as the server does.
    pub fn new(
        ledger: L,
        audit: A,
        calendar: MarketCalendar,
        clock: Arc<dyn Clock>,
        timeout: Duration,
        book: B,
    ) -> Self {
        Self {
            ledger,
            audit,
            calendar,
            clock,
            timeout,
            book,
        }
    }

    /// Reads everything, then judges: an unreadable input outranks every rule.
    async fn judge(&self) -> Result<u64, ResetRule> {
        let today = self.clock.now_ist().date_naive();
        let monday = today
            .checked_sub_days(Days::new(today.weekday().num_days_from_monday().into()))
            .ok_or_else(|| unavailable("week start"))?;
        // FIRST read: its tail is what the commit is checked against, so a ledger line landing
        // after it, during any later read, turns the commit into a `Conflict`.
        let since = self
            .ledger
            .lines_since(monday)
            .await
            .map_err(|e| read_failed("ledger_since", e))?;
        let rows = self
            .audit
            .read_all()
            .await
            .map_err(|e| read_failed("audit", e))?;
        let book = self.book.book_state().await;
        let current = self
            .ledger
            .current_epoch()
            .await
            .map_err(|e| read_failed("current_epoch", e))?;

        let (day, week) = net_pnl(&since.lines, today, monday)?;
        let open_lots = has_trade_line(&current)?;
        let latch = restore_latch(&rows, &self.calendar, today);
        if latch.unparsable > 0 {
            return Err(unavailable("breaker trip row"));
        }
        let flat = match book {
            BookState::Flat => true,
            BookState::NotFlat => false,
            BookState::Unknown => return Err(unavailable("book")),
        };
        // In precedence order.
        let rules = [
            (latch.until.is_some(), ResetRule::BreakerLatched),
            (day < Decimal::ZERO, ResetRule::DailyLoss),
            (week < Decimal::ZERO, ResetRule::WeeklyLoss),
            (!flat, ResetRule::BookNotFlat),
            (open_lots, ResetRule::OpenLots),
        ];
        match rules.iter().find(|(refuses, _)| *refuses) {
            Some((_, rule)) => Err(*rule),
            None => Ok(since.tail),
        }
    }
}

impl<L: PaperLedgerRepo, A: AuditRepo, B: BookProvider> ResetGuard for LedgerResetGuard<L, A, B> {
    /// One deadline covers every read and the book query. On expiry the pending read is
    /// dropped and the answer is `GuardUnavailable`.
    async fn check(&self) -> Result<u64, ResetRule> {
        tokio::time::timeout(self.timeout, self.judge())
            .await
            .unwrap_or_else(|_| Err(unavailable("deadline")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use bazaar_core::config::CalendarConfig;
    use bazaar_core::ports::{AuditDraft, AuditRow, ChainFn, LedgerSince, SourceError};
    use bazaar_core::testutil::FakeClock;
    use bazaar_execution::breaker::{DailyLossBreaker, DayPnl, PnlSource};
    use chrono::TimeZone;
    use chrono_tz::Asia::Kolkata;

    use crate::account::{AccountError, ChargeSource, PaperAccount, ResetReason, ResetRequest};

    const MON: &str = "2026-11-23";
    const TUE: &str = "2026-11-24";
    const FRI: &str = "2026-11-20";

    /// Ledger, audit log and book in memory. Every read is logged and can fail, hang, or have a
    /// ledger line land while it runs.
    #[derive(Default)]
    struct Fake {
        lines: Mutex<Vec<LedgerEntry>>,
        audits: Mutex<Vec<AuditRow>>,
        calls: Mutex<Vec<&'static str>>,
        fail: Mutex<Option<&'static str>>,
        hang: Mutex<Option<&'static str>>,
        slow: Mutex<Vec<(&'static str, Duration)>>,
        append_during: Mutex<Option<&'static str>>,
        book: Mutex<Option<BookState>>,
    }

    impl Fake {
        async fn read(&self, call: &'static str) -> Result<(), RepoError> {
            self.calls.lock().unwrap().push(call);
            if *self.append_during.lock().unwrap() == Some(call) {
                self.push(1, "charge", "1", TUE);
            }
            let slow = self
                .slow
                .lock()
                .unwrap()
                .iter()
                .find(|s| s.0 == call)
                .copied();
            if let Some((_, delay)) = slow {
                tokio::time::sleep(delay).await;
            }
            let hangs = *self.hang.lock().unwrap() == Some(call);
            if hangs {
                std::future::pending::<()>().await;
            }
            if *self.fail.lock().unwrap() == Some(call) {
                return Err(RepoError::TooLarge);
            }
            Ok(())
        }
        fn push(&self, epoch: u32, kind: &str, amount: &str, date: &str) {
            self.push_at(epoch, kind, amount, &format!("{date}T10:00:00+05:30"));
        }
        fn push_at(&self, epoch: u32, kind: &str, amount: &str, ts: &str) {
            self.lines.lock().unwrap().push(LedgerEntry {
                epoch,
                kind: kind.into(),
                amount: amount.parse().unwrap(),
                ref_id: None,
                ts_ist_rfc3339: ts.into(),
            });
        }
        fn trip_row(&self, payload: &str) {
            self.audits.lock().unwrap().push(AuditRow {
                seq: 1,
                draft: AuditDraft {
                    ts_ist_rfc3339: format!("{TUE}T10:00:00+05:30"),
                    actor: "breaker".into(),
                    kind: "breaker_tripped".into(),
                    payload: payload.into(),
                },
                prev_hash: [0; 32],
                hash: [0; 32],
            });
        }
        /// The raw stores, without going through a read that a fault is injected into.
        fn snapshot(&self) -> (Vec<LedgerEntry>, Vec<AuditRow>) {
            let lines = self.lines.lock().unwrap().clone();
            (lines, self.audits.lock().unwrap().clone())
        }
    }

    /// The guard's deadline in these tests.
    const TIMEOUT: Duration = Duration::from_millis(100);

    #[derive(Clone)]
    struct Shared(Arc<Fake>);

    impl PaperLedgerRepo for Shared {
        async fn append(&self, e: LedgerEntry) -> Result<(), RepoError> {
            self.append_many(vec![e]).await
        }
        async fn append_many(&self, es: Vec<LedgerEntry>) -> Result<(), RepoError> {
            self.0.lines.lock().unwrap().extend(es);
            Ok(())
        }
        async fn current_epoch(&self) -> Result<Vec<LedgerEntry>, RepoError> {
            // `PaperAccount::reset` replays the epoch before it asks the guard; faults hit only
            // the guard's read, the one that follows `lines_since`.
            let guards_read = self.0.calls.lock().unwrap().contains(&"lines_since");
            if guards_read {
                self.0.read("current_epoch").await?;
            }
            let lines = self.0.lines.lock().unwrap();
            let max = lines.iter().map(|l| l.epoch).max();
            Ok(lines
                .iter()
                .filter(|l| Some(l.epoch) == max)
                .cloned()
                .collect())
        }
        async fn lines_since(&self, day: NaiveDate) -> Result<LedgerSince, RepoError> {
            // Rows and tail are one snapshot, taken before anything lands "during" the read.
            let mut lines = self.0.lines.lock().unwrap().clone();
            self.0.read("lines_since").await?;
            let tail = lines.len() as u64;
            // As the store: only a well-formed `+05:30` row dated before `day` is dropped.
            lines.retain(|l| {
                let ts = &l.ts_ist_rfc3339;
                !(ts.ends_with("+05:30")
                    && DateTime::parse_from_rfc3339(ts).is_ok_and(|t| t.date_naive() < day))
            });
            Ok(LedgerSince { lines, tail })
        }
        async fn new_epoch(
            &self,
            opening: LedgerEntry,
            expected_tail: u64,
            audit: AuditDraft,
            chain: ChainFn,
        ) -> Result<u32, RepoError> {
            if expected_tail != self.0.lines.lock().unwrap().len() as u64 {
                return Err(RepoError::Conflict);
            }
            let epoch = opening.epoch;
            self.0.lines.lock().unwrap().push(opening);
            AuditRepo::append(self, audit, chain).await?;
            Ok(epoch)
        }
    }

    impl AuditRepo for Shared {
        async fn append(&self, draft: AuditDraft, _: ChainFn) -> Result<AuditRow, RepoError> {
            let mut g = self.0.audits.lock().unwrap();
            let row = AuditRow {
                seq: g.len() as u64 + 1,
                draft,
                prev_hash: [0; 32],
                hash: [0; 32],
            };
            g.push(row.clone());
            Ok(row)
        }
        async fn read_all(&self) -> Result<Vec<AuditRow>, RepoError> {
            self.0.read("audit").await?;
            Ok(self.0.audits.lock().unwrap().clone())
        }
    }

    impl BookProvider for Shared {
        async fn book_state(&self) -> BookState {
            // A failing provider answers Unknown.
            if self.0.read("book").await.is_err() {
                return BookState::Unknown;
            }
            self.0.book.lock().unwrap().unwrap_or(BookState::Flat)
        }
    }

    struct NoCharge;
    impl ChargeSource for NoCharge {
        fn charge(
            &self,
            _: &str,
            _: bool,
            _: u32,
            _: Decimal,
            _: &str,
        ) -> Result<Decimal, SourceError> {
            Err(SourceError::Unavailable)
        }
    }

    fn calendar() -> MarketCalendar {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config");
        MarketCalendar::from_config(&bazaar_core::config::load::<CalendarConfig>(&dir).unwrap())
    }

    struct Rig {
        fake: Arc<Fake>,
        clock: Arc<FakeClock>,
    }

    impl Rig {
        /// An opened account (epoch 1, opened last Friday); now is Tuesday 15:00 IST.
        fn new() -> Self {
            let fake = Arc::new(Fake::default());
            fake.push(1, "opening", "100000", FRI);
            let now = Kolkata.with_ymd_and_hms(2026, 11, 24, 15, 0, 0).unwrap();
            Self {
                fake,
                clock: Arc::new(FakeClock::at(now)),
            }
        }
        fn at(&self, day: u32, hour: u32) {
            let t = Kolkata.with_ymd_and_hms(2026, 11, day, hour, 0, 0);
            self.clock.set(t.unwrap());
        }
        fn guard(&self) -> LedgerResetGuard<Shared, Shared, Shared> {
            let s = Shared(self.fake.clone());
            LedgerResetGuard::new(
                s.clone(),
                s.clone(),
                calendar(),
                self.clock.clone(),
                TIMEOUT,
                s,
            )
        }
        async fn reset(&self) -> Result<u32, AccountError> {
            let account = PaperAccount::new(
                Shared(self.fake.clone()),
                NoCharge,
                self.clock.clone(),
                Decimal::from(100_000),
            );
            let req = ResetRequest {
                reason: ResetReason::OwnerRequested,
                actor: "owner".into(),
            };
            let guard = self.guard();
            self.fake.calls.lock().unwrap().clear();
            // Never hangs, whatever the fake does.
            let done = tokio::time::timeout(Duration::from_secs(5), account.reset(&req, &guard));
            done.await.unwrap()
        }
        /// The guard and the account refuse with exactly `rule`; nothing is written.
        async fn refuses(&self, rule: ResetRule) {
            assert_eq!(self.guard().check().await, Err(rule));
            let before = self.fake.snapshot();
            assert_eq!(self.reset().await, Err(AccountError::ResetRefused(rule)));
            assert_eq!(self.fake.snapshot(), before);
        }
        async fn passes(&self) {
            let tail = self.fake.lines.lock().unwrap().len() as u64;
            assert_eq!(self.guard().check().await, Ok(tail));
        }
    }

    /// RISK-176-1: the week is up but today lost.
    #[tokio::test]
    async fn a_losing_day_in_a_winning_week_is_daily_loss() {
        let r = Rig::new();
        r.fake.push(1, "charge", "5000", MON);
        r.fake.push(1, "charge", "-3000", TUE);
        r.refuses(ResetRule::DailyLoss).await;
    }

    #[tokio::test]
    async fn a_winning_day_in_a_losing_week_is_weekly_loss() {
        let r = Rig::new();
        r.fake.push(1, "charge", "-5000", MON);
        r.fake.push(1, "charge", "3000", TUE);
        r.refuses(ResetRule::WeeklyLoss).await;
    }

    #[tokio::test]
    async fn todays_loss_is_summed_across_epochs_and_zero_passes() {
        let r = Rig::new();
        r.fake.push(1, "charge", "-600", TUE);
        r.fake.push(2, "opening", "100000", TUE);
        r.fake.push(2, "charge", "500", TUE);
        // The current epoch alone is up Rs 500.
        r.refuses(ResetRule::DailyLoss).await;
        r.fake.push(2, "charge", "100", TUE);
        r.passes().await;
    }

    #[tokio::test]
    async fn opening_lines_are_not_profit() {
        let r = Rig::new();
        r.fake.push(1, "charge", "-1", TUE);
        r.fake.push(2, "opening", "100000", TUE);
        r.refuses(ResetRule::DailyLoss).await;
    }

    #[tokio::test]
    async fn a_friday_loss_blocks_saturday_and_clears_next_monday() {
        let r = Rig::new();
        r.fake.push(1, "charge", "-10", FRI);
        r.fake.push(2, "opening", "100000", FRI);
        r.at(21, 10); // Saturday, same ISO week
        r.refuses(ResetRule::WeeklyLoss).await;
        r.at(22, 23); // Sunday still belongs to it
        r.refuses(ResetRule::WeeklyLoss).await;
        r.at(23, 0); // Monday
        r.passes().await;
        assert_eq!(r.reset().await, Ok(3));
    }

    /// 01:00 IST is still the previous date in UTC; day and week follow the IST clock.
    #[tokio::test]
    async fn day_and_week_are_ist_dates_where_the_utc_date_differs() {
        let r = Rig::new();
        r.fake.push(1, "charge", "50", MON);
        r.fake
            .push_at(1, "charge", "-10", "2026-11-24T00:30:00+05:30");
        r.at(24, 1);
        // A UTC "today" (Monday) would net +40 for the day and pass.
        r.refuses(ResetRule::DailyLoss).await;
        let r = Rig::new();
        r.fake.push(1, "charge", "-10", FRI);
        r.at(23, 1); // UTC: Sunday, still last week
        r.passes().await;
    }

    #[tokio::test]
    async fn a_latched_breaker_refuses_after_restart_and_a_reset_keeps_its_rows() {
        struct Losing;
        impl PnlSource for Losing {
            async fn day_pnl(&self) -> Result<DayPnl, SourceError> {
                Ok(DayPnl {
                    pnl: Decimal::from(-3000),
                    equity_base: Decimal::from(100_000),
                })
            }
        }
        let r = Rig::new();
        let breaker = || {
            let audit = Arc::new(Shared(r.fake.clone()));
            let pct = Decimal::from(2);
            DailyLossBreaker::new(Losing, audit, calendar(), pct, pct, r.clock.now_ist())
        };
        let live = breaker().await.unwrap();
        assert_eq!(live.tripped(r.clock.now_ist()).await, Some(true));
        let trips = r.fake.snapshot().1;
        // A new guard (a restart) has only the audit rows.
        r.refuses(ResetRule::BreakerLatched).await;
        let restarted = breaker().await.unwrap();
        assert_eq!(restarted.tripped(r.clock.now_ist()).await, Some(false));
        // Once the latch has expired a reset goes through and leaves the trip rows unchanged.
        r.at(25, 10);
        assert_eq!(r.reset().await, Ok(2));
        let after = r.fake.snapshot().1;
        assert_eq!(after.len(), trips.len() + 1);
        assert_eq!(after[..trips.len()], trips[..]);
        assert_eq!(after[trips.len()].draft.kind, "paper_reset");
    }

    #[tokio::test]
    async fn every_failing_read_is_guard_unavailable() {
        for call in ["lines_since", "audit", "book", "current_epoch"] {
            let r = Rig::new();
            r.passes().await;
            *r.fake.fail.lock().unwrap() = Some(call);
            r.refuses(ResetRule::GuardUnavailable).await;
        }
    }

    /// Includes a hung ledger read and a book provider that never answers.
    #[tokio::test(start_paused = true)]
    async fn every_hung_read_is_guard_unavailable_within_the_deadline() {
        for call in ["lines_since", "audit", "book", "current_epoch"] {
            let r = Rig::new();
            r.passes().await;
            *r.fake.hang.lock().unwrap() = Some(call);
            let started = tokio::time::Instant::now();
            let got = tokio::time::timeout(Duration::from_secs(5), r.guard().check()).await;
            assert_eq!(got, Ok(Err(ResetRule::GuardUnavailable)), "{call}");
            assert_eq!(started.elapsed(), TIMEOUT, "{call}");
            r.refuses(ResetRule::GuardUnavailable).await;
        }
    }

    /// No single read is over the deadline; two together are.
    #[tokio::test(start_paused = true)]
    async fn one_deadline_covers_all_reads_together() {
        let calls = ["lines_since", "audit", "book", "current_epoch"];
        let delay = TIMEOUT * 6 / 10;
        for pair in calls.windows(2) {
            let r = Rig::new();
            *r.fake.slow.lock().unwrap() = vec![(pair[0], delay)];
            r.passes().await;
            r.fake.slow.lock().unwrap().push((pair[1], delay));
            let started = tokio::time::Instant::now();
            r.refuses(ResetRule::GuardUnavailable).await;
            // `refuses` runs the guard twice: alone, then inside the reset.
            assert_eq!(started.elapsed(), TIMEOUT * 2, "{pair:?}");
        }
    }

    #[tokio::test]
    async fn a_line_dated_after_today_is_guard_unavailable() {
        // A profit dated tomorrow would offset today's loss.
        let r = Rig::new();
        r.fake.push(1, "charge", "-10", TUE);
        r.fake.push(1, "charge", "20", "2026-11-25");
        r.refuses(ResetRule::GuardUnavailable).await;
        // Any kind, in an older epoch too, from the first instant of the next IST day.
        for kind in ["opening", "charge", "buy", "sell"] {
            let r = Rig::new();
            r.fake.push_at(1, kind, "1", "2026-11-25T00:00:00+05:30");
            r.fake.push(2, "opening", "100000", FRI);
            r.refuses(ResetRule::GuardUnavailable).await;
        }
        // The last second of today is not in the future, even though the clock reads 15:00.
        let r = Rig::new();
        r.fake
            .push_at(1, "charge", "1", "2026-11-24T23:59:59+05:30");
        r.passes().await;
    }

    #[tokio::test]
    async fn a_bad_line_is_guard_unavailable_whatever_its_age_or_kind() {
        let ok_ts = "2026-11-24T10:00:00+05:30";
        let cases = [
            ("dividend", ok_ts),
            ("charge", "garbage"),
            ("charge", "2026-11-24T04:30:00+00:00"),
            ("charge", "2026-02-30T10:00:00+05:30"),
            ("charge", "2020-01-01T10:00:00Z"),
            ("opening", "garbage"),
        ];
        for (kind, ts) in cases {
            let r = Rig::new();
            r.fake.push_at(1, kind, "1", ts);
            // An older epoch: only the guard's cross-epoch read sees the line.
            r.fake.push(2, "opening", "100000", FRI);
            r.refuses(ResetRule::GuardUnavailable).await;
        }
        // The current-epoch check refuses an unknown kind on its own too.
        let r = Rig::new();
        r.fake.push(1, "dividend", "1", FRI);
        let lines = r.fake.snapshot().0;
        assert_eq!(has_trade_line(&lines), Err(ResetRule::GuardUnavailable));
    }

    #[tokio::test]
    async fn overflow_a_bad_trip_row_and_an_unknown_book_are_guard_unavailable() {
        let r = Rig::new();
        let max = Decimal::MAX.to_string();
        r.fake.push(1, "charge", &max, TUE);
        r.fake.push(2, "opening", "100000", TUE);
        r.passes().await;
        r.fake.push(1, "charge", &max, TUE);
        r.refuses(ResetRule::GuardUnavailable).await;
        let r = Rig::new();
        r.fake.trip_row("not json");
        r.refuses(ResetRule::GuardUnavailable).await;
        // Any unreadable trip row refuses, also beside a readable open one (not BreakerLatched).
        let r = Rig::new();
        r.fake.trip_row(r#"{"until":"2026-11-25"}"#);
        r.fake.trip_row(r#"{"until":"max"}"#);
        r.refuses(ResetRule::GuardUnavailable).await;
        let r = Rig::new();
        *r.fake.book.lock().unwrap() = Some(BookState::Unknown);
        r.refuses(ResetRule::GuardUnavailable).await;
    }

    #[tokio::test]
    async fn a_trade_line_is_open_lots_even_in_profit() {
        for kind in ["buy", "sell"] {
            let r = Rig::new();
            r.fake.push(1, kind, "250", TUE);
            r.refuses(ResetRule::OpenLots).await;
        }
        // Last week's trade in the current epoch counts; one in an older epoch does not.
        let r = Rig::new();
        r.fake.push(1, "buy", "-1", FRI);
        r.at(23, 10);
        r.refuses(ResetRule::OpenLots).await;
        r.fake.push(2, "opening", "100000", FRI);
        r.passes().await;
    }

    #[tokio::test]
    async fn a_position_or_working_order_is_book_not_flat() {
        let r = Rig::new();
        *r.fake.book.lock().unwrap() = Some(BookState::NotFlat);
        r.refuses(ResetRule::BookNotFlat).await;
    }

    /// Every rule applies at once; clearing one cause at a time shows the order.
    #[tokio::test]
    async fn precedence_when_several_rules_apply() {
        let r = Rig::new();
        r.fake.push(1, "buy", "-10", TUE);
        r.fake.trip_row(r#"{"until":"2026-11-25"}"#);
        *r.fake.book.lock().unwrap() = Some(BookState::Unknown);
        r.refuses(ResetRule::GuardUnavailable).await;
        *r.fake.book.lock().unwrap() = Some(BookState::NotFlat);
        r.refuses(ResetRule::BreakerLatched).await;
        r.fake.audits.lock().unwrap().clear();
        r.refuses(ResetRule::DailyLoss).await;
        r.fake.push(1, "charge", "10", TUE);
        r.fake.push(1, "charge", "-1", MON);
        r.refuses(ResetRule::WeeklyLoss).await;
        r.fake.push(1, "charge", "1", MON);
        r.refuses(ResetRule::BookNotFlat).await;
        *r.fake.book.lock().unwrap() = None;
        r.refuses(ResetRule::OpenLots).await;
    }

    #[tokio::test]
    async fn the_tail_is_read_first_and_a_later_line_makes_the_commit_conflict() {
        let r = Rig::new();
        r.passes().await;
        let order = ["lines_since", "audit", "book", "current_epoch"];
        assert_eq!(*r.fake.calls.lock().unwrap(), order);
        for during in ["audit", "book", "current_epoch"] {
            let r = Rig::new();
            *r.fake.append_during.lock().unwrap() = Some(during);
            // The returned tail is the first read's, not a later one.
            assert_eq!(r.guard().check().await, Ok(1), "{during}");
            let (lines, audits) = r.fake.snapshot();
            assert_eq!(lines.len(), 2);
            let got = r.reset().await;
            let conflict = AccountError::Ledger(RepoError::Conflict);
            assert_eq!(got, Err(conflict), "{during}");
            // Only the racing lines landed: no opening line, no audit row.
            let now = r.fake.snapshot();
            assert_eq!(now.0.len(), 3);
            assert!(now.0[1..].iter().all(|l| l.kind == "charge"));
            assert_eq!(now.1, audits);
        }
    }
}
