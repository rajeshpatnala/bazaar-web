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
        let next = self.state.apply(event).map_err(|_| ExecError::Storage)?;
        let audit = bazaar_core::ports::AuditDraft {
            ts_ist_rfc3339: now.to_rfc3339(),
            actor: actor.to_owned(),
            kind: event.as_str().into(),
            payload: serde_json::json!({ "intent": self.id.to_string(), "state": next.as_str(), "actor": actor, "at": now.to_rfc3339() })
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

    /// Persist the user's decision; the repo keeps the first one.
    ///
    /// # Errors
    /// The repo's error.
    pub async fn record(
        &self,
        id: IntentId,
        d: ApprovalDecision,
        now: DateTime<Tz>,
    ) -> Result<ApprovalDecision, RepoError> {
        self.repo.decide(id.0, d, now.to_rfc3339()).await
    }
}
