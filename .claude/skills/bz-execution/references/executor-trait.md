# 6. Executor trait

(From `bz-execution` SKILL.md; open when the task touches this area.)


This crate owns the trait and the state machine (section 3); `bz-architecture` and `references/live-phase.md` link here and do not restate them.

```rust
#[async_trait]
pub trait Executor: Send + Sync {
    fn kind(&self) -> ExecutorKind;                                   // Paper | Live, for labels and audit
    async fn place(&self, o: &ApprovedOrder) -> Result<ExecAck, ExecError>;   // idempotent on o.tag
    async fn cancel(&self, id: &BrokerOrderId) -> Result<(), ExecError>;
    async fn modify(&self, id: &BrokerOrderId, m: &Modify) -> Result<(), ExecError>;
    async fn open_orders(&self) -> Result<Vec<BrokerOrder>, ExecError>;    // for reconciliation
    async fn positions(&self) -> Result<Vec<BrokerPosition>, ExecError>;
    fn events(&self) -> tokio::sync::broadcast::Receiver<ExecEvent>;   // Ack, Fill, PartialFill, Reject, Cancel, SquareOff
}
```

Margin is not on `Executor`: the pipeline asks the single `MarginSource` port (`core::ports`; Kite margins API for live and paper (see `references/paper-fills.md`), fixed fake in tests). `Err(Unavailable)` blocks the order; never estimate by hand.

`ExecError` is a `thiserror` enum mirroring broker classes (`InsufficientMargin`, `InsufficientHoldings`, `InvalidInput`, `RateLimited`, `SessionExpired`, `Network`, `Rejected(String)`, `Unavailable`) so paper can return the same errors live would. Fills arrive as `ExecEvent`s consumed by one `FillHandler` that updates positions, journal and audit; it is the same code for paper and live.
