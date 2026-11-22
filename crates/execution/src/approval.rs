//! `ApprovalQueue`: intents waiting for a user decision, first decision wins, TTL expiry.
//! Owned by the single-writer pipeline, so no lock is needed.

use std::collections::{HashMap, VecDeque};

use bazaar_core::ports::{ApprovalDecision, ApprovalRepo, OrderRepo, RepoError};
use chrono::{DateTime, Duration};
use chrono_tz::Tz;
use rust_decimal::Decimal;

use crate::audit::chain_hash;
use crate::executor::{ExecAck, ExecError};
use crate::state::{ExecEvent, OrderState};
use crate::types::{IntentId, OrderIntent};

/// How one intent ended (or, for `Queued`, where it waits).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Queued,
    Declined,
    Expired,
    /// The gate (submit or approval-time re-run) blocked it; the text is the gate's reason.
    GateBlocked(String),
    /// The fresh quote moved past the drift limit; `new_risk` is the re-computed rupee risk.
    QuoteDrifted {
        new_risk: Decimal,
    },
    /// Kill switch, breaker or rate limiter refused the send.
    GuardBlocked(ExecError),
    Placed(ExecAck),
    /// Sent, but the fill could not be written; reconciliation (F14) repairs the stored state.
    PlacedUnrecorded(ExecAck),
    /// The executor refused or failed after the send.
    ExecFailed(ExecError),
    /// A second decision on a finished intent gets the first outcome.
    AlreadyDecided(Box<Outcome>),
    Error(ExecError),
}

/// Persisted position of one order: version for the optimistic check, current state.
#[derive(Debug, Clone, Copy)]
pub struct Track {
    pub id: IntentId,
    pub version: u32,
    pub state: OrderState,
}

impl Track {
    /// Apply `event` and persist state, event and audit row in one repo transaction (write-ahead).
    ///
    /// # Errors
    /// `ExecError::Storage` for an illegal transition or a failed or stale write.
    pub async fn advance<O: OrderRepo>(
        &mut self,
        orders: &O,
        event: ExecEvent,
        now: DateTime<Tz>,
    ) -> Result<(), ExecError> {
        self.advance_as(orders, event, now, "pipeline").await
    }

    /// Like [`Self::advance`], recording who caused the event (the approver for approve/decline).
    ///
    /// # Errors
    /// `ExecError::Storage` for an illegal transition or a failed or stale write.
    pub async fn advance_as<O: OrderRepo>(
        &mut self,
        orders: &O,
        event: ExecEvent,
        now: DateTime<Tz>,
        actor: &str,
    ) -> Result<(), ExecError> {
        self.advance_why(orders, event, now, actor, None).await
    }

    /// Like [`Self::advance_as`], with a `reason` carried in the audit payload (restart reload
    /// records why it expired or blocked an order).
    ///
    /// # Errors
    /// `ExecError::Storage` for an illegal transition or a failed or stale write.
    pub async fn advance_why<O: OrderRepo>(
        &mut self,
        orders: &O,
        event: ExecEvent,
        now: DateTime<Tz>,
        actor: &str,
        reason: Option<&str>,
    ) -> Result<(), ExecError> {
        let next = self.state.apply(event).map_err(|_| ExecError::Storage)?;
        let audit = bazaar_core::ports::AuditDraft {
            ts_ist_rfc3339: now.to_rfc3339(),
            actor: actor.to_owned(),
            kind: event.as_str().into(),
            payload: serde_json::json!({ "intent": self.id.to_string(), "state": next.as_str(), "actor": actor, "at": now.to_rfc3339(), "reason": reason })
                .to_string(),
        };
        let version = orders
            .advance(
                self.id.0,
                self.version,
                next.as_str().into(),
                event.as_str().into(),
                audit,
                chain_hash,
            )
            .await
            .map_err(|_| ExecError::Storage)?;
        self.version = version;
        self.state = next;
        Ok(())
    }
}

/// A queued intent.
#[derive(Debug, Clone)]
pub struct Pending {
    pub intent: OrderIntent,
    pub track: Track,
    pub queued_at: DateTime<Tz>,
}

/// What `begin` found for an intent id.
#[derive(Debug)]
pub enum Begin {
    /// Taken out of the queue; the caller must `finish` it.
    Live(Box<Pending>),
    Done(Outcome),
    Unknown,
}

/// Queue plus the outcomes of finished intents.
#[derive(Debug)]
pub struct ApprovalQueue<A: ApprovalRepo> {
    repo: A,
    ttl: Duration,
    pending: HashMap<IntentId, Pending>,
    done: HashMap<IntentId, Outcome>,
    done_order: VecDeque<IntentId>,
    pub(crate) done_cap: usize,
}

/// Finished outcomes kept for `AlreadyDecided` (oldest evicted first).
pub const MAX_DONE: usize = 10_000;

impl<A: ApprovalRepo> ApprovalQueue<A> {
    /// `ttl_secs` is `execution.approval_ttl_secs`.
    pub fn new(repo: A, ttl_secs: u32) -> Self {
        Self {
            repo,
            ttl: Duration::seconds(i64::from(ttl_secs)),
            pending: HashMap::new(),
            done: HashMap::new(),
            done_order: VecDeque::new(),
            done_cap: MAX_DONE,
        }
    }

    /// Test access to the approval repo (fakes inspect and inject failures through it).
    #[cfg(test)]
    pub(crate) fn repo(&self) -> &A {
        &self.repo
    }

    /// Startup guard: expire (audited) every pending order that already has a decision row, so
    /// it is never re-presented. Returns the ids.
    ///
    /// # Errors
    /// `RepoError` when the repo fails; the caller must not reload on an error.
    pub(crate) async fn reconcile_strays(&self, now: DateTime<Tz>) -> Result<Vec<u128>, RepoError> {
        self.repo
            .reconcile_stray_decisions(&now.to_rfc3339(), chain_hash)
            .await
    }

    /// True when `created` (an intent's creation time, the queue time of a reloaded intent) has
    /// reached the TTL at `now`.
    #[must_use]
    pub(crate) fn ttl_passed(&self, created: DateTime<Tz>, now: DateTime<Tz>) -> bool {
        now - created >= self.ttl
    }

    /// True when the intent is queued or already finished (reload skips such ids).
    #[must_use]
    pub(crate) fn knows(&self, id: IntentId) -> bool {
        self.pending.contains_key(&id) || self.done.contains_key(&id)
    }

    /// Drop every queued (not finished) intent; used when a reload fails half way.
    pub(crate) fn clear_pending(&mut self) {
        self.pending.clear();
    }

    pub fn enqueue(&mut self, p: Pending) {
        self.pending.insert(p.track.id, p);
    }

    /// True when `p` has reached the TTL at `now`.
    #[must_use]
    pub fn is_expired(&self, p: &Pending, now: DateTime<Tz>) -> bool {
        now - p.queued_at >= self.ttl
    }

    /// Take the intent out of the queue, or report its finished outcome.
    pub fn begin(&mut self, id: IntentId) -> Begin {
        if let Some(p) = self.pending.remove(&id) {
            return Begin::Live(Box::new(p));
        }
        self.done
            .get(&id)
            .cloned()
            .map_or(Begin::Unknown, Begin::Done)
    }

    pub fn finish(&mut self, id: IntentId, outcome: Outcome) {
        if self.done.insert(id, outcome).is_none() {
            self.done_order.push_back(id);
        }
        while self.done_order.len() > self.done_cap {
            if let Some(old) = self.done_order.pop_front() {
                self.done.remove(&old);
            }
        }
    }

    /// Ids past the TTL at `now`, oldest id first.
    #[must_use]
    pub fn stale(&self, now: DateTime<Tz>) -> Vec<IntentId> {
        let mut ids: Vec<_> = self
            .pending
            .values()
            .filter(|p| self.is_expired(p, now))
            .map(|p| p.track.id)
            .collect();
        ids.sort();
        ids
    }

    /// Record the user's decision, move the order and write the audit row in ONE repo
    /// transaction (the order must still be pending at the track's version). On success the
    /// track holds the new state and version; on any error it is unchanged and nothing is stored.
    ///
    /// # Errors
    /// `RepoError::Conflict` when the order is not pending, was already decided, or the version
    /// is stale. A Conflict does NOT prove an Approve exists: re-read the order before acting.
    pub(crate) async fn decide_and_advance(
        &self,
        track: &mut Track,
        d: ApprovalDecision,
        event: ExecEvent,
        now: DateTime<Tz>,
        actor: &str,
        risk_inr: Option<rust_decimal::Decimal>,
    ) -> Result<(), RepoError> {
        let next = track.state.apply(event).map_err(|_| RepoError::Conflict)?;
        let audit = bazaar_core::ports::AuditDraft {
            ts_ist_rfc3339: now.to_rfc3339(),
            actor: actor.to_owned(),
            kind: event.as_str().into(),
            payload: serde_json::json!({ "intent": track.id.to_string(), "state": next.as_str(), "actor": actor, "at": now.to_rfc3339(), "risk_inr": risk_inr.map(|r| r.to_string()) })
                .to_string(),
        };
        let version = self
            .repo
            .decide_and_advance(
                track.id.0,
                d,
                actor,
                track.version,
                next.as_str().into(),
                event.as_str().into(),
                audit,
                chain_hash,
            )
            .await?;
        track.version = version;
        track.state = next;
        Ok(())
    }
}
