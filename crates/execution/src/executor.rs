//! The `Executor` trait and `ApprovedOrder`, the only thing an executor accepts.

use std::future::Future;

use rust_decimal::Decimal;
use thiserror::Error;

use crate::types::{IntentId, OrderIntent};

/// An order that passed the gate, the approval, the gate re-run and the guards.
///
/// The constructor is `pub(crate)`: only the pipeline inside `execution` can mint one, so no
/// caller can reach an `Executor` with a raw order.
///
/// ```compile_fail
/// use bazaar_execution::executor::ApprovedOrder;
/// fn forge(i: bazaar_execution::types::OrderIntent) -> ApprovedOrder { ApprovedOrder::new(i) }
/// ```
#[derive(Debug, PartialEq, Eq)]
pub struct ApprovedOrder {
    intent: OrderIntent,
}

impl ApprovedOrder {
    #[allow(dead_code)] // first caller is the pipeline (slice 1)
    pub(crate) const fn new(intent: OrderIntent) -> Self {
        Self { intent }
    }

    /// The approved intent.
    #[must_use]
    pub const fn intent(&self) -> &OrderIntent {
        &self.intent
    }
}

/// What the executor reports after a send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecAck {
    pub intent_id: IntentId,
    /// Broker or paper order id.
    pub external_id: String,
    pub filled_qty: u32,
    pub avg_price: Option<Decimal>,
}

/// Why an order did not reach, or was refused by, the executor.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ExecError {
    /// Lot, tick, freeze or quantity rule failed; the text names the rule, no data.
    #[error("invalid input: {0}")]
    InvalidInput(&'static str),
    #[error("kill switch engaged")]
    KillSwitch,
    #[error("daily-loss breaker tripped")]
    BreakerTripped,
    #[error("rate limit reached")]
    RateLimited,
    #[error("insufficient funds")]
    InsufficientFunds,
    /// Margin source could not answer; never estimated.
    #[error("margin unavailable")]
    MarginUnavailable,
    /// The send outcome is not known; the order becomes `Unknown`.
    #[error("outcome unknown")]
    Unknown,
    #[error("storage error")]
    Storage,
}

/// The final step of the pipeline; paper and live differ only here.
pub trait Executor: Send + Sync {
    /// Place one approved order.
    fn place(
        &self,
        order: &ApprovedOrder,
    ) -> impl Future<Output = Result<ExecAck, ExecError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{OrderKind, Product};
    use bazaar_risk::types::Side;
    use chrono::TimeZone;
    use chrono_tz::Asia::Kolkata;

    struct Recording;
    impl Executor for Recording {
        fn place(
            &self,
            o: &ApprovedOrder,
        ) -> impl Future<Output = Result<ExecAck, ExecError>> + Send {
            std::future::ready(Ok(ExecAck {
                intent_id: o.intent().id,
                external_id: "x".into(),
                filled_qty: o.intent().qty,
                avg_price: None,
            }))
        }
    }

    #[test]
    fn executor_takes_only_an_approved_order() {
        let intent = OrderIntent {
            id: IntentId(7),
            symbol: "ABC".into(),
            side: Side::Buy,
            qty: 3,
            kind: OrderKind::Market,
            product: Product::Cnc,
            reduce_only: false,
            created_at: Kolkata.with_ymd_and_hms(2026, 1, 5, 10, 0, 0).unwrap(),
        };
        let approved = ApprovedOrder::new(intent);
        let fut = Recording.place(&approved);
        let ack = std::pin::pin!(fut);
        let waker = std::task::Waker::noop();
        let mut cx = std::task::Context::from_waker(waker);
        let polled = ack.poll(&mut cx);
        assert!(matches!(polled, std::task::Poll::Ready(Ok(_))));
        let std::task::Poll::Ready(Ok(ack)) = polled else {
            return;
        };
        assert_eq!((ack.intent_id, ack.filled_qty), (IntentId(7), 3));
    }

    #[test]
    fn errors_have_safe_messages() {
        assert_eq!(ExecError::KillSwitch.to_string(), "kill switch engaged");
        assert_eq!(
            ExecError::InvalidInput("lot size").to_string(),
            "invalid input: lot size"
        );
    }
}
