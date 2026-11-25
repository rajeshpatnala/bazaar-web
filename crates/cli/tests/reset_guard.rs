//! BZ-F10-S9e: `LedgerResetGuard` over the real `SqliteStore` on a temp file (here because
//! `paper` may not depend on `store`; a file, because a connection sqlx discards after a dropped
//! future would come back as an empty database in memory). The store has ONE connection and the guard's first read holds it
//! for the whole scan, so a read dropped by the guard's deadline must give it back: the kill
//! switch has to engage afterwards (SEC-178-1, CR-178-1, RISK-178-1).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::future::{Future, poll_fn};
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

use bazaar_core::calendar::MarketCalendar;
use bazaar_core::config::{CalendarConfig, load};
use bazaar_core::ports::{
    AuditDraft, AuditRepo, KillSwitchRepo, LedgerEntry, PaperLedgerRepo, SourceError,
};
use bazaar_core::testutil::FakeClock;
use bazaar_execution::audit::{chain_hash, verify_chain};
use bazaar_paper::account::{
    AccountError, ChargeSource, PaperAccount, RefuseAll, ResetGuard, ResetReason, ResetRequest,
    ResetRule,
};
use bazaar_paper::reset_guard::{BookProvider, BookState, LedgerResetGuard};
use bazaar_store::SqliteStore;
use chrono::{NaiveDate, TimeZone};
use chrono_tz::Asia::Kolkata;
use rust_decimal::Decimal;
use tempfile::TempDir;

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

struct Flat;
impl BookProvider for Flat {
    async fn book_state(&self) -> BookState {
        std::future::ready(BookState::Flat).await
    }
}

const TS: &str = "2026-11-24T10:00:00+05:30";

fn clock() -> Arc<FakeClock> {
    let now = Kolkata.with_ymd_and_hms(2026, 11, 24, 15, 0, 0);
    Arc::new(FakeClock::at(now.unwrap()))
}

fn monday() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, 23).unwrap()
}

/// A reset through a `PaperAccount` over the store.
async fn reset<G: ResetGuard>(
    store: &SqliteStore,
    reason: ResetReason,
    guard: &G,
) -> Result<u32, AccountError> {
    let account = PaperAccount::new(store.clone(), NoCharge, clock(), Decimal::from(100_000));
    let request = ResetRequest {
        reason,
        actor: "owner".into(),
    };
    account.reset(&request, guard).await
}

fn guard(
    store: &SqliteStore,
    timeout: Duration,
) -> LedgerResetGuard<SqliteStore, SqliteStore, Flat> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../config");
    let cfg = load::<CalendarConfig>(std::path::Path::new(dir)).unwrap();
    LedgerResetGuard::new(
        store.clone(),
        store.clone(),
        MarketCalendar::from_config(&cfg),
        clock(),
        timeout,
        Flat,
    )
}

fn line(kind: &str, amount: i64) -> LedgerEntry {
    LedgerEntry {
        epoch: 1,
        kind: kind.into(),
        amount: Decimal::from(amount),
        ref_id: None,
        ts_ist_rfc3339: TS.into(),
    }
}

/// An opened account followed by `batches` x 1,000 charge lines of +1, all dated today.
async fn opened(batches: usize) -> (SqliteStore, TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("paper.db"))
        .await
        .unwrap();
    let first = reset(&store, ResetReason::Initial, &RefuseAll).await;
    assert_eq!(first, Ok(1));
    for _ in 0..batches {
        let batch = vec![line("charge", 1); 1_000];
        PaperLedgerRepo::append_many(&store, batch).await.unwrap();
    }
    (store, dir)
}

/// Ledger lines and tail, plus the audit rows.
async fn contents(store: &SqliteStore) -> (Vec<LedgerEntry>, u64, usize) {
    let since = store.lines_since(monday()).await.unwrap();
    (
        since.lines,
        since.tail,
        store.read_all().await.unwrap().len(),
    )
}

/// The kill switch engages and persists with its audit row, well inside the store's 10 s
/// acquire timeout.
async fn kill_switch_engages(store: &SqliteStore) {
    let rows = store.read_all().await.unwrap().len();
    let draft = AuditDraft {
        ts_ist_rfc3339: TS.into(),
        actor: "owner".into(),
        kind: "kill_switch_engaged".into(),
        payload: "{}".into(),
    };
    let set = KillSwitchRepo::set(store, true, "test", draft, chain_hash);
    let within = tokio::time::timeout(Duration::from_secs(5), set).await;
    assert_eq!(within.expect("connection not freed"), Ok(()));
    assert_eq!(store.is_engaged().await, Ok(true));
    let after = store.read_all().await.unwrap();
    assert_eq!(after.len(), rows + 1);
    assert_eq!(after[rows].draft.kind, "kill_switch_engaged");
    assert_eq!(verify_chain(&after), Ok(()));
}

/// A SECOND store on the same file sees the engaged switch and the same audit rows: they are
/// committed, not just visible inside a transaction on the writer's one connection.
async fn persisted(dir: &TempDir, store: &SqliteStore) {
    let rows = store.read_all().await.unwrap();
    let path = dir.path().join("paper.db");
    let open = tokio::time::timeout(Duration::from_secs(5), SqliteStore::open(&path));
    let other = open.await.expect("second open blocked").unwrap();
    assert_eq!(other.is_engaged().await, Ok(true));
    assert_eq!(other.read_all().await.unwrap(), rows);
    let last = rows.last().unwrap();
    assert_eq!(last.draft.kind, "kill_switch_engaged");
}

/// Drops `ledger_lines_since` while pending after 1, 2, 4, ... suspensions, until it finishes
/// first: the drops fall before and after its transaction begins and mid-scan.
#[tokio::test]
async fn dropping_a_pending_ledger_read_frees_the_only_connection() {
    let (store, dir) = opened(2).await;
    let before = contents(&store).await;
    let (mut stop_after, mut deepest) = (1_u32, 0);
    loop {
        let mut read = Box::pin(PaperLedgerRepo::lines_since(&store, monday()));
        let mut pending = 0;
        let still_pending = poll_fn(|cx| match read.as_mut().poll(cx) {
            Poll::Ready(_) => Poll::Ready(false),
            Poll::Pending => {
                pending += 1;
                if pending == stop_after {
                    Poll::Ready(true)
                } else {
                    Poll::Pending
                }
            }
        })
        .await;
        if !still_pending {
            break;
        }
        // While the read is alive (here: not polled) nothing else gets a connection.
        let probe = tokio::time::timeout(Duration::from_millis(100), store.is_engaged()).await;
        assert!(
            probe.is_err(),
            "read did not hold the connection at {stop_after}"
        );
        drop(read);
        kill_switch_engages(&store).await;
        let now = contents(&store).await;
        assert_eq!((now.0, now.1), (before.0.clone(), before.1));
        deepest = stop_after;
        stop_after *= 2;
    }
    persisted(&dir, &store).await;
    // Begin, tail and three pages: the deepest drop was inside the open transaction.
    assert!(deepest >= 4, "only reached {deepest} suspensions");
}

#[tokio::test]
async fn a_guard_deadline_refuses_and_leaves_the_store_usable() {
    let (store, dir) = opened(9).await;
    let before = contents(&store).await;
    // Every input is readable and passes, so only the deadline can refuse below.
    let long = guard(&store, Duration::from_secs(5));
    assert_eq!(long.check().await, Ok(9_001));
    let short = guard(&store, Duration::from_nanos(1));
    let got = tokio::time::timeout(Duration::from_secs(5), short.check()).await;
    assert_eq!(got, Ok(Err(ResetRule::GuardUnavailable)));
    kill_switch_engages(&store).await;
    let now = contents(&store).await;
    assert_eq!((now.0, now.1), (before.0, before.1));
    persisted(&dir, &store).await;
}

/// Pins the day the guard asks the store for: Monday's loss counts on Tuesday.
#[tokio::test]
async fn a_monday_loss_with_a_tuesday_gain_is_weekly_loss_on_the_real_store() {
    let (store, _dir) = opened(0).await;
    let mut loss = line("charge", -5_000);
    loss.ts_ist_rfc3339 = "2026-11-23T10:00:00+05:30".into();
    let lines = vec![loss, line("charge", 3_000)];
    PaperLedgerRepo::append_many(&store, lines).await.unwrap();
    let before = contents(&store).await;
    assert_eq!((before.1, before.2), (3, 1));
    let guard = guard(&store, Duration::from_secs(5));
    assert_eq!(guard.check().await, Err(ResetRule::WeeklyLoss));
    let refused = AccountError::ResetRefused(ResetRule::WeeklyLoss);
    let owner = ResetReason::OwnerRequested;
    assert_eq!(reset(&store, owner, &guard).await, Err(refused));
    assert_eq!(contents(&store).await, before);
}

#[tokio::test]
async fn the_real_guard_judges_the_real_ledger_and_commits_on_its_tail() {
    let (store, _dir) = opened(1).await;
    let guard = guard(&store, Duration::from_secs(5));
    assert_eq!(guard.check().await, Ok(1_001));
    let owner = ResetReason::OwnerRequested;
    assert_eq!(reset(&store, owner, &guard).await, Ok(2));
    let rows = store.read_all().await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].draft.kind, "paper_reset");
    assert_eq!(verify_chain(&rows), Ok(()));

    // Today's net is still +1000 across both epochs. A charge of 1001 turns it negative.
    let mut loss = line("charge", -1_001);
    loss.epoch = 2;
    PaperLedgerRepo::append(&store, loss).await.unwrap();
    let before = contents(&store).await;
    let refused = AccountError::ResetRefused(ResetRule::DailyLoss);
    let owner = ResetReason::OwnerRequested;
    assert_eq!(reset(&store, owner, &guard).await, Err(refused));
    assert_eq!(contents(&store).await, before);
}
