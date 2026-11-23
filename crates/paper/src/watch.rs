//! Tracks square-offs that are still blocked, so the caller can alarm (RISK-119-6).
//!
//! A blocked MIS square-off leaves a position open. The watch is fed every [`TickOutcome`] and
//! [`ClockOutcome`] and says whether any square-off is still outstanding: an order stays blocked
//! until a fill that COMPLETES it is booked (a partial fill leaves the position open, so it does
//! not clear it). `ClockOutcome` carries no fills, so clock passes never clear a blocked order.

use std::collections::HashSet;

use bazaar_execution::types::IntentId;

use crate::executor::{ClockOutcome, TickOutcome};

/// Outstanding blocked square-offs.
#[derive(Debug, Default)]
pub struct SquareOffWatch {
    blocked: HashSet<IntentId>,
    /// The last clock call could not queue a square-off for some symbol.
    queue_failed: bool,
}

impl SquareOffWatch {
    /// Apply one tick's outcome.
    pub fn on_tick(&mut self, out: &TickOutcome) {
        for f in out.fills.iter().filter(|f| f.complete) {
            self.blocked.remove(&f.id);
        }
        for (id, _) in &out.blocked_square_offs {
            self.blocked.insert(*id);
        }
    }

    /// Apply one clock call's outcome (the run is repeated until nothing fails).
    pub fn on_clock(&mut self, out: &ClockOutcome) {
        self.queue_failed = !out.square_off.failed.is_empty();
    }

    /// Number of square-offs still outstanding (blocked orders plus a failed queueing).
    #[must_use]
    pub fn outstanding(&self) -> usize {
        self.blocked.len() + usize::from(self.queue_failed)
    }
}
