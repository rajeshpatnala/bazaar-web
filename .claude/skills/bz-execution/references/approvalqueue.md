# 5. ApprovalQueue

(From `bz-execution` SKILL.md; open when the task touches this area.)


```rust
pub struct PendingApproval { intent: OrderIntent, gate_snapshot: GateVerdict, created: Ts, expires: Ts }
#[async_trait] pub trait ApprovalQueue {
    async fn enqueue(&self, i: OrderIntent, g: GateVerdict) -> Result<ApprovalId>;
    async fn list_pending(&self) -> Result<Vec<PendingApproval>>;
    // the queue fetches the fresh quote through its own quote/MarginSource ports; handlers pass only these
    async fn approve(&self, id: ApprovalId, by: Approver, over: Option<Override>) -> Result<ApprovalOutcome, ApprovalError>;
    async fn decline(&self, id: ApprovalId, reason: Option<String>) -> Result<()>;
    async fn expire_stale(&self) -> Result<Vec<ApprovalId>>;      // run on a timer and before every list
}
```

- Expiry: TTL from `risk.toml` `[execution] approval_ttl_secs` (short, minutes for intraday). Expired intents can never be approved; the user re-runs the recommendation.
- On `approve`, **re-run the DecisionGate with a fresh quote** and `now`: entry-zone, risk rupees from current price, daily-loss room, positions, session open, event/ban rules. A failure returns `Blocked`, not a silent send. If price moved beyond the configured drift limit the dialog must re-render with the new rupee risk and require another click.
- Approve is single use: a second click on the same id returns `ApprovalError::AlreadyDecided(existing_outcome)` (HTTP 409 `approval_already_decided` carrying the outcome), never a second order. Enforce with a unique index on `approvals(intent_id)`.
- `ApprovalError` variants map one-to-one to the API problem codes: `AlreadyDecided` (`approval_already_decided`), `Expired` (`approval_expired`), `QuoteDrifted { new_risk }` (`quote_drifted`), `GateBlocked(GateVerdict)` (`gate_blocked`), `OverrideInvalid` (`override_invalid`), `KillSwitchEngaged` (`kill_switch_engaged`), `QuoteUnavailable` (`quote_unavailable`), `BrokerSessionExpired` (`broker_session_expired`), `RateLimited` (`rate_limited`), `NotFound` (`not_found`).
- Approvals are bound to the exact intent hash (qty, price, side, instrument). Editing any field creates a new intent.
- The UI contract (dialog content, PAPER/LIVE badges) belongs to `bz-web-ui`; the HTTP handlers (`POST /api/v1/order-intents`, `POST /api/v1/approvals/{id}/approve { override? }`, `POST /api/v1/approvals/{id}/reject`, `PUT /api/v1/kill-switch`; table in `bz-web-api` `references/api-contract.md`) stay thin and call `ApprovalQueue`. There is no "place order" endpoint.
- Approve needs a valid session and the CSRF token; `by: Approver` is the logged-in owner from that session and is written to the audit log as `actor = User`. Pending approvals live in SQLite, so they survive a server restart, and still expire by `approval_ttl_secs` (`expire_stale` runs at startup).
