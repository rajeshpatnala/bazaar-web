//! The `OrderIntent` state machine: a pure `apply(state, event)`.

use thiserror::Error;

/// Where an order is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OrderState {
    Proposed,
    PendingApproval,
    Approved,
    Sent,
    PartiallyFilled,
    /// Send outcome not known; only the reconciler (F14) resolves it.
    Unknown,
    // Terminal states below.
    Blocked,
    Declined,
    Expired,
    Filled,
    Rejected,
    Cancelled,
}

/// What happened to an order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExecEvent {
    GatePassed,
    /// The gate, or the fresh-quote re-gate, blocked it, or the quote drifted.
    GateBlocked,
    Approve,
    Decline,
    Expire,
    /// Handed to the executor.
    Send,
    /// Kill switch, breaker or rate limiter refused the send.
    GuardBlocked,
    PartialFill,
    Fill,
    Reject,
    Cancel,
    /// The send outcome is not known.
    Lost,
}

/// A (state, event) pair that is not a valid transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("illegal transition: {event:?} in {from:?}")]
pub struct IllegalTransition {
    pub from: OrderState,
    pub event: ExecEvent,
}

impl ExecEvent {
    /// Stable label used in storage (`OrderRepo::advance` event kind).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GatePassed => "gate_passed",
            Self::GateBlocked => "gate_blocked",
            Self::Approve => "approve",
            Self::Decline => "decline",
            Self::Expire => "expire",
            Self::Send => "send",
            Self::GuardBlocked => "guard_blocked",
            Self::PartialFill => "partial_fill",
            Self::Fill => "fill",
            Self::Reject => "reject",
            Self::Cancel => "cancel",
            Self::Lost => "lost",
        }
    }
}

impl OrderState {
    /// Terminal states never change.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Blocked
                | Self::Declined
                | Self::Expired
                | Self::Filled
                | Self::Rejected
                | Self::Cancelled
        )
    }

    /// Stable label used in storage.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::PendingApproval => "pending_approval",
            Self::Approved => "approved",
            Self::Sent => "sent",
            Self::PartiallyFilled => "partially_filled",
            Self::Unknown => "unknown",
            Self::Blocked => "blocked",
            Self::Declined => "declined",
            Self::Expired => "expired",
            Self::Filled => "filled",
            Self::Rejected => "rejected",
            Self::Cancelled => "cancelled",
        }
    }

    /// Next state, or `IllegalTransition`. Pure: no clock, no I/O.
    ///
    /// # Errors
    /// `IllegalTransition` for any pair not in the table, including every event on a
    /// terminal state.
    pub const fn apply(self, event: ExecEvent) -> Result<Self, IllegalTransition> {
        use ExecEvent as E;
        use OrderState as S;
        let next = match (self, event) {
            (S::Proposed, E::GatePassed) => S::PendingApproval,
            (S::Proposed | S::PendingApproval, E::GateBlocked) | (S::Approved, E::GuardBlocked) => {
                S::Blocked
            }
            (S::PendingApproval, E::Approve) => S::Approved,
            (S::PendingApproval, E::Decline) => S::Declined,
            (S::PendingApproval, E::Expire) => S::Expired,
            (S::Approved, E::Send) => S::Sent,
            (S::Sent | S::PartiallyFilled, E::PartialFill) => S::PartiallyFilled,
            (S::Sent | S::PartiallyFilled | S::Unknown, E::Fill) => S::Filled,
            (S::Sent | S::Unknown, E::Reject) => S::Rejected,
            (S::Sent | S::PartiallyFilled | S::Unknown, E::Cancel) => S::Cancelled,
            (S::Sent | S::PartiallyFilled, E::Lost) => S::Unknown,
            (from, event) => return Err(IllegalTransition { from, event }),
        };
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATES: [OrderState; 12] = [
        OrderState::Proposed,
        OrderState::PendingApproval,
        OrderState::Approved,
        OrderState::Sent,
        OrderState::PartiallyFilled,
        OrderState::Unknown,
        OrderState::Blocked,
        OrderState::Declined,
        OrderState::Expired,
        OrderState::Filled,
        OrderState::Rejected,
        OrderState::Cancelled,
    ];
    const EVENTS: [ExecEvent; 12] = [
        ExecEvent::GatePassed,
        ExecEvent::GateBlocked,
        ExecEvent::Approve,
        ExecEvent::Decline,
        ExecEvent::Expire,
        ExecEvent::Send,
        ExecEvent::GuardBlocked,
        ExecEvent::PartialFill,
        ExecEvent::Fill,
        ExecEvent::Reject,
        ExecEvent::Cancel,
        ExecEvent::Lost,
    ];

    #[test]
    fn happy_path_reaches_filled() {
        let mut s = OrderState::Proposed;
        for e in [
            ExecEvent::GatePassed,
            ExecEvent::Approve,
            ExecEvent::Send,
            ExecEvent::PartialFill,
            ExecEvent::Fill,
        ] {
            s = s.apply(e).unwrap();
        }
        assert_eq!(s, OrderState::Filled);
    }

    #[test]
    fn terminal_states_never_change() {
        for s in STATES.into_iter().filter(|s| s.is_terminal()) {
            for e in EVENTS {
                assert_eq!(s.apply(e), Err(IllegalTransition { from: s, event: e }));
            }
        }
    }

    #[test]
    fn every_pair_is_valid_or_illegal_and_never_leaves_a_terminal_state() {
        let mut valid = 0;
        for s in STATES {
            for e in EVENTS {
                if let Ok(n) = s.apply(e) {
                    valid += 1;
                    assert!(!s.is_terminal());
                    assert_ne!(n, OrderState::Proposed);
                }
            }
        }
        assert_eq!(valid, 20);
    }

    #[test]
    fn nothing_is_sent_without_approval() {
        for s in STATES.into_iter().filter(|s| *s != OrderState::Approved) {
            assert!(s.apply(ExecEvent::Send).is_err());
        }
    }

    #[test]
    fn unknown_only_resolves_through_fill_reject_cancel() {
        assert_eq!(
            OrderState::Sent.apply(ExecEvent::Lost),
            Ok(OrderState::Unknown)
        );
        assert!(OrderState::Unknown.apply(ExecEvent::Send).is_err());
        assert_eq!(
            OrderState::Unknown.apply(ExecEvent::Reject),
            Ok(OrderState::Rejected)
        );
    }

    #[test]
    fn event_labels_are_unique() {
        let mut v: Vec<_> = EVENTS.iter().map(|e| e.as_str()).collect();
        v.sort_unstable();
        v.dedup();
        assert_eq!(v.len(), EVENTS.len());
    }

    #[test]
    fn labels_are_unique() {
        let mut v: Vec<_> = STATES.iter().map(|s| s.as_str()).collect();
        v.sort_unstable();
        v.dedup();
        assert_eq!(v.len(), STATES.len());
    }
}
