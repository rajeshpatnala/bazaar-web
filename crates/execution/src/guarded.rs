//! `SafetyGuard` (kill switch, breaker, rate limit in that fixed order) and the `GuardedExecutor`
//! decorator that re-checks it right before `inner.place`. The pipeline (slice 3) pre-checks with
//! the non-consuming `peek`; only `GuardedExecutor::place` consumes a rate slot, so one order uses
//! exactly one slot (CR-105-1). This is the second layer: a caller holding only the guarded
//! wrapper cannot skip a guard.

use std::sync::Arc;

use bazaar_core::clock::Clock;
use bazaar_core::ports::{AuditRepo, KillSwitchRepo};
use chrono::DateTime;
use chrono_tz::Tz;

use crate::breaker::{DailyLossBreaker, PnlSource};
use crate::executor::{ApprovedOrder, ExecAck, ExecError, Executor};
use crate::killswitch::KillSwitch;
use crate::ratelimit::RateLimiter;
use crate::types::OrderIntent;

/// The three pre-send guards. Generic over the stores, never `dyn`.
pub struct SafetyGuard<R: KillSwitchRepo, P: PnlSource, A: AuditRepo> {
    kill: KillSwitch<R>,
    breaker: DailyLossBreaker<P, A>,
    limiter: RateLimiter,
    /// `execution.breaker_engages_kill_switch`.
    breaker_engages_kill_switch: bool,
}

impl<R: KillSwitchRepo, P: PnlSource, A: AuditRepo> std::fmt::Debug for SafetyGuard<R, P, A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SafetyGuard").finish_non_exhaustive()
    }
}

impl<R: KillSwitchRepo, P: PnlSource, A: AuditRepo> SafetyGuard<R, P, A> {
    pub const fn new(
        kill: KillSwitch<R>,
        breaker: DailyLossBreaker<P, A>,
        limiter: RateLimiter,
        breaker_engages_kill_switch: bool,
    ) -> Self {
        Self {
            kill,
            breaker,
            limiter,
            breaker_engages_kill_switch,
        }
    }

    /// Pipeline pre-check: the same refusals as `place` but no rate slot is consumed.
    ///
    /// # Errors
    /// `KillSwitch`, `BreakerTripped` or `RateLimited`.
    pub async fn peek(&self, intent: &OrderIntent, now: DateTime<Tz>) -> Result<(), ExecError> {
        self.run(intent, now, false).await
    }

    /// First refusal wins; the limiter is consumed only when everything before it passed.
    pub(crate) async fn check(
        &self,
        intent: &OrderIntent,
        now: DateTime<Tz>,
    ) -> Result<(), ExecError> {
        self.run(intent, now, true).await
    }

    async fn run(
        &self,
        intent: &OrderIntent,
        now: DateTime<Tz>,
        consume: bool,
    ) -> Result<(), ExecError> {
        self.kill.admits(intent.reduce_only).await?;
        if self.breaker.tripped(now).await.is_some() {
            if self.breaker_engages_kill_switch {
                // Idempotent; a failed write still leaves the latch set (refuses). After a
                // same-day release the next check engages again (and audits it): a tripped day
                // stays blocked (RISK-105-6).
                if let Err(e) = self.kill.engage("breaker", "daily loss limit").await {
                    tracing::warn!(error = %e, "breaker could not persist the kill switch");
                }
                self.kill.admits(intent.reduce_only).await?;
            } else if !intent.reduce_only {
                return Err(ExecError::BreakerTripped);
            }
        }
        if consume {
            self.limiter.try_acquire(now, intent.reduce_only)
        } else {
            self.limiter.peek(now, intent.reduce_only)
        }
    }
}

/// Wraps an executor so every placement is guarded.
pub struct GuardedExecutor<E: Executor, R: KillSwitchRepo, P: PnlSource, A: AuditRepo> {
    inner: E,
    guard: SafetyGuard<R, P, A>,
    clock: Arc<dyn Clock>,
}

impl<E: Executor, R: KillSwitchRepo, P: PnlSource, A: AuditRepo> std::fmt::Debug
    for GuardedExecutor<E, R, P, A>
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuardedExecutor").finish_non_exhaustive()
    }
}

impl<E: Executor, R: KillSwitchRepo, P: PnlSource, A: AuditRepo> GuardedExecutor<E, R, P, A> {
    pub const fn new(inner: E, guard: SafetyGuard<R, P, A>, clock: Arc<dyn Clock>) -> Self {
        Self {
            inner,
            guard,
            clock,
        }
    }
}

impl<E: Executor, R: KillSwitchRepo, P: PnlSource, A: AuditRepo> Executor
    for GuardedExecutor<E, R, P, A>
{
    async fn place(&self, order: &ApprovedOrder) -> Result<ExecAck, ExecError> {
        self.guard
            .check(order.intent(), self.clock.now_ist())
            .await?;
        self.inner.place(order).await
    }
}

#[cfg(test)]
#[allow(clippy::unused_async_trait_impl)]
mod tests {
    use super::*;
    use crate::breaker::fakes::{FakePnl, breaker_on};
    use crate::killswitch::fakes::{At, MemRepo, Shared};
    use crate::types::{IntentId, OrderKind, Product};
    use bazaar_risk::types::Side;
    use std::sync::atomic::{AtomicU32, Ordering::SeqCst};

    struct Recording(Arc<AtomicU32>);
    impl Executor for Recording {
        async fn place(&self, o: &ApprovedOrder) -> Result<ExecAck, ExecError> {
            self.0.fetch_add(1, SeqCst);
            Ok(ExecAck {
                intent_id: o.intent().id,
                external_id: "x".into(),
                filled_qty: o.intent().qty,
                avg_price: None,
            })
        }
    }

    fn intent(reduce_only: bool) -> OrderIntent {
        OrderIntent {
            id: IntentId(1),
            symbol: "ABC".into(),
            side: Side::Buy,
            qty: 1,
            kind: OrderKind::Market,
            product: Product::Mis,
            reduce_only,
            created_at: At.now_ist(),
        }
    }

    struct Rig {
        ex: GuardedExecutor<Recording, Shared, FakePnl, MemRepo>,
        calls: Arc<AtomicU32>,
        repo: Arc<MemRepo>,
    }

    async fn rig(pnl: i64, per_min: u32, engage: bool, allow_reduce: bool) -> Rig {
        let repo = Arc::new(MemRepo::default());
        let calls = Arc::new(AtomicU32::new(0));
        let kill = KillSwitch::new(
            Shared(repo.clone()),
            Arc::new(At),
            "RELEASE KILL SWITCH".into(),
            allow_reduce,
        )
        .unwrap();
        let breaker = breaker_on(FakePnl::set(pnl, false), &repo, At.now_ist()).await;
        let guard = SafetyGuard::new(
            kill,
            breaker,
            RateLimiter::new(100, per_min, 100).unwrap(),
            engage,
        );
        let ex = GuardedExecutor::new(Recording(calls.clone()), guard, Arc::new(At));
        Rig { ex, calls, repo }
    }

    async fn place(r: &Rig, reduce: bool) -> Result<ExecAck, ExecError> {
        r.ex.place(&ApprovedOrder::new(intent(reduce))).await
    }

    #[tokio::test]
    async fn passing_intent_reaches_inner_once() {
        let r = rig(0, 10, false, true).await;
        assert!(place(&r, false).await.is_ok());
        assert_eq!(r.calls.load(SeqCst), 1);
    }

    #[tokio::test]
    async fn each_layer_refuses_without_reaching_inner() {
        let r = rig(0, 10, false, true).await;
        r.ex.guard.kill.engage("user", "t").await.unwrap();
        assert_eq!(place(&r, false).await, Err(ExecError::KillSwitch));
        let r = rig(-3000, 10, false, true).await;
        assert_eq!(place(&r, false).await, Err(ExecError::BreakerTripped));
        let r = rig(0, 1, false, true).await;
        place(&r, false).await.unwrap();
        assert_eq!(place(&r, false).await, Err(ExecError::RateLimited));
        assert_eq!(r.calls.load(SeqCst), 1);
    }

    #[tokio::test]
    async fn erroring_kill_switch_repo_refuses_even_reduce_only() {
        let r = rig(0, 10, false, true).await;
        r.repo.fail_read.store(true, SeqCst);
        assert_eq!(place(&r, true).await, Err(ExecError::KillSwitch));
        assert_eq!(r.calls.load(SeqCst), 0);
    }

    #[tokio::test]
    async fn breaker_without_kill_switch_still_lets_a_reduce_only_exit_pass() {
        let r = rig(-3000, 10, false, false).await;
        assert!(place(&r, true).await.is_ok());
        assert_eq!(place(&r, false).await, Err(ExecError::BreakerTripped));
    }

    #[tokio::test]
    async fn breaker_engaging_kill_switch_follows_reduce_only_config() {
        let r = rig(-3000, 10, true, false).await;
        assert_eq!(place(&r, true).await, Err(ExecError::KillSwitch));
        assert!(r.ex.guard.kill.is_engaged().await);
        let r = rig(-3000, 10, true, true).await;
        assert!(place(&r, true).await.is_ok());
        assert_eq!(place(&r, false).await, Err(ExecError::KillSwitch));
    }

    #[tokio::test]
    async fn earlier_refusal_consumes_no_rate_slot() {
        let r = rig(0, 1, false, true).await;
        r.ex.guard.kill.engage("user", "t").await.unwrap();
        for _ in 0..3 {
            assert!(place(&r, false).await.is_err());
        }
        r.ex.guard
            .kill
            .release("RELEASE KILL SWITCH", "user")
            .await
            .unwrap();
        assert!(place(&r, false).await.is_ok());
    }

    #[tokio::test]
    async fn pipeline_peek_then_guarded_place_counts_one_slot() {
        let r = rig(0, 2, false, true).await;
        let i = intent(false);
        for _ in 0..5 {
            r.ex.guard.peek(&i, At.now_ist()).await.unwrap();
        }
        assert!(place(&r, false).await.is_ok());
        r.ex.guard.peek(&i, At.now_ist()).await.unwrap();
        assert!(place(&r, false).await.is_ok());
        assert_eq!(place(&r, false).await, Err(ExecError::RateLimited));
        assert_eq!(r.calls.load(SeqCst), 2);
    }

    #[tokio::test]
    async fn reduce_only_exit_passes_at_the_minute_cap_and_keeps_entry_budget() {
        let r = rig(0, 1, false, true).await;
        assert!(place(&r, false).await.is_ok());
        assert_eq!(place(&r, false).await, Err(ExecError::RateLimited));
        assert!(place(&r, true).await.is_ok());
        assert_eq!(r.calls.load(SeqCst), 2);
        // A fresh rig: exits first do not use up the single entry slot.
        let r = rig(0, 1, false, true).await;
        assert!(place(&r, true).await.is_ok());
        assert!(place(&r, false).await.is_ok());
    }

    #[tokio::test]
    async fn breaker_kill_switch_is_persisted_and_seen_by_a_second_rig() {
        let r = rig(-3000, 10, true, false).await;
        assert_eq!(place(&r, false).await, Err(ExecError::KillSwitch));
        assert!(r.repo.engaged.load(SeqCst));
        let logged = r
            .repo
            .rows
            .lock()
            .unwrap()
            .iter()
            .any(|x| x.draft.kind == "kill_switch_engaged" && x.draft.actor == "breaker");
        assert!(logged);
        let kill = KillSwitch::new(
            Shared(r.repo.clone()),
            Arc::new(At),
            "RELEASE KILL SWITCH".into(),
            false,
        )
        .unwrap();
        assert!(kill.is_engaged().await);
    }

    #[tokio::test]
    async fn released_kill_switch_on_a_tripped_day_is_engaged_again_and_audited() {
        let r = rig(-3000, 10, true, false).await;
        assert_eq!(place(&r, false).await, Err(ExecError::KillSwitch));
        r.ex.guard
            .kill
            .release("RELEASE KILL SWITCH", "user")
            .await
            .unwrap();
        assert_eq!(place(&r, false).await, Err(ExecError::KillSwitch));
        assert!(r.repo.engaged.load(SeqCst));
        let n = r
            .repo
            .rows
            .lock()
            .unwrap()
            .iter()
            .filter(|x| x.draft.kind == "kill_switch_engaged")
            .count();
        assert_eq!(n, 2);
        assert_eq!(r.calls.load(SeqCst), 0);
    }
}
