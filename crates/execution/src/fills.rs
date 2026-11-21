//! `FillHandler` (executor reports -> order state) and the `RecordingExecutor` test double.

use std::sync::{Arc, Mutex};

use bazaar_core::ports::OrderRepo;
use chrono::DateTime;
use chrono_tz::Tz;

use crate::approval::Track;
use crate::executor::{ApprovedOrder, ExecAck, ExecError, Executor};
use crate::state::ExecEvent;
use crate::types::IntentId;

/// Moves an order to `PartiallyFilled` or `Filled` from a cumulative filled quantity.
#[derive(Debug)]
pub struct FillHandler<O: OrderRepo> {
    orders: Arc<O>,
}

impl<O: OrderRepo> FillHandler<O> {
    pub const fn new(orders: Arc<O>) -> Self {
        Self { orders }
    }

    /// `filled` is the cumulative quantity of `qty`; zero changes nothing.
    ///
    /// # Errors
    /// `ExecError::Storage` when the transition or its write fails.
    pub async fn apply(
        &self,
        track: &mut Track,
        qty: u32,
        filled: u32,
        now: DateTime<Tz>,
    ) -> Result<(), ExecError> {
        let event = if filled >= qty {
            ExecEvent::Fill
        } else if filled > 0 {
            ExecEvent::PartialFill
        } else {
            return Ok(());
        };
        track.advance(&*self.orders, event, now).await
    }
}

/// Executor that places nothing real: records ids and fills in full. For tests of every layer.
#[derive(Debug, Clone, Default)]
pub struct RecordingExecutor {
    placed: Arc<Mutex<Vec<IntentId>>>,
}

impl RecordingExecutor {
    /// Ids placed so far, in order.
    #[must_use]
    pub fn placed(&self) -> Vec<IntentId> {
        self.placed
            .lock()
            .map_or_else(|_| Vec::new(), |v| v.clone())
    }
}

#[allow(clippy::unused_async_trait_impl)] // nothing to await
impl Executor for RecordingExecutor {
    async fn place(&self, order: &ApprovedOrder) -> Result<ExecAck, ExecError> {
        self.placed
            .lock()
            .map_err(|_| ExecError::Storage)?
            .push(order.intent().id);
        Ok(ExecAck {
            intent_id: order.intent().id,
            external_id: "rec".into(),
            filled_qty: order.intent().qty,
            avg_price: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::fakes::MemOrders;
    use crate::state::OrderState;
    use bazaar_core::ports::{AuditDraft, OrderRecord};

    async fn sent(o: &MemOrders) -> Track {
        let rec = OrderRecord {
            intent_id: 1,
            version: 0,
            state: "sent".into(),
            payload: String::new(),
        };
        let d = AuditDraft {
            ts_ist_rfc3339: String::new(),
            actor: String::new(),
            kind: String::new(),
            payload: String::new(),
        };
        o.insert(rec, d, crate::audit::chain_hash).await.unwrap();
        Track {
            id: IntentId(1),
            version: 0,
            state: OrderState::Sent,
        }
    }

    #[tokio::test]
    async fn partial_then_full_fill_and_zero_is_a_no_op() {
        let o = Arc::new(MemOrders::default());
        let mut t = sent(&o).await;
        let h = FillHandler::new(o.clone());
        let now = crate::pipeline::fakes::start();
        h.apply(&mut t, 10, 0, now).await.unwrap();
        assert_eq!(t.state, OrderState::Sent);
        h.apply(&mut t, 10, 4, now).await.unwrap();
        assert_eq!(t.state, OrderState::PartiallyFilled);
        h.apply(&mut t, 10, 10, now).await.unwrap();
        assert_eq!((t.state, t.version), (OrderState::Filled, 2));
        // Terminal: a further fill is an illegal transition, reported as storage.
        assert_eq!(h.apply(&mut t, 10, 10, now).await, Err(ExecError::Storage));
    }

    #[tokio::test]
    async fn stale_version_loses() {
        let o = Arc::new(MemOrders::default());
        let mut t = sent(&o).await;
        t.version = 5;
        let h = FillHandler::new(o);
        assert_eq!(
            h.apply(&mut t, 1, 1, crate::pipeline::fakes::start()).await,
            Err(ExecError::Storage)
        );
    }
}
