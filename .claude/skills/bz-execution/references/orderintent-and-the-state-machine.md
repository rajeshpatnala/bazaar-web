# 3. OrderIntent and the state machine

(From `bz-execution` SKILL.md; open when the task touches this area.)


`OrderIntent` is the immutable *proposal*: id (UUIDv7), `recommendation_id`, instrument, side, qty, order type/price/trigger, product (`CNC|MIS|NRML`), stop and target, `risk_rupees`, `expires_at`, `mode_at_creation`, optional `override_reason`. Mutable progress lives in `OrderState`.

```
Created --validate/gate fail--> Blocked(reasons)                       terminal
Created --gate ok--> AwaitingApproval --user declines--> Declined      terminal
                          |--ttl elapsed--> Expired                    terminal
                          |--recheck at approval fails--> Blocked
                          `--approve--> Approved --guards fail--> Blocked
                                           `--send--> Submitting
Submitting --ack--> Working --fill--> PartiallyFilled --fill--> Filled terminal
Working|PartiallyFilled --cancel--> Cancelled                          terminal (keeps fills)
Submitting|Working --broker reject--> Rejected(reason)                 terminal
Submitting --timeout/unknown error--> Unknown --reconcile--> Working|Filled|Rejected|Cancelled
```

```rust
pub enum OrderState { Created, Blocked(Vec<BlockReason>), AwaitingApproval { since: Ts },
    Declined, Expired, Approved(ApprovalRef), Submitting { tag: ClientTag },
    Working { broker_id: BrokerOrderId }, PartiallyFilled { filled: Qty, avg: Price },
    Filled { avg: Price }, Cancelled { filled: Qty }, Rejected(String), Unknown }

impl OrderState {
    /// Pure and total: every (state, event) pair is either a valid next state or Err.
    pub fn apply(self, ev: &OrderEvent) -> Result<OrderState, IllegalTransition> { /* match */ }
}
```

Rules: `apply` is pure (no IO) so it is property-testable; a repository `advance(intent_id, event)` loads, applies, persists state + event + audit row in one transaction with optimistic version check. `Unknown` is the only state allowed after an ambiguous send and **never** auto-retries (reconciliation, section 5 of `references/live-phase.md`).
