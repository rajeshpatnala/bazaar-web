//! The single-writer `Pipeline`: gate, approval queue, re-gate at approval, pre-check of the
//! guards (non-consuming `peek`), then the guarded executor. All state lives in one task fed by a
//! channel, so two approvals of one intent resolve first-wins without locks. Paper and live differ
//! only in the executor.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use bazaar_core::clock::Clock;
use bazaar_core::config::ExecutionSection;
use bazaar_core::ports::{
    ApprovalDecision, ApprovalRepo, AuditRepo, KillSwitchRepo, OrderRecord, OrderRepo,
    QuoteSnapshot, QuoteSource, RepoError,
};
use chrono::DateTime;
use chrono_tz::Tz;
use rust_decimal::Decimal;
use tokio::sync::{mpsc, oneshot};

use crate::approval::{ApprovalQueue, Begin, Outcome, Pending, Track};
use crate::audit::chain_hash;
use crate::breaker::PnlSource;
use crate::executor::{ApprovedOrder, ExecError, Executor};
use crate::fills::FillHandler;
use crate::guarded::GuardedExecutor;
use crate::killswitch::ReleaseError;
use crate::state::{ExecEvent, OrderState};
use crate::types::{IntentId, OrderIntent};

/// Longest `actor` / `reason` / typed phrase accepted at the handle edge (SEC-100-4).
pub const MAX_TEXT: usize = 500;

/// What the gate says. The real-`Gate` adapter lives in the composition slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateOutcome {
    /// Allowed; `risk_inr` is the gate's rupee risk for the order (0 for exits), recorded in the
    /// approval audit payload as a string.
    Pass {
        risk_inr: Decimal,
    },
    Block(String),
    /// The quote moved past the drift limit; the re-computed rupee risk.
    Drifted {
        new_risk: Decimal,
    },
}

/// Sync gate port. The pipeline runs it in `spawn_blocking` (the gate does CPU work and takes
/// locks), so it never stalls the writer's runtime thread.
pub trait GatePort: Send + Sync + 'static {
    fn gate(&self, intent: &OrderIntent, quote: Option<&QuoteSnapshot>) -> GateOutcome;
}

/// An executor wrapped in the guards: the pipeline pre-checks and drives the kill switch.
pub trait Guarded: Executor {
    fn peek(
        &self,
        intent: &OrderIntent,
        now: DateTime<Tz>,
    ) -> impl Future<Output = Result<(), ExecError>> + Send;
    fn engage(
        &self,
        actor: &str,
        reason: &str,
    ) -> impl Future<Output = Result<(), ExecError>> + Send;
    fn release(
        &self,
        typed: &str,
        actor: &str,
    ) -> impl Future<Output = Result<(), ReleaseError>> + Send;
    /// The kill switch's in-process latch, shared with the handle (read only there). Contract:
    /// this MUST be the very flag the guard checks, never a fresh `AtomicBool`, or the GET
    /// would show "off" while orders are refused.
    fn latch_flag(&self) -> Arc<AtomicBool>;
}

impl<E: Executor, R: KillSwitchRepo, P: PnlSource, A: AuditRepo> Guarded
    for GuardedExecutor<E, R, P, A>
{
    async fn peek(&self, intent: &OrderIntent, now: DateTime<Tz>) -> Result<(), ExecError> {
        self.guard().peek(intent, now).await
    }
    async fn engage(&self, actor: &str, reason: &str) -> Result<(), ExecError> {
        self.guard().kill().engage(actor, reason).await
    }
    async fn release(&self, typed: &str, actor: &str) -> Result<(), ReleaseError> {
        self.guard().kill().release(typed, actor).await
    }
    fn latch_flag(&self) -> Arc<AtomicBool> {
        self.guard().kill().latch_flag()
    }
}

/// Why a handle call failed before or outside an order outcome.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PipelineError {
    #[error("text longer than {MAX_TEXT} bytes")]
    TooLong,
    #[error("pipeline stopped")]
    Closed,
    #[error(transparent)]
    Exec(#[from] ExecError),
    #[error(transparent)]
    Release(#[from] ReleaseError),
}

type Reply<T> = oneshot::Sender<T>;

enum Cmd {
    Submit(OrderIntent, Reply<Outcome>),
    Decide(IntentId, ApprovalDecision, String, Reply<Outcome>),
    Kill {
        engage: bool,
        actor: String,
        text: String,
        reply: Reply<Result<(), PipelineError>>,
    },
    ExpireStale(Reply<usize>),
}

/// Cloneable front door; there is no other way in.
#[derive(Debug, Clone)]
pub struct PipelineHandle {
    tx: mpsc::Sender<Cmd>,
    latch: Arc<AtomicBool>,
}

impl PipelineHandle {
    /// True while the kill switch's in-process latch is set (engaged, or an engage whose save
    /// failed). Read straight from the shared flag, so it never queues behind a slow command and
    /// still answers after the pipeline stopped (the last value stays; never "off" by default).
    #[must_use]
    pub fn kill_switch_latched(&self) -> bool {
        // SeqCst is deliberate: one safety flag shared with the guard; the cost is irrelevant.
        self.latch.load(Ordering::SeqCst)
    }

    async fn call<T>(&self, make: impl FnOnce(Reply<T>) -> Cmd) -> Result<T, PipelineError> {
        let (r, rx) = oneshot::channel();
        self.tx
            .send(make(r))
            .await
            .map_err(|_| PipelineError::Closed)?;
        rx.await.map_err(|_| PipelineError::Closed)
    }

    /// Gate an intent and queue it for approval.
    ///
    /// # Errors
    /// `Closed` when the pipeline stopped.
    pub async fn submit(&self, intent: OrderIntent) -> Result<Outcome, PipelineError> {
        self.call(|r| Cmd::Submit(intent, r)).await
    }

    /// Record the user's decision (`actor` is the authenticated session user, audited); a second
    /// one gets `AlreadyDecided`.
    ///
    /// # Errors
    /// `TooLong` (actor over [`MAX_TEXT`]) or `Closed` when the pipeline stopped.
    pub async fn decide(
        &self,
        id: IntentId,
        d: ApprovalDecision,
        actor: &str,
    ) -> Result<Outcome, PipelineError> {
        if actor.len() > MAX_TEXT {
            return Err(PipelineError::TooLong);
        }
        let actor = actor.to_owned();
        self.call(|r| Cmd::Decide(id, d, actor, r)).await
    }

    /// Engage (`text` = reason) or release (`text` = typed phrase). `actor` and `text` are capped
    /// at [`MAX_TEXT`] bytes (the store caps bytes too, SEC-130-1) before anything is sent or audited.
    ///
    /// # Errors
    /// `TooLong`, `Closed`, or the kill switch's own error.
    pub async fn set_kill_switch(
        &self,
        engage: bool,
        actor: &str,
        text: &str,
    ) -> Result<(), PipelineError> {
        if actor.len() > MAX_TEXT || text.len() > MAX_TEXT {
            return Err(PipelineError::TooLong);
        }
        let (actor, text) = (actor.to_owned(), text.to_owned());
        self.call(|reply| Cmd::Kill {
            engage,
            actor,
            text,
            reply,
        })
        .await?
    }

    /// Expire every queued intent past its TTL (run at startup and on a timer).
    ///
    /// # Errors
    /// `Closed` when the pipeline stopped.
    pub async fn expire_stale(&self) -> Result<usize, PipelineError> {
        self.call(Cmd::ExpireStale).await
    }
}

/// I/O timeout used when the supplied one is zero (RISK-109-11).
pub const DEFAULT_IO_TIMEOUT: Duration = Duration::from_secs(5);
/// Ceiling for the I/O timeout: a larger value is clamped so a bad config cannot stall the writer.
pub const MAX_IO_TIMEOUT: Duration = Duration::from_secs(30);

/// What the pipeline takes from `[execution]` config: the approval TTL and the per-call I/O
/// timeout. Built only from the typed section, so no raw `Duration` is passed at the call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineConfig {
    approval_ttl_secs: u32,
    io_timeout: Duration,
}

impl PipelineConfig {
    /// `execution.approval_ttl_secs` and `execution.io_timeout_secs` (loader range 1..=30; the
    /// pipeline still clamps as a second layer).
    #[must_use]
    pub fn from_section(e: &ExecutionSection) -> Self {
        Self {
            approval_ttl_secs: e.approval_ttl_secs,
            io_timeout: clamp_io(e.io_timeout()),
        }
    }

    /// The approval TTL in seconds.
    #[must_use]
    pub const fn approval_ttl_secs(&self) -> u32 {
        self.approval_ttl_secs
    }

    /// The per-call I/O timeout.
    #[must_use]
    pub const fn io_timeout(&self) -> Duration {
        self.io_timeout
    }
}

fn clamp_io(d: Duration) -> Duration {
    if d.is_zero() {
        DEFAULT_IO_TIMEOUT
    } else {
        d.min(MAX_IO_TIMEOUT)
    }
}

/// The writer. Owns the queue, the executor and the order store.
pub struct Pipeline<X: Guarded, G: GatePort, Q: QuoteSource, O: OrderRepo, A: ApprovalRepo> {
    pub(crate) exec: X,
    gate: Arc<G>,
    quotes: Q,
    orders: Arc<O>,
    fills: FillHandler<O>,
    queue: ApprovalQueue<A>,
    clock: Arc<dyn Clock>,
    io_timeout: Duration,
}

impl<X: Guarded, G: GatePort, Q: QuoteSource, O: OrderRepo, A: ApprovalRepo> std::fmt::Debug
    for Pipeline<X, G, Q, O, A>
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pipeline").finish_non_exhaustive()
    }
}

impl<X: Guarded, G: GatePort, Q: QuoteSource, O: OrderRepo, A: ApprovalRepo>
    Pipeline<X, G, Q, O, A>
{
    /// `cfg.io_timeout` bounds every quote read, gate run and send so one hung call cannot stall
    /// the writer; `cfg` comes from `PipelineConfig::from_section`.
    #[allow(clippy::too_many_arguments)] // composition root wires every port once
    pub fn new(
        exec: X,
        gate: Arc<G>,
        quotes: Q,
        orders: Arc<O>,
        approvals: A,
        cfg: PipelineConfig,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            exec,
            gate,
            quotes,
            fills: FillHandler::new(orders.clone()),
            orders,
            queue: ApprovalQueue::new(approvals, cfg.approval_ttl_secs),
            clock,
            io_timeout: clamp_io(cfg.io_timeout),
        }
    }

    /// Move the pipeline into its task; the handle is the only way to reach it.
    #[must_use]
    pub fn spawn(self) -> PipelineHandle
    where
        Self: Send + 'static,
    {
        let (tx, mut rx) = mpsc::channel(64);
        let latch = self.exec.latch_flag();
        tokio::spawn(async move {
            let mut me = self;
            while let Some(cmd) = rx.recv().await {
                match cmd {
                    Cmd::Submit(i, r) => drop(r.send(me.submit(i).await)),
                    Cmd::Decide(id, d, who, r) => drop(r.send(me.decide(id, d, &who).await)),
                    Cmd::Kill {
                        engage,
                        actor,
                        text,
                        reply,
                    } => {
                        let res = if engage {
                            me.exec
                                .engage(&actor, &text)
                                .await
                                .map_err(PipelineError::from)
                        } else {
                            me.exec
                                .release(&text, &actor)
                                .await
                                .map_err(PipelineError::from)
                        };
                        drop(reply.send(res));
                    }
                    Cmd::ExpireStale(r) => drop(r.send(me.expire_stale().await)),
                }
            }
        });
        PipelineHandle { tx, latch }
    }

    async fn run_gate(&self, intent: &OrderIntent, quote: Option<QuoteSnapshot>) -> GateOutcome {
        let (g, i) = (self.gate.clone(), intent.clone());
        let run = tokio::task::spawn_blocking(move || g.gate(&i, quote.as_ref()));
        match self.bounded(run).await {
            Some(Ok(g)) => g,
            Some(Err(_)) => GateOutcome::Block("gate task failed".into()),
            None => GateOutcome::Block("gate timed out".into()),
        }
    }

    /// Re-run the gate with a fresh quote; no quote (or a late one) blocks.
    async fn regate(&self, intent: &OrderIntent) -> GateOutcome {
        match self.bounded(self.quotes.quote(&intent.symbol)).await {
            Some(Ok(q)) => self.run_gate(intent, Some(q)).await,
            _ => GateOutcome::Block("no fresh quote".into()),
        }
    }

    /// `None` when `f` did not finish within the I/O timeout.
    async fn bounded<T>(&self, f: impl Future<Output = T>) -> Option<T> {
        tokio::time::timeout(self.io_timeout, f).await.ok()
    }

    /// Insert, gate and queue one intent.
    pub(crate) async fn submit(&mut self, intent: OrderIntent) -> Outcome {
        self.try_submit(intent).await.unwrap_or_else(Outcome::Error)
    }

    async fn try_submit(&mut self, intent: OrderIntent) -> Result<Outcome, ExecError> {
        let now = self.clock.now_ist();
        let id = intent.id;
        let record = OrderRecord {
            intent_id: id.0,
            version: 0,
            state: OrderState::Proposed.as_str().into(),
            payload: intent.to_payload(),
        };
        let audit = bazaar_core::ports::AuditDraft {
            ts_ist_rfc3339: now.to_rfc3339(),
            actor: "pipeline".into(),
            kind: "proposed".into(),
            payload: serde_json::json!({ "intent": id.to_string() }).to_string(),
        };
        self.orders
            .insert(record, audit, chain_hash)
            .await
            .map_err(|_| ExecError::Storage)?;
        let mut track = Track {
            id,
            version: 0,
            state: OrderState::Proposed,
        };
        // No quote, or a late one, blocks explicitly: the gate never sees `None` (SEC-109-3).
        let blocked = match self.bounded(self.quotes.quote(&intent.symbol)).await {
            Some(Ok(q)) => self.run_gate(&intent, Some(q)).await,
            _ => GateOutcome::Block("no fresh quote".into()),
        };
        match blocked {
            GateOutcome::Pass { .. } => {
                track
                    .advance(&*self.orders, ExecEvent::GatePassed, now)
                    .await?;
                self.queue.enqueue(Pending {
                    intent,
                    track,
                    queued_at: now,
                });
                Ok(Outcome::Queued)
            }
            blocked => {
                track
                    .advance(&*self.orders, ExecEvent::GateBlocked, now)
                    .await?;
                let out = blocked_outcome(blocked);
                self.queue.finish(id, out.clone());
                Ok(out)
            }
        }
    }

    /// Resolve a user decision. First wins; later calls get `AlreadyDecided`. A storage failure
    /// before anything was persisted puts the intent back in the queue so the user can retry; after
    /// that point the intent is finished as `Error` (its stored state is what reconciliation uses).
    pub(crate) async fn decide(
        &mut self,
        id: IntentId,
        d: ApprovalDecision,
        actor: &str,
    ) -> Outcome {
        match self.queue.begin(id) {
            Begin::Unknown => Outcome::Error(ExecError::InvalidInput("unknown intent")),
            Begin::Done(o) => Outcome::AlreadyDecided(Box::new(o)),
            Begin::Live(mut p) => match self.resolve(&mut p, d, actor).await {
                Ok(out) => {
                    self.queue.finish(id, out.clone());
                    out
                }
                Err(e) if p.track.state == OrderState::PendingApproval => {
                    self.queue.enqueue(*p);
                    Outcome::Error(e)
                }
                Err(e) => {
                    self.queue.finish(id, Outcome::Error(e.clone()));
                    Outcome::Error(e)
                }
            },
        }
    }

    async fn resolve(
        &self,
        p: &mut Pending,
        d: ApprovalDecision,
        actor: &str,
    ) -> Result<Outcome, ExecError> {
        let now = self.clock.now_ist();
        let o = &*self.orders;
        if self.queue.is_expired(p, now) {
            p.track.advance(o, ExecEvent::Expire, now).await?;
            return Ok(Outcome::Expired);
        }
        if d == ApprovalDecision::Decline {
            let stopped = self
                .decide_step(p, d, ExecEvent::Decline, now, actor, None)
                .await?;
            return Ok(stopped.unwrap_or(Outcome::Declined));
        }
        // Re-gate with a fresh quote; no quote means no approval-time gate, so it blocks.
        let blocked = self.regate(&p.intent).await;
        let GateOutcome::Pass { risk_inr } = blocked else {
            p.track.advance(o, ExecEvent::GateBlocked, now).await?;
            return Ok(blocked_outcome(blocked));
        };
        // Order: TTL, fresh-quote re-gate, then ONE atomic decide+advance (audited), then send.
        // The quote fetch can take seconds: re-read the clock, re-check the TTL with it and use
        // that time for the decision and audit (RISK-132-2).
        let now = self.clock.now_ist();
        if self.queue.is_expired(p, now) {
            p.track.advance(o, ExecEvent::Expire, now).await?;
            return Ok(Outcome::Expired);
        }
        if let Some(stopped) = self
            .decide_step(p, d, ExecEvent::Approve, now, actor, Some(risk_inr))
            .await?
        {
            return Ok(stopped);
        }
        self.send_approved(p, now, "pipeline", None).await
    }

    /// Pre-send guards, then `Send` and the executor call, for an order already `Approved`.
    async fn send_approved(
        &self,
        p: &mut Pending,
        now: DateTime<Tz>,
        actor: &str,
        reason: Option<&str>,
    ) -> Result<Outcome, ExecError> {
        let o = &*self.orders;
        if let Err(e) = self.exec.peek(&p.intent, now).await {
            p.track
                .advance_why(o, ExecEvent::GuardBlocked, now, actor, reason)
                .await?;
            return Ok(Outcome::GuardBlocked(e));
        }
        p.track.advance(o, ExecEvent::Send, now).await?;
        let order = ApprovedOrder::new(p.intent.clone());
        // A send that hangs is an unknown outcome, never a retry.
        let sent = self
            .bounded(self.exec.place(&order))
            .await
            .unwrap_or(Err(ExecError::Unknown));
        match sent {
            Ok(ack) => {
                // The order is out: a failed fill write must not hide that (RISK-109-2).
                if self
                    .fills
                    .apply(&mut p.track, p.intent.qty, ack.filled_qty, now)
                    .await
                    .is_err()
                {
                    tracing::error!(intent = %p.track.id, "order sent but fill not recorded");
                    return Ok(Outcome::PlacedUnrecorded(ack));
                }
                Ok(Outcome::Placed(ack))
            }
            Err(e) => {
                let ev = if e == ExecError::Unknown {
                    ExecEvent::Lost
                } else {
                    ExecEvent::Reject
                };
                p.track.advance(o, ev, now).await?;
                Ok(Outcome::ExecFailed(e))
            }
        }
    }

    /// The one atomic decision write (approval row, order advance, audit row). `Some(outcome)`
    /// when another decision or state already holds: nothing is sent. A Conflict does not prove
    /// an Approve exists (RISK-131-5), so the order is re-read and its state decides the outcome;
    /// a still-pending order is not decided: it is kept and a retryable `Storage` error returned.
    async fn decide_step(
        &self,
        p: &mut Pending,
        d: ApprovalDecision,
        event: ExecEvent,
        now: DateTime<Tz>,
        actor: &str,
        risk_inr: Option<Decimal>,
    ) -> Result<Option<Outcome>, ExecError> {
        match self
            .queue
            .decide_and_advance(&mut p.track, d, event, now, actor, risk_inr)
            .await
        {
            Ok(()) => Ok(None),
            Err(RepoError::Conflict) => self.after_conflict(p).await.map(Some),
            Err(_) => Err(ExecError::Storage),
        }
    }

    async fn after_conflict(&self, p: &mut Pending) -> Result<Outcome, ExecError> {
        let rec = self
            .orders
            .get(p.track.id.0)
            .await
            .map_err(|_| ExecError::Storage)?
            .ok_or(ExecError::Storage)?;
        if rec.state == OrderState::PendingApproval.as_str() {
            // Still pending: a lost write race (busy/snapshot also maps to Conflict), not a
            // decision. Nothing is decided or declined; the intent stays queued and the caller
            // may retry (CR-132-1, RISK-132-3).
            return Err(ExecError::Storage);
        }
        let prior = match rec.state.as_str() {
            "declined" => Outcome::Declined,
            "expired" => Outcome::Expired,
            _ => Outcome::Error(ExecError::InvalidInput("order already resolved")),
        };
        Ok(Outcome::AlreadyDecided(Box::new(prior)))
    }

    /// Restart reload; call exactly once before [`Self::spawn`], and abort startup on `Err`
    /// (the queue is emptied on a failure part way, but nothing may be served). Order matters
    /// (fail closed):
    /// 1. stray decision rows (a pending order that already has a decision) are expired, audited,
    ///    and never re-presented; an error here aborts the reload;
    /// 2. `pending_approval` orders are re-queued (queue time = the intent's creation time, so a
    ///    reloaded intent never lives longer than its TTL allows); past-TTL or undecodable ones
    ///    are expired, audited with the reason;
    /// 3. `approved` but never sent orders are resumed only if TTL, a fresh-quote re-gate and the
    ///    guards all pass; otherwise they end `blocked`, audited with the reason, nothing sent;
    /// 4. `sent`, `partially_filled` and `unknown` orders are never touched or re-sent (the
    ///    reconciler owns them); they are counted and logged, and a failed list is an error.
    ///
    /// Ids already queued or finished are skipped, so a repeat call changes nothing.
    ///
    /// # Errors
    /// `ExecError::Storage` when a list, the stray-row guard or an audited write fails or a list
    /// is too large; nothing is reported as all-clear.
    pub async fn reload(&mut self) -> Result<ReloadReport, ExecError> {
        let out = self.reload_inner().await;
        if out.is_err() {
            self.queue.clear_pending();
        }
        out
    }

    async fn reload_inner(&mut self) -> Result<ReloadReport, ExecError> {
        const BY: &str = "system";
        let now = self.clock.now_ist();
        let strays = self
            .queue
            .reconcile_strays(now)
            .await
            .map_err(|_| ExecError::Storage)?
            .len();
        let mut rep = ReloadReport {
            strays,
            ..ReloadReport::default()
        };
        let pending = self
            .orders
            .list_in_state(OrderState::PendingApproval.as_str(), RELOAD_LIMIT)
            .await
            .map_err(|_| ExecError::Storage)?;
        let approved = self
            .orders
            .list_in_state(OrderState::Approved.as_str(), RELOAD_LIMIT)
            .await
            .map_err(|_| ExecError::Storage)?;
        for r in pending {
            let id = IntentId(r.intent_id);
            if self.queue.knows(id) {
                continue;
            }
            let mut track = track_of(&r, OrderState::PendingApproval);
            let reason = match OrderIntent::from_payload(id, &r.payload, now) {
                Some(intent) if !self.queue.ttl_passed(intent.created_at, now) => {
                    self.queue.enqueue(Pending {
                        queued_at: intent.created_at,
                        intent,
                        track,
                    });
                    rep.requeued += 1;
                    continue;
                }
                Some(_) => "restart_reload: past approval TTL",
                None => "restart_reload: stored order not decodable",
            };
            track
                .advance_why(&*self.orders, ExecEvent::Expire, now, BY, Some(reason))
                .await?;
            self.queue.finish(id, Outcome::Expired);
            rep.expired += 1;
        }
        for r in approved {
            let id = IntentId(r.intent_id);
            if self.queue.knows(id) {
                continue;
            }
            let mut track = track_of(&r, OrderState::Approved);
            let out = if let Some(intent) = OrderIntent::from_payload(id, &r.payload, now) {
                let mut p = Pending {
                    queued_at: intent.created_at,
                    intent,
                    track,
                };
                self.resume_approved(&mut p)
                    .await
                    .unwrap_or_else(Outcome::Error)
            } else {
                let why = "restart_reload: stored order not decodable";
                track
                    .advance_why(&*self.orders, ExecEvent::GuardBlocked, now, BY, Some(why))
                    .await?;
                Outcome::GateBlocked("stored order not decodable".into())
            };
            match &out {
                Outcome::Placed(_) | Outcome::PlacedUnrecorded(_) => rep.resumed += 1,
                // The order may still be `approved` in the store: not counted as blocked.
                Outcome::Error(e) => {
                    tracing::error!(intent = %id, error = %e, "reload: approved order not resolved");
                    rep.errors += 1;
                }
                _ => rep.blocked += 1,
            }
            self.queue.finish(id, out);
        }
        rep.in_flight = self.count_in_flight().await?;
        Ok(rep)
    }

    /// Count sent, partially filled and unknown orders (never re-sent); a failed list is an error.
    async fn count_in_flight(&self) -> Result<usize, ExecError> {
        let mut total = 0;
        for st in [
            OrderState::Sent,
            OrderState::PartiallyFilled,
            OrderState::Unknown,
        ] {
            let n = self
                .orders
                .list_in_state(st.as_str(), RELOAD_LIMIT)
                .await
                .map_err(|_| ExecError::Storage)?
                .len();
            if n > 0 {
                tracing::warn!(
                    state = st.as_str(),
                    n,
                    "reload: in-flight orders left to reconciliation, never re-sent"
                );
            }
            total += n;
        }
        Ok(total)
    }

    /// An `approved`-but-unsent order after a restart: TTL (measured from the intent's creation,
    /// stricter than the live path, which measures from the queue time), then a fresh-quote
    /// re-gate, then the same guards and send as a live approval. A failure ends it `blocked`,
    /// audited with the reason (actor "system"), and sends nothing. The clock is re-read after
    /// the quote.
    async fn resume_approved(&self, p: &mut Pending) -> Result<Outcome, ExecError> {
        const BY: &str = "system";
        let o = &*self.orders;
        let expired = "approval expired before restart resume";
        let now = self.clock.now_ist();
        if self.queue.is_expired(p, now) {
            let why = format!("restart_reload: {expired}");
            p.track
                .advance_why(o, ExecEvent::GuardBlocked, now, BY, Some(&why))
                .await?;
            return Ok(Outcome::GateBlocked(expired.into()));
        }
        let gate = self.regate(&p.intent).await;
        let now = self.clock.now_ist();
        if !matches!(gate, GateOutcome::Pass { .. }) {
            let why = match &gate {
                GateOutcome::Block(r) => format!("restart_reload: gate blocked: {r}"),
                _ => "restart_reload: quote drifted".to_owned(),
            };
            p.track
                .advance_why(o, ExecEvent::GuardBlocked, now, BY, Some(&why))
                .await?;
            return Ok(blocked_outcome(gate));
        }
        if self.queue.is_expired(p, now) {
            let why = format!("restart_reload: {expired}");
            p.track
                .advance_why(o, ExecEvent::GuardBlocked, now, BY, Some(&why))
                .await?;
            return Ok(Outcome::GateBlocked(expired.into()));
        }
        self.send_approved(p, now, BY, Some("restart_reload: guard refused the send"))
            .await
    }

    /// Expire every queued intent past its TTL; returns how many.
    pub async fn expire_stale(&mut self) -> usize {
        let now = self.clock.now_ist();
        let mut n = 0;
        for id in self.queue.stale(now) {
            if let Begin::Live(mut p) = self.queue.begin(id) {
                let out = match p.track.advance(&*self.orders, ExecEvent::Expire, now).await {
                    Ok(()) => Outcome::Expired,
                    Err(e) => Outcome::Error(e),
                };
                self.queue.finish(id, out);
                n += 1;
            }
        }
        n
    }
}

/// Most orders per state a reload will read; more is an error, never a silent truncation.
const RELOAD_LIMIT: u32 = 1000;

/// What a restart reload did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReloadReport {
    /// Stray decision rows expired by the startup guard.
    pub strays: usize,
    /// Pending intents back in the queue.
    pub requeued: usize,
    /// Pending intents expired (past TTL or undecodable).
    pub expired: usize,
    /// Approved-but-unsent orders sent after a fresh TTL check, re-gate and guards.
    pub resumed: usize,
    /// Approved-but-unsent orders ended `blocked`, nothing sent.
    pub blocked: usize,
    /// Approved orders whose resolution failed (storage); they may still be `approved`.
    pub errors: usize,
    /// Sent, partially filled or unknown orders left alone.
    pub in_flight: usize,
}

fn track_of(r: &OrderRecord, state: OrderState) -> Track {
    Track {
        id: IntentId(r.intent_id),
        version: r.version,
        state,
    }
}

fn blocked_outcome(g: GateOutcome) -> Outcome {
    match g {
        GateOutcome::Drifted { new_risk } => Outcome::QuoteDrifted { new_risk },
        GateOutcome::Block(r) => Outcome::GateBlocked(r),
        GateOutcome::Pass { .. } => Outcome::Error(ExecError::Storage),
    }
}

#[cfg(test)]
#[allow(clippy::unused_async_trait_impl)]
pub(crate) mod fakes {
    use super::*;
    use crate::breaker::fakes::{FakePnl, breaker_on};
    use crate::fills::RecordingExecutor;
    use crate::guarded::SafetyGuard;
    use crate::killswitch::KillSwitch;
    use crate::killswitch::fakes::{At, MemRepo, Shared};
    use crate::ratelimit::RateLimiter;
    use bazaar_core::ports::{AuditDraft, ChainFn, RepoError, SourceError};
    use bazaar_risk::types::Side;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering::SeqCst};

    #[derive(Default)]
    pub(crate) struct MemOrders(
        pub Mutex<HashMap<u128, (u32, String)>>,
        pub Mutex<Option<String>>,
        /// Creation time (ns, from the insert's audit timestamp), like the real store's `created_ns`.
        pub Mutex<HashMap<u128, i64>>,
        /// Stored payload per order (the intent JSON the reload decodes).
        pub Mutex<HashMap<u128, String>>,
        /// Every `advance`: (order id, audit actor, audit payload).
        pub Mutex<Vec<(u128, String, String)>>,
    );
    impl OrderRepo for MemOrders {
        async fn insert(&self, o: OrderRecord, a: AuditDraft, _: ChainFn) -> Result<(), RepoError> {
            let mut m = self.0.lock().unwrap();
            if m.contains_key(&o.intent_id) {
                return Err(RepoError::Conflict);
            }
            let ns = chrono::DateTime::parse_from_rfc3339(&a.ts_ist_rfc3339)
                .ok()
                .and_then(|d| d.timestamp_nanos_opt())
                .unwrap_or(0);
            self.2.lock().unwrap().insert(o.intent_id, ns);
            self.3.lock().unwrap().insert(o.intent_id, o.payload);
            m.insert(o.intent_id, (0, o.state));
            Ok(())
        }
        async fn get(&self, id: u128) -> Result<Option<OrderRecord>, RepoError> {
            Ok(self.0.lock().unwrap().get(&id).map(|(v, s)| OrderRecord {
                intent_id: id,
                version: *v,
                state: s.clone(),
                payload: self.3.lock().unwrap().get(&id).cloned().unwrap_or_default(),
            }))
        }
        async fn advance(
            &self,
            id: u128,
            ver: u32,
            st: String,
            _: String,
            au: AuditDraft,
            _: ChainFn,
        ) -> Result<u32, RepoError> {
            if self.1.lock().unwrap().as_deref() == Some(st.as_str()) {
                return Err(RepoError::Other);
            }
            let mut m = self.0.lock().unwrap();
            let e = m.get_mut(&id).ok_or(RepoError::Other)?;
            if e.0 != ver {
                return Err(RepoError::Conflict);
            }
            *e = (ver + 1, st);
            self.4.lock().unwrap().push((id, au.actor, au.payload));
            Ok(ver + 1)
        }
        async fn list_in_state(&self, st: &str, limit: u32) -> Result<Vec<OrderRecord>, RepoError> {
            let m = self.0.lock().unwrap();
            let mut ids: Vec<u128> = m
                .iter()
                .filter(|(_, (_, s))| s == st)
                .map(|(i, _)| *i)
                .collect();
            let created = self.2.lock().unwrap();
            // Oldest first: creation time, then id (the real store's ORDER BY).
            ids.sort_unstable_by_key(|i| (created.get(i).copied().unwrap_or(0), *i));
            if ids.len() > limit as usize {
                return Err(RepoError::TooLarge);
            }
            Ok(ids
                .into_iter()
                .map(|i| OrderRecord {
                    intent_id: i,
                    version: m[&i].0,
                    state: m[&i].1.clone(),
                    payload: self.3.lock().unwrap().get(&i).cloned().unwrap_or_default(),
                })
                .collect())
        }
    }

    #[derive(Default)]
    pub(crate) struct MemApprovals {
        pub rows: Mutex<HashMap<u128, ApprovalDecision>>,
        /// Shared order table: when set, `decide_and_advance` has the real repo's state and
        /// version checks (RISK-131-4, TS-131-1).
        pub orders: Option<std::sync::Arc<MemOrders>>,
        /// Inject a failure between the approval row and the order advance (rolls back).
        pub fail_between: AtomicBool,
        /// Every committed decision: (approval actor, audit actor).
        pub actors: Mutex<Vec<(String, String)>>,
        /// Payload of every committed decision audit.
        pub payloads: Mutex<Vec<String>>,
    }
    impl ApprovalRepo for MemApprovals {
        async fn decide_and_advance(
            &self,
            id: u128,
            d: ApprovalDecision,
            actor: &str,
            ver: u32,
            ns: String,
            _: String,
            au: AuditDraft,
            _: ChainFn,
        ) -> Result<u32, RepoError> {
            let ok_target = match d {
                ApprovalDecision::Approve => ["approved"].contains(&ns.as_str()),
                ApprovalDecision::Decline => {
                    ["declined", "expired", "blocked"].contains(&ns.as_str())
                }
            };
            if !ok_target {
                return Err(RepoError::Conflict);
            }
            let mut m = self.rows.lock().unwrap();
            if m.contains_key(&id) {
                return Err(RepoError::Conflict);
            }
            let Some(orders) = &self.orders else {
                // No order table wired: first-wins only (the real store is authoritative).
                m.insert(id, d);
                return Ok(ver + 1);
            };
            let mut o = orders.0.lock().unwrap();
            let e = o.get_mut(&id).ok_or(RepoError::Conflict)?;
            if e.1 != "pending_approval" || e.0 != ver {
                return Err(RepoError::Conflict);
            }
            // Tentative approval row, then the failure point, then the advance: a failure
            // leaves both tables unchanged (the real repo rolls back the transaction).
            if self.fail_between.load(SeqCst) {
                return Err(RepoError::Other);
            }
            self.payloads.lock().unwrap().push(au.payload.clone());
            m.insert(id, d);
            self.actors
                .lock()
                .unwrap()
                .push((actor.to_owned(), au.actor));
            *e = (ver + 1, ns);
            Ok(ver + 1)
        }

        async fn reconcile_stray_decisions(
            &self,
            _: &str,
            _: ChainFn,
        ) -> Result<Vec<u128>, RepoError> {
            let rows = self.rows.lock().unwrap();
            let Some(orders) = &self.orders else {
                return Ok(Vec::new());
            };
            let mut o = orders.0.lock().unwrap();
            let mut ids: Vec<u128> = o
                .iter()
                .filter(|(i, (_, st))| st == "pending_approval" && rows.contains_key(i))
                .map(|(i, _)| *i)
                .collect();
            ids.sort_unstable();
            for i in &ids {
                let e = o.get_mut(i).unwrap();
                *e = (e.0 + 1, "expired".into());
            }
            Ok(ids)
        }
    }

    /// (fail, hang, clock moved on by this many seconds while the quote is fetched).
    pub(crate) struct Quotes(
        pub AtomicBool,
        pub AtomicBool,
        pub Mutex<Option<(Arc<Tick>, i64)>>,
    );
    impl QuoteSource for Quotes {
        async fn quote(&self, _: &str) -> Result<QuoteSnapshot, SourceError> {
            if let Some((clock, secs)) = &*self.2.lock().unwrap() {
                *clock.0.lock().unwrap() += chrono::Duration::seconds(*secs);
            }
            if self.1.load(SeqCst) {
                std::future::pending::<()>().await;
            }
            if self.0.load(SeqCst) {
                return Err(SourceError::Unavailable);
            }
            Ok(QuoteSnapshot {
                last: Decimal::from(100),
                lower_circuit: None,
                upper_circuit: None,
                bar_volume: None,
                at_ist_rfc3339: String::new(),
            })
        }
    }

    /// Gate with a settable verdict; records the thread it ran on.
    pub(crate) struct FakeGate {
        pub verdict: Mutex<GateOutcome>,
        pub thread: Mutex<Option<std::thread::ThreadId>>,
    }
    impl GatePort for FakeGate {
        fn gate(&self, _: &OrderIntent, _: Option<&QuoteSnapshot>) -> GateOutcome {
            *self.thread.lock().unwrap() = Some(std::thread::current().id());
            self.verdict.lock().unwrap().clone()
        }
    }

    /// Clock the test can move.
    pub(crate) struct Tick(pub Mutex<DateTime<Tz>>);
    impl Clock for Tick {
        fn now_ist(&self) -> DateTime<Tz> {
            *self.0.lock().unwrap()
        }
    }

    pub(crate) type TestGuarded = GuardedExecutor<RecordingExecutor, Shared, FakePnl, MemRepo>;
    pub(crate) type TestPipe = Pipeline<TestGuarded, FakeGate, Quotes, MemOrders, MemApprovals>;

    pub(crate) struct Rig {
        pub pipe: TestPipe,
        pub rec: RecordingExecutor,
        pub gate: Arc<FakeGate>,
        pub clock: Arc<Tick>,
        pub orders: Arc<MemOrders>,
    }

    pub(crate) fn start() -> DateTime<Tz> {
        At.now_ist()
    }

    pub(crate) fn intent(id: u128, reduce_only: bool) -> OrderIntent {
        OrderIntent {
            id: IntentId(id),
            symbol: "ABC".into(),
            side: Side::Buy,
            qty: 2,
            kind: crate::types::OrderKind::Market,
            product: crate::types::Product::Mis,
            reduce_only,
            created_at: start(),
        }
    }

    /// Rig with `per_min` rate slots, `pnl` day P&L (rupees) and a 300 s approval TTL.
    pub(crate) async fn rig(per_min: u32, pnl: i64) -> Rig {
        rig_with(per_min, pnl, MemApprovals::default()).await
    }

    pub(crate) async fn rig_with(per_min: u32, pnl: i64, mut approvals: MemApprovals) -> Rig {
        let repo = Arc::new(MemRepo::default());
        let clock = Arc::new(Tick(Mutex::new(start())));
        let kill = KillSwitch::new(
            Shared(repo.clone()),
            clock.clone(),
            "RELEASE KILL SWITCH".into(),
            true,
        )
        .unwrap();
        let breaker = breaker_on(FakePnl::set(pnl, false), &repo, start()).await;
        let guard = SafetyGuard::new(
            kill,
            breaker,
            RateLimiter::new(100, per_min, 100).unwrap(),
            false,
        );
        let rec = RecordingExecutor::default();
        let exec = GuardedExecutor::new(rec.clone(), guard, clock.clone());
        let gate = Arc::new(FakeGate {
            verdict: Mutex::new(GateOutcome::Pass {
                risk_inr: Decimal::ZERO,
            }),
            thread: Mutex::new(None),
        });
        let orders = Arc::new(MemOrders::default());
        approvals.orders = Some(orders.clone());
        let pipe = Pipeline::new(
            exec,
            gate.clone(),
            Quotes(
                AtomicBool::new(false),
                AtomicBool::new(false),
                Mutex::new(None),
            ),
            orders.clone(),
            approvals,
            // Struct literal on purpose: the 150 ms timeout is below the loader's 1..=30 s range.
            PipelineConfig {
                approval_ttl_secs: 300,
                io_timeout: Duration::from_millis(150),
            },
            clock.clone(),
        );
        Rig {
            pipe,
            rec,
            gate,
            clock,
            orders,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fakes::*;
    use super::*;
    use crate::fills::RecordingExecutor;
    use ApprovalDecision::{Approve, Decline};
    use proptest::prelude::*;

    fn ids(r: &Rig) -> Vec<u128> {
        r.rec.placed().iter().map(|i| i.0).collect()
    }
    async fn queued(r: &mut Rig, id: u128, reduce: bool) {
        assert_eq!(r.pipe.submit(intent(id, reduce)).await, Outcome::Queued);
    }

    /// Seed an order row as a restart would find it (the in-memory queue is empty).
    async fn seed(r: &Rig, id: u128, state: &str, payload: String) {
        r.orders
            .insert(
                OrderRecord {
                    intent_id: id,
                    version: 0,
                    state: state.into(),
                    payload,
                },
                bazaar_core::ports::AuditDraft {
                    ts_ist_rfc3339: start().to_rfc3339(),
                    actor: "t".into(),
                    kind: "seed".into(),
                    payload: "{}".into(),
                },
                chain_hash,
            )
            .await
            .unwrap();
    }
    /// Audit rows the fake recorded for `id`: (actor, payload).
    fn audits(r: &Rig, id: u128) -> Vec<(String, String)> {
        r.orders
            .4
            .lock()
            .unwrap()
            .iter()
            .filter(|a| a.0 == id)
            .map(|a| (a.1.clone(), a.2.clone()))
            .collect()
    }
    fn state_of(r: &Rig, id: u128) -> String {
        r.orders.0.lock().unwrap()[&id].1.clone()
    }

    #[tokio::test]
    async fn submit_stores_the_full_intent_and_reload_requeues_it_with_its_original_time() {
        let mut r = rig(10, 0).await;
        queued(&mut r, 1, false).await;
        let stored = r.orders.get(1).await.unwrap().unwrap().payload;
        assert_eq!(
            OrderIntent::from_payload(IntentId(1), &stored, start()),
            Some(intent(1, false))
        );
        // Restart: a fresh pipeline over the same stores finds it pending.
        seed(&r, 2, "pending_approval", intent(2, false).to_payload()).await;
        *r.clock.0.lock().unwrap() += chrono::Duration::seconds(100);
        let rep = r.pipe.reload().await.unwrap();
        assert_eq!((rep.requeued, rep.expired), (1, 0));
        // A repeat reload changes nothing: ids already queued are skipped.
        let again = r.pipe.reload().await.unwrap();
        assert_eq!((again.requeued, again.expired), (0, 0));
        // Original queue time: 100 s already used, so 200 s more is past the 300 s TTL.
        *r.clock.0.lock().unwrap() += chrono::Duration::seconds(200);
        assert_eq!(r.pipe.expire_stale().await, 2);
        assert_eq!(state_of(&r, 2), "expired");
    }

    #[tokio::test]
    async fn reload_requeued_intent_is_approved_with_a_fresh_regate() {
        let mut r = rig(10, 0).await;
        seed(&r, 5, "pending_approval", intent(5, false).to_payload()).await;
        // Reload needs the order at version 0 and the pending state: seeded as such.
        assert_eq!(r.pipe.reload().await.unwrap().requeued, 1);
        *r.gate.verdict.lock().unwrap() = GateOutcome::Block("moved".into());
        assert!(matches!(
            r.pipe.decide(IntentId(5), Approve, "u").await,
            Outcome::GateBlocked(_)
        ));
        assert_eq!(ids(&r), Vec::<u128>::new());
        assert_eq!(state_of(&r, 5), "blocked");
    }

    #[tokio::test]
    async fn reload_expires_past_ttl_and_undecodable_pending_rows_audited() {
        let mut r = rig(10, 0).await;
        seed(&r, 1, "pending_approval", intent(1, false).to_payload()).await;
        seed(&r, 2, "pending_approval", String::new()).await;
        seed(
            &r,
            3,
            "pending_approval",
            r#"{"symbol":"ABC","qty":2}"#.into(),
        )
        .await;
        *r.clock.0.lock().unwrap() += chrono::Duration::seconds(300);
        let rep = r.pipe.reload().await.unwrap();
        assert_eq!((rep.requeued, rep.expired), (0, 3));
        for id in [1, 2, 3] {
            assert_eq!(state_of(&r, id), "expired");
        }
        assert_eq!(
            r.pipe.decide(IntentId(2), Approve, "u").await,
            Outcome::AlreadyDecided(Box::new(Outcome::Expired))
        );
        assert_eq!(ids(&r), Vec::<u128>::new());
        let a = audits(&r, 1);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].0, "system");
        assert!(a[0].1.contains("restart_reload: past approval TTL"));
        assert!(audits(&r, 2)[0].1.contains("not decodable"));
        // An undecodable row expires even inside the TTL.
        seed(&r, 4, "pending_approval", "garbage".into()).await;
        assert_eq!(r.pipe.reload().await.unwrap().expired, 1);
        assert_eq!(state_of(&r, 4), "expired");
    }

    #[tokio::test]
    async fn reload_never_re_presents_an_order_with_a_stray_decision_row() {
        let seeded = MemApprovals::default();
        seeded.rows.lock().unwrap().insert(1, Approve);
        let mut r = rig_with(10, 0, seeded).await;
        seed(&r, 1, "pending_approval", intent(1, false).to_payload()).await;
        let rep = r.pipe.reload().await.unwrap();
        assert_eq!((rep.strays, rep.requeued), (1, 0));
        assert_eq!(state_of(&r, 1), "expired");
        assert_eq!(
            r.pipe.decide(IntentId(1), Approve, "u").await,
            Outcome::Error(ExecError::InvalidInput("unknown intent"))
        );
        assert_eq!(ids(&r), Vec::<u128>::new());
    }

    #[tokio::test]
    async fn reload_resumes_an_approved_unsent_order_only_after_ttl_and_regate_pass() {
        let mut r = rig(10, 0).await;
        seed(&r, 1, "approved", intent(1, false).to_payload()).await;
        let rep = r.pipe.reload().await.unwrap();
        assert_eq!((rep.resumed, rep.blocked), (1, 0));
        assert_eq!(ids(&r), vec![1]);
        assert_eq!(state_of(&r, 1), "filled");
        // The gate ran (a fresh quote was fetched) before the send.
        assert!(r.gate.thread.lock().unwrap().is_some());
        // Finished: a late decide gets the stored outcome and nothing is sent twice.
        assert!(matches!(
            r.pipe.decide(IntentId(1), Approve, "u").await,
            Outcome::AlreadyDecided(_)
        ));
        assert_eq!(ids(&r), vec![1]);
    }

    #[tokio::test]
    async fn reload_blocks_an_approved_unsent_order_when_gate_ttl_quote_or_guard_fails() {
        // Gate block.
        let mut r = rig(10, 0).await;
        seed(&r, 1, "approved", intent(1, false).to_payload()).await;
        *r.gate.verdict.lock().unwrap() = GateOutcome::Block("x".into());
        let rep = r.pipe.reload().await.unwrap();
        assert_eq!((rep.resumed, rep.blocked), (0, 1));
        assert_eq!(state_of(&r, 1), "blocked");
        // No fresh quote.
        let mut r2 = rig(10, 0).await;
        seed(&r2, 1, "approved", intent(1, false).to_payload()).await;
        r2.pipe
            .quotes
            .0
            .store(true, std::sync::atomic::Ordering::SeqCst);
        r2.pipe.reload().await.unwrap();
        assert_eq!(state_of(&r2, 1), "blocked");
        // Past the TTL.
        let mut r3 = rig(10, 0).await;
        seed(&r3, 1, "approved", intent(1, false).to_payload()).await;
        *r3.clock.0.lock().unwrap() += chrono::Duration::seconds(300);
        r3.pipe.reload().await.unwrap();
        assert_eq!(state_of(&r3, 1), "blocked");
        // TTL passes during the slow quote.
        let mut r4 = rig(10, 0).await;
        seed(&r4, 1, "approved", intent(1, false).to_payload()).await;
        *r4.clock.0.lock().unwrap() += chrono::Duration::seconds(299);
        *r4.pipe.quotes.2.lock().unwrap() = Some((r4.clock.clone(), 2));
        r4.pipe.reload().await.unwrap();
        assert_eq!(state_of(&r4, 1), "blocked");
        // Guard refuses (no rate slots).
        let mut r5 = rig(1, 0).await;
        queued(&mut r5, 9, false).await;
        assert!(matches!(
            r5.pipe.decide(IntentId(9), Approve, "u").await,
            Outcome::Placed(_)
        ));
        seed(&r5, 1, "approved", intent(1, false).to_payload()).await;
        r5.pipe.reload().await.unwrap();
        assert_eq!(state_of(&r5, 1), "blocked");
        // An undecodable approved row is blocked, never sent.
        let mut r6 = rig(10, 0).await;
        seed(&r6, 1, "approved", "garbage".into()).await;
        r6.pipe.reload().await.unwrap();
        assert_eq!(state_of(&r6, 1), "blocked");
        // Kill switch engaged.
        let mut r7 = rig(10, 0).await;
        seed(&r7, 1, "approved", intent(1, false).to_payload()).await;
        r7.pipe.exec.engage("a", "r").await.unwrap();
        r7.pipe.reload().await.unwrap();
        assert_eq!(state_of(&r7, 1), "blocked");
        // Reduce-only with no position: the gate refuses.
        let mut r8 = rig(10, 0).await;
        seed(&r8, 1, "approved", intent(1, true).to_payload()).await;
        *r8.gate.verdict.lock().unwrap() = GateOutcome::Block("reduce_only: no position".into());
        r8.pipe.reload().await.unwrap();
        assert_eq!(state_of(&r8, 1), "blocked");
        // Quote drifted.
        let mut r9 = rig(10, 0).await;
        seed(&r9, 1, "approved", intent(1, false).to_payload()).await;
        *r9.gate.verdict.lock().unwrap() = GateOutcome::Drifted {
            new_risk: Decimal::from(9),
        };
        let rep = r9.pipe.reload().await.unwrap();
        assert_eq!((rep.resumed, rep.blocked), (0, 1));
        assert_eq!(state_of(&r9, 1), "blocked");
        for r in [&r, &r2, &r3, &r4, &r6, &r7, &r8, &r9] {
            assert_eq!(ids(r), Vec::<u128>::new());
        }
        assert_eq!(ids(&r5), vec![9]);
        // Every restart-time block is audited by "system" with its reason.
        for (rg, why) in [
            (&r, "gate blocked: x"),
            (&r2, "restart_reload"),
            (&r3, "approval expired"),
            (&r4, "approval expired"),
            (&r5, "guard refused"),
            (&r6, "not decodable"),
            (&r7, "guard refused"),
            (&r8, "reduce_only: no position"),
            (&r9, "quote drifted"),
        ] {
            let a = audits(rg, 1);
            assert_eq!(a.len(), 1, "{why}");
            assert_eq!(a[0].0, "system");
            assert!(a[0].1.contains(why), "{why}: {}", a[0].1);
        }
    }

    #[tokio::test]
    async fn reload_never_touches_or_resends_sent_partial_or_unknown_orders() {
        let mut r = rig(10, 0).await;
        for (id, st) in [(1, "sent"), (2, "partially_filled"), (3, "unknown")] {
            seed(&r, id, st, intent(id, false).to_payload()).await;
        }
        let rep = r.pipe.reload().await.unwrap();
        assert_eq!(rep.in_flight, 3);
        assert_eq!(ids(&r), Vec::<u128>::new());
        assert_eq!(state_of(&r, 1), "sent");
        assert_eq!(state_of(&r, 2), "partially_filled");
        assert_eq!(state_of(&r, 3), "unknown");
    }

    #[tokio::test]
    async fn reload_with_more_rows_than_the_limit_is_an_error_and_sends_nothing() {
        let mut r = rig(10, 0).await;
        seed(&r, 1, "approved", intent(1, false).to_payload()).await;
        // More approved rows than the reload limit is an error, never a truncation.
        for id in 10..=(10 + u128::from(RELOAD_LIMIT)) {
            r.orders
                .0
                .lock()
                .unwrap()
                .insert(id, (0, "approved".into()));
        }
        assert_eq!(r.pipe.reload().await, Err(ExecError::Storage));
        assert_eq!(ids(&r), Vec::<u128>::new());
    }

    #[tokio::test]
    async fn happy_path_places_once_and_second_approval_is_already_decided() {
        let mut r = rig(10, 0).await;
        queued(&mut r, 1, false).await;
        let out = r.pipe.decide(IntentId(1), Approve, "u").await;
        assert!(matches!(&out, Outcome::Placed(a) if a.filled_qty == 2));
        assert_eq!(r.orders.0.lock().unwrap()[&1].1, "filled");
        assert_eq!(
            r.pipe.decide(IntentId(1), Approve, "u").await,
            Outcome::AlreadyDecided(Box::new(out))
        );
        assert!(matches!(
            r.pipe.decide(IntentId(1), Decline, "u").await,
            Outcome::AlreadyDecided(_)
        ));
        assert_eq!(ids(&r), vec![1]);
    }

    #[tokio::test]
    async fn gate_block_at_submit_and_at_approval_and_drift_never_send() {
        let mut r = rig(10, 0).await;
        *r.gate.verdict.lock().unwrap() = GateOutcome::Block("G01".into());
        assert_eq!(
            r.pipe.submit(intent(1, false)).await,
            Outcome::GateBlocked("G01".into())
        );
        *r.gate.verdict.lock().unwrap() = GateOutcome::Pass {
            risk_inr: Decimal::ZERO,
        };
        queued(&mut r, 2, false).await;
        *r.gate.verdict.lock().unwrap() = GateOutcome::Drifted {
            new_risk: Decimal::from(9),
        };
        assert_eq!(
            r.pipe.decide(IntentId(2), Approve, "u").await,
            Outcome::QuoteDrifted {
                new_risk: Decimal::from(9)
            }
        );
        assert_eq!(r.orders.0.lock().unwrap()[&2].1, "blocked");
        assert_eq!(ids(&r), Vec::<u128>::new());
    }

    #[tokio::test]
    async fn missing_quote_at_approval_blocks() {
        let mut r = rig(10, 0).await;
        queued(&mut r, 1, false).await;
        r.pipe
            .quotes
            .0
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            r.pipe.decide(IntentId(1), Approve, "u").await,
            Outcome::GateBlocked("no fresh quote".into())
        );
        assert_eq!(ids(&r), Vec::<u128>::new());
    }

    #[tokio::test]
    async fn expiry_decline_and_expire_stale() {
        let mut r = rig(10, 0).await;
        queued(&mut r, 1, false).await;
        queued(&mut r, 2, false).await;
        queued(&mut r, 3, false).await;
        assert_eq!(
            r.pipe.decide(IntentId(3), Decline, "u").await,
            Outcome::Declined
        );
        *r.clock.0.lock().unwrap() += chrono::Duration::seconds(301);
        assert_eq!(
            r.pipe.decide(IntentId(1), Approve, "u").await,
            Outcome::Expired
        );
        assert_eq!(r.pipe.expire_stale().await, 1);
        assert_eq!(r.orders.0.lock().unwrap()[&2].1, "expired");
        assert_eq!(
            r.pipe.decide(IntentId(9), Approve, "u").await,
            Outcome::Error(ExecError::InvalidInput("unknown intent"))
        );
        assert_eq!(ids(&r), Vec::<u128>::new());
    }

    #[tokio::test]
    async fn gate_runs_off_the_runtime_thread() {
        let mut r = rig(10, 0).await;
        queued(&mut r, 1, false).await;
        let ran_on = r.gate.thread.lock().unwrap().unwrap();
        assert_ne!(ran_on, std::thread::current().id());
    }

    #[tokio::test]
    async fn kill_switch_blocks_entries_both_layers_but_exit_passes() {
        let mut r = rig(10, 0).await;
        queued(&mut r, 1, false).await;
        queued(&mut r, 2, true).await;
        r.pipe.exec.engage("t", "test").await.unwrap();
        // Pipeline layer: refused before the executor is touched.
        assert_eq!(
            r.pipe.decide(IntentId(1), Approve, "u").await,
            Outcome::GuardBlocked(ExecError::KillSwitch)
        );
        assert_eq!(ids(&r), Vec::<u128>::new());
        // Guard layer: the wrapper refuses a raw approved order on its own.
        let raw = ApprovedOrder::new(intent(5, false));
        assert_eq!(r.pipe.exec.place(&raw).await, Err(ExecError::KillSwitch));
        assert_eq!(ids(&r), Vec::<u128>::new());
        assert!(matches!(
            r.pipe.decide(IntentId(2), Approve, "u").await,
            Outcome::Placed(_)
        ));
        assert_eq!(ids(&r), vec![2]);
    }

    #[tokio::test]
    async fn breaker_blocks_entries_both_layers_but_exit_passes() {
        let mut r = rig(10, -2500).await;
        queued(&mut r, 1, false).await;
        queued(&mut r, 2, true).await;
        assert_eq!(
            r.pipe.decide(IntentId(1), Approve, "u").await,
            Outcome::GuardBlocked(ExecError::BreakerTripped)
        );
        let raw = ApprovedOrder::new(intent(5, false));
        assert_eq!(
            r.pipe.exec.place(&raw).await,
            Err(ExecError::BreakerTripped)
        );
        assert!(matches!(
            r.pipe.decide(IntentId(2), Approve, "u").await,
            Outcome::Placed(_)
        ));
        assert_eq!(ids(&r), vec![2]);
    }

    #[tokio::test]
    async fn one_rate_slot_per_order_and_exit_is_exempt() {
        let mut r = rig(1, 0).await;
        for i in 1..=3 {
            queued(&mut r, i, i == 3).await;
        }
        // peek consumed nothing: the single slot is still there for the first order's place.
        assert!(matches!(
            r.pipe.decide(IntentId(1), Approve, "u").await,
            Outcome::Placed(_)
        ));
        // Pipeline layer refuses the second entry; guard layer would too.
        assert_eq!(
            r.pipe.decide(IntentId(2), Approve, "u").await,
            Outcome::GuardBlocked(ExecError::RateLimited)
        );
        assert_eq!(
            r.pipe
                .exec
                .place(&ApprovedOrder::new(intent(7, false)))
                .await,
            Err(ExecError::RateLimited)
        );
        assert!(matches!(
            r.pipe.decide(IntentId(3), Approve, "u").await,
            Outcome::Placed(_)
        ));
        assert_eq!(ids(&r), vec![1, 3]);
    }

    #[tokio::test]
    async fn handle_shows_the_latch_set_by_engage_and_cleared_by_release() {
        let r = rig(10, 0).await;
        let h = r.pipe.spawn();
        assert!(!h.kill_switch_latched());
        h.set_kill_switch(true, "a", "why").await.unwrap();
        assert!(h.kill_switch_latched());
        // A clone sees the same flag.
        assert!(h.clone().kill_switch_latched());
        h.set_kill_switch(false, "a", "RELEASE KILL SWITCH")
            .await
            .unwrap();
        assert!(!h.kill_switch_latched());
    }

    #[tokio::test]
    async fn kill_switch_text_cap_counts_bytes_not_characters() {
        let r = rig(10, 0).await;
        let h = r.pipe.spawn();
        // 300 two-byte characters: 300 chars but 600 bytes, over the store cap.
        let long = "\u{e9}".repeat(300);
        assert_eq!(
            h.set_kill_switch(true, "user", &long).await,
            Err(PipelineError::TooLong)
        );
        assert_eq!(
            h.set_kill_switch(true, &long, "r").await,
            Err(PipelineError::TooLong)
        );
        assert!(
            h.set_kill_switch(true, "user", &"\u{e9}".repeat(250))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn handle_caps_text_and_routes_commands() {
        let r = rig(10, 0).await;
        let rec = r.rec.clone();
        let h = r.pipe.spawn();
        let long = "x".repeat(501);
        assert_eq!(
            h.set_kill_switch(true, &long, "r").await,
            Err(PipelineError::TooLong)
        );
        assert_eq!(
            h.set_kill_switch(true, "a", &long).await,
            Err(PipelineError::TooLong)
        );
        assert_eq!(
            h.set_kill_switch(false, "a", &"y".repeat(500)).await,
            Err(PipelineError::Release(ReleaseError::WrongPhrase))
        );
        h.set_kill_switch(true, "a", "why").await.unwrap();
        assert_eq!(h.submit(intent(1, false)).await.unwrap(), Outcome::Queued);
        assert_eq!(
            h.decide(IntentId(1), Approve, "u").await.unwrap(),
            Outcome::GuardBlocked(ExecError::KillSwitch)
        );
        h.set_kill_switch(false, "a", "RELEASE KILL SWITCH")
            .await
            .unwrap();
        assert_eq!(h.expire_stale().await.unwrap(), 0);
        assert_eq!(rec.placed(), vec![]);
    }

    #[tokio::test]
    async fn hung_approval_time_quote_blocks_instead_of_stalling() {
        let mut r = rig(10, 0).await;
        queued(&mut r, 1, false).await;
        r.pipe
            .quotes
            .1
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let out = r.pipe.decide(IntentId(1), Approve, "u").await;
        assert!(matches!(out, Outcome::GateBlocked(_)), "{out:?}");
        assert_eq!(ids(&r), Vec::<u128>::new());
    }

    #[tokio::test]
    async fn fake_repos_model_list_in_state_and_first_wins_decide_and_advance() {
        use crate::pipeline::fakes::{MemApprovals, MemOrders};
        let draft = || bazaar_core::ports::AuditDraft {
            ts_ist_rfc3339: "2026-11-22T09:15:00+05:30".into(),
            actor: "t".into(),
            kind: "k".into(),
            payload: "{}".into(),
        };
        let o = MemOrders::default();
        let rec = |id| OrderRecord {
            intent_id: id,
            version: 0,
            state: "pending_approval".into(),
            payload: String::new(),
        };
        for id in [2, 1] {
            o.insert(rec(id), draft(), chain_hash).await.unwrap();
        }
        let ids: Vec<u128> = o
            .list_in_state("pending_approval", 5)
            .await
            .unwrap()
            .iter()
            .map(|r| r.intent_id)
            .collect();
        assert_eq!(ids, vec![1, 2]);
        assert_eq!(
            o.list_in_state("pending_approval", 1).await,
            Err(bazaar_core::ports::RepoError::TooLarge)
        );
        let a = MemApprovals::default();
        let go = |d| {
            a.decide_and_advance(
                1,
                d,
                "u",
                0,
                "approved".into(),
                "e".into(),
                draft(),
                chain_hash,
            )
        };
        assert_eq!(go(Approve).await, Ok(1));
        assert_eq!(
            go(Decline).await,
            Err(bazaar_core::ports::RepoError::Conflict)
        );
    }

    #[tokio::test]
    async fn fake_list_in_state_sorts_by_creation_time_then_id() {
        use crate::pipeline::fakes::MemOrders;
        let o = MemOrders::default();
        for (id, ts) in [(3u128, "09:15"), (1, "09:17"), (2, "09:17"), (4, "09:16")] {
            let d = bazaar_core::ports::AuditDraft {
                ts_ist_rfc3339: format!("2026-11-22T{ts}:00+05:30"),
                actor: "t".into(),
                kind: "k".into(),
                payload: "{}".into(),
            };
            let r = OrderRecord {
                intent_id: id,
                version: 0,
                state: "pending_approval".into(),
                payload: String::new(),
            };
            o.insert(r, d, chain_hash).await.unwrap();
        }
        let ids: Vec<u128> = o
            .list_in_state("pending_approval", 9)
            .await
            .unwrap()
            .iter()
            .map(|r| r.intent_id)
            .collect();
        assert_eq!(ids, vec![3, 4, 1, 2]);
    }

    #[tokio::test]
    async fn fake_decide_and_advance_checks_state_version_target_and_rolls_back() {
        use crate::pipeline::fakes::{MemApprovals, MemOrders};
        use bazaar_core::ports::RepoError::{Conflict, Other};
        let draft = || bazaar_core::ports::AuditDraft {
            ts_ist_rfc3339: "2026-11-22T09:15:00+05:30".into(),
            actor: "t".into(),
            kind: "k".into(),
            payload: "{}".into(),
        };
        let orders = Arc::new(MemOrders::default());
        for (id, st) in [(1, "pending_approval"), (2, "placed")] {
            let r = OrderRecord {
                intent_id: id,
                version: 0,
                state: st.into(),
                payload: String::new(),
            };
            orders.insert(r, draft(), chain_hash).await.unwrap();
        }
        let a = MemApprovals {
            orders: Some(orders.clone()),
            ..Default::default()
        };
        let go = |id, d, v, to: &str| {
            a.decide_and_advance(id, d, "u", v, to.into(), "e".into(), draft(), chain_hash)
        };
        assert_eq!(go(2, Approve, 0, "approved").await, Err(Conflict)); // not pending
        assert_eq!(go(9, Approve, 0, "approved").await, Err(Conflict)); // missing
        assert_eq!(go(1, Approve, 3, "approved").await, Err(Conflict)); // stale version
        assert_eq!(go(1, Approve, 0, "placed").await, Err(Conflict)); // target vs decision
        a.fail_between
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(go(1, Approve, 0, "approved").await, Err(Other)); // injected failure
        assert!(a.rows.lock().unwrap().is_empty());
        assert_eq!(
            orders.get(1).await.unwrap().unwrap().state,
            "pending_approval"
        );
        a.fail_between
            .store(false, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(go(1, Approve, 0, "approved").await, Ok(1));
        assert_eq!(orders.get(1).await.unwrap().unwrap().state, "approved");
    }

    #[test]
    fn confirmation_strings_are_recorded() {
        // The loader refuses unknown keys, so provenance lives in the files as text (TS-130-1).
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../config/");
        let read = |f: &str| std::fs::read_to_string(format!("{dir}{f}")).unwrap();
        let risk = read("risk.toml");
        let exec = risk.split("\n[execution]\n").nth(1).unwrap();
        assert!(exec.contains("source = \"owner confirmed 2026-11-22\""));
        assert!(exec.contains("verified_on = \"2026-11-22\""));
        assert!(read("data.toml").contains("kite.*: owner confirmed 2026-11-22"));
        assert!(read("strategies.toml").contains("owner confirmed 2026-11-22"));
    }

    #[test]
    fn pipeline_config_reads_ttl_and_timeout_from_the_risk_section() {
        let r: bazaar_core::config::RiskConfig = bazaar_core::config::load(std::path::Path::new(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../config"),
        ))
        .unwrap();
        let c = PipelineConfig::from_section(&r.execution);
        assert_eq!(c.approval_ttl_secs(), r.execution.approval_ttl_secs);
        assert_eq!(c.io_timeout(), Duration::from_secs(5));
        assert!(
            r.execution.breaker_engages_kill_switch,
            "owner confirmed 2026-11-22"
        );
    }

    #[tokio::test]
    async fn io_timeout_is_defaulted_and_clamped() {
        assert_eq!(
            clamp_io(Duration::from_millis(150)),
            Duration::from_millis(150)
        );
        assert_eq!(clamp_io(Duration::ZERO), DEFAULT_IO_TIMEOUT);
        assert_eq!(clamp_io(Duration::from_secs(3600)), MAX_IO_TIMEOUT);
    }

    #[tokio::test]
    async fn fill_write_failure_after_send_is_reported_as_placed_unrecorded() {
        let mut r = rig(10, 0).await;
        queued(&mut r, 1, false).await;
        *r.orders.1.lock().unwrap() = Some("filled".into());
        let out = r.pipe.decide(IntentId(1), Approve, "u").await;
        assert!(matches!(out, Outcome::PlacedUnrecorded(a) if a.filled_qty == 2));
        assert_eq!(ids(&r), vec![1]);
    }

    #[tokio::test]
    async fn storage_failure_before_approval_keeps_the_intent_for_a_retry() {
        let mut r = rig(10, 0).await;
        queued(&mut r, 1, false).await;
        // Failure between the approval row and the order advance: both roll back.
        r.pipe
            .queue
            .repo()
            .fail_between
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            r.pipe.decide(IntentId(1), Approve, "u").await,
            Outcome::Error(ExecError::Storage)
        );
        assert!(r.pipe.queue.repo().rows.lock().unwrap().is_empty());
        assert_eq!(r.orders.0.lock().unwrap()[&1].1, "pending_approval");
        assert_eq!(ids(&r), Vec::<u128>::new());
        r.pipe
            .queue
            .repo()
            .fail_between
            .store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(matches!(
            r.pipe.decide(IntentId(1), Approve, "u").await,
            Outcome::Placed(_)
        ));
    }

    #[tokio::test]
    async fn ttl_and_regate_run_before_any_decision_is_written() {
        let mut r = rig(10, 0).await;
        queued(&mut r, 1, false).await;
        queued(&mut r, 2, false).await;
        // Re-gate block: Blocked, no approval row, nothing sent.
        *r.gate.verdict.lock().unwrap() = GateOutcome::Block("x".into());
        assert!(matches!(
            r.pipe.decide(IntentId(1), Approve, "u").await,
            Outcome::GateBlocked(_)
        ));
        assert!(r.pipe.queue.repo().rows.lock().unwrap().is_empty());
        // Past the TTL: Expired, no approval row, nothing sent.
        *r.gate.verdict.lock().unwrap() = GateOutcome::Pass {
            risk_inr: Decimal::ZERO,
        };
        *r.clock.0.lock().unwrap() += chrono::Duration::seconds(300);
        assert_eq!(
            r.pipe.decide(IntentId(2), Approve, "u").await,
            Outcome::Expired
        );
        assert!(r.pipe.queue.repo().rows.lock().unwrap().is_empty());
        assert_eq!(ids(&r), Vec::<u128>::new());
    }

    #[tokio::test]
    async fn decide_on_a_terminal_order_is_refused_and_nothing_is_written_or_sent() {
        let mut r = rig(10, 0).await;
        queued(&mut r, 1, false).await;
        // The order reached a terminal state behind the queue's back.
        r.orders.0.lock().unwrap().get_mut(&1).unwrap().1 = "expired".into();
        let out = r.pipe.decide(IntentId(1), Approve, "u").await;
        assert_eq!(out, Outcome::AlreadyDecided(Box::new(Outcome::Expired)));
        assert!(r.pipe.queue.repo().rows.lock().unwrap().is_empty());
        assert_eq!(r.orders.0.lock().unwrap()[&1].1, "expired");
        assert_eq!(ids(&r), Vec::<u128>::new());
    }

    #[tokio::test]
    async fn approver_is_stored_on_the_approval_row_and_the_audit_row() {
        let mut r = rig(10, 0).await;
        queued(&mut r, 1, false).await;
        assert!(matches!(
            r.pipe.decide(IntentId(1), Approve, "alice").await,
            Outcome::Placed(_)
        ));
        assert_eq!(
            *r.pipe.queue.repo().actors.lock().unwrap(),
            vec![("alice".to_owned(), "alice".to_owned())]
        );
    }

    #[tokio::test]
    async fn approval_audit_payload_carries_risk_inr_as_a_string() {
        let mut r = rig(10, 0).await;
        queued(&mut r, 1, false).await;
        *r.gate.verdict.lock().unwrap() = GateOutcome::Pass {
            risk_inr: Decimal::new(12050, 2),
        };
        assert!(matches!(
            r.pipe.decide(IntentId(1), Approve, "alice").await,
            Outcome::Placed(_)
        ));
        let payloads = r.pipe.queue.repo().payloads.lock().unwrap().clone();
        assert_eq!(payloads.len(), 1);
        let v: serde_json::Value = serde_json::from_str(&payloads[0]).unwrap();
        assert_eq!(v["risk_inr"], "120.50");
    }

    #[tokio::test]
    async fn a_lost_race_conflict_keeps_the_order_pending_and_a_retry_succeeds() {
        // Conflict with the order still pending at the same version (a lost write race, or a
        // stray decision row): nothing is declined, the intent stays queued (CR-132-1).
        let seeded = MemApprovals::default();
        seeded.rows.lock().unwrap().insert(1, Approve);
        let mut r = rig_with(10, 0, seeded).await;
        queued(&mut r, 1, false).await;
        // A Decline click is not reported as "already decided" either.
        let out = r.pipe.decide(IntentId(1), Decline, "u").await;
        assert_eq!(out, Outcome::Error(ExecError::Storage));
        let out = r.pipe.decide(IntentId(1), Approve, "u").await;
        assert_eq!(out, Outcome::Error(ExecError::Storage));
        assert_eq!(r.orders.0.lock().unwrap()[&1].1, "pending_approval");
        assert_eq!(ids(&r), Vec::<u128>::new());
        // The race clears: a retried Approve goes through.
        r.pipe.queue.repo().rows.lock().unwrap().clear();
        assert!(matches!(
            r.pipe.decide(IntentId(1), Approve, "u").await,
            Outcome::Placed(_)
        ));
        assert_eq!(ids(&r), vec![1]);
    }

    #[tokio::test]
    async fn ttl_is_rechecked_after_a_slow_quote_and_nothing_is_decided_or_sent() {
        let mut r = rig(10, 0).await;
        queued(&mut r, 1, false).await;
        // Still inside the TTL when decide starts; the quote fetch takes the clock past it.
        *r.clock.0.lock().unwrap() += chrono::Duration::seconds(299);
        *r.pipe.quotes.2.lock().unwrap() = Some((r.clock.clone(), 2));
        assert_eq!(
            r.pipe.decide(IntentId(1), Approve, "u").await,
            Outcome::Expired
        );
        assert!(r.pipe.queue.repo().rows.lock().unwrap().is_empty());
        assert_eq!(r.orders.0.lock().unwrap()[&1].1, "expired");
        assert_eq!(ids(&r), Vec::<u128>::new());
    }

    #[tokio::test]
    async fn ttl_boundary_expires_at_ttl_not_before() {
        let mut r = rig(10, 0).await;
        queued(&mut r, 1, false).await;
        queued(&mut r, 2, false).await;
        *r.clock.0.lock().unwrap() += chrono::Duration::seconds(299);
        assert!(matches!(
            r.pipe.decide(IntentId(1), Approve, "u").await,
            Outcome::Placed(_)
        ));
        *r.clock.0.lock().unwrap() += chrono::Duration::seconds(1);
        assert_eq!(
            r.pipe.decide(IntentId(2), Approve, "u").await,
            Outcome::Expired
        );
    }

    #[tokio::test]
    async fn hung_quote_and_hung_send_time_out() {
        let mut r = rig(10, 0).await;
        r.pipe
            .quotes
            .1
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            r.pipe.submit(intent(1, false)).await,
            Outcome::GateBlocked("no fresh quote".into())
        );
        r.pipe
            .quotes
            .1
            .store(false, std::sync::atomic::Ordering::SeqCst);
        queued(&mut r, 2, false).await;
        r.rec.set_hang(true);
        assert_eq!(
            r.pipe.decide(IntentId(2), Approve, "u").await,
            Outcome::ExecFailed(ExecError::Unknown)
        );
        assert_eq!(r.orders.0.lock().unwrap()[&2].1, "unknown");
    }

    #[tokio::test]
    async fn submit_without_a_quote_blocks_and_actor_is_audited_and_capped() {
        let mut r = rig(10, 0).await;
        r.pipe
            .quotes
            .0
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            r.pipe.submit(intent(1, false)).await,
            Outcome::GateBlocked("no fresh quote".into())
        );
        let h = rig(10, 0).await.pipe.spawn();
        assert_eq!(
            h.decide(IntentId(1), Approve, &"x".repeat(501)).await,
            Err(PipelineError::TooLong)
        );
    }

    #[test]
    fn finished_outcomes_are_capped() {
        let mut q = ApprovalQueue::new(MemApprovals::default(), 300);
        q.done_cap = 2;
        for i in 1..=3 {
            q.finish(IntentId(i), Outcome::Declined);
        }
        assert!(matches!(q.begin(IntentId(1)), Begin::Unknown));
        assert!(matches!(q.begin(IntentId(3)), Begin::Done(_)));
    }

    #[derive(Debug, Clone)]
    enum Op {
        Submit(bool),
        Approve(usize),
        Decline(usize),
        Time(i64),
        Kill(bool),
        GateBlock(bool),
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            any::<bool>().prop_map(Op::Submit),
            (0..6usize).prop_map(Op::Approve),
            (0..6usize).prop_map(Op::Decline),
            (0..400i64).prop_map(Op::Time),
            any::<bool>().prop_map(Op::Kill),
            any::<bool>().prop_map(Op::GateBlock),
        ]
    }

    proptest! {
        /// AC1: nothing is placed after a gate Block, while killed (unless a reduce-only exit),
        /// after expiry, or without an approval.
        #[test]
        fn nothing_is_placed_unless_every_condition_holds(ops in proptest::collection::vec(op(), 1..40)) {
            let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
            rt.block_on(async {
                let mut r = rig(1000, 0).await;
                // (id, reduce_only, queued_at_secs, open)
                let mut items: Vec<(u128, bool, i64, bool)> = vec![];
                let (mut secs, mut killed, mut blocked, mut next) = (0i64, false, false, 1u128);
                let mut expected: Vec<u128> = vec![];
                for o in ops {
                    match o {
                        Op::Submit(red) => {
                            let out = r.pipe.submit(intent(next, red)).await;
                            prop_assert_eq!(out == Outcome::Queued, !blocked);
                            if !blocked { items.push((next, red, secs, true)); }
                            next += 1;
                        }
                        Op::Decline(i) => {
                            if let Some(it) = items.get_mut(i) {
                                r.pipe.decide(IntentId(it.0), Decline, "u").await;
                                it.3 = false;
                            }
                        }
                        Op::Approve(i) => {
                            if let Some(it) = items.get_mut(i) {
                                let out = r.pipe.decide(IntentId(it.0), Approve, "u").await;
                                if it.3 {
                                    let ok = secs - it.2 < 300 && !blocked && (!killed || it.1);
                                    if ok { expected.push(it.0); }
                                    prop_assert_eq!(matches!(out, Outcome::Placed(_)), ok);
                                    it.3 = false;
                                } else {
                                    prop_assert!(matches!(out, Outcome::AlreadyDecided(_)));
                                }
                            }
                        }
                        Op::Time(s) => {
                            secs += s;
                            *r.clock.0.lock().unwrap() += chrono::Duration::seconds(s);
                        }
                        Op::Kill(on) => {
                            if on { r.pipe.exec.engage("p", "prop").await.unwrap(); }
                            else if killed { r.pipe.exec.release("RELEASE KILL SWITCH", "p").await.unwrap(); }
                            killed = on;
                        }
                        Op::GateBlock(b) => {
                            blocked = b;
                            *r.gate.verdict.lock().unwrap() = if b { GateOutcome::Block("x".into()) } else { GateOutcome::Pass { risk_inr: Decimal::ZERO } };
                        }
                    }
                    prop_assert_eq!(ids(&r), expected.clone());
                }
                Ok(())
            })?;
        }
    }

    #[test]
    fn recording_executor_is_empty_at_start() {
        assert_eq!(RecordingExecutor::default().placed(), vec![]);
    }
}
