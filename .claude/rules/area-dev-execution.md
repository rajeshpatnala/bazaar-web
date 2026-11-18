---
paths:
  - "crates/execution/**"
  - "crates/paper/**"
  - "crates/broker/**"
---
# You are in dev-execution's area (bz-execution)

1. One pipeline for every mode: `Signal -> Recommendation -> OrderIntent -> validate -> DecisionGate -> Approval -> pre-send guards -> Executor -> Fill -> Position/Journal/Audit`. Paper and live differ **only** in the `Executor` implementation. No `if paper` anywhere upstream of it.
2. Default mode is `paper`. A missing, unparsable or unknown `mode.execution_mode` falls back to `paper`, never to a more permissive mode.
3. The Executor accepts only an `ApprovedOrder`, a type whose constructor is `pub(crate)` inside `execution` and which only `Pipeline` can mint from a `GatedIntent` (minted by the `risk` crate's DecisionGate, see `bz-backtest-risk`) after approval and a gate re-run. No public `place_order(raw)` exists anywhere.
4. Fail closed. Audit append fails, kill-switch read fails, gate errors, clock unavailable, margin unknown: the order is blocked and the reason is audited if possible.
5. Safety guards (kill switch, breaker, rate limiter) wrap the Executor as a decorator (`GuardedExecutor`) AND are checked in the pipeline, so a future caller cannot skip them. Tests assert both layers.
6. Never bypass the gate: no `skip_gate`, `force`, `dry_run=false` shortcuts. An override is a typed reason entered by the user (`bz-trading-discipline`), stored, audited.
7. The user is the approver in `semi_auto`. Approval is an explicit click per order showing rupee risk; no "approve all", no remembered approval, no default-focused confirm that fires on Enter from a stale dialog.
8. The paper account behaves like real cash: no negative cash, no free margin, charges debited, same rejects a broker would give. If paper is easier than live, paper results are lies.
9. State transitions are only through the state machine function; terminal states never change; every transition appends an audit entry in the same DB transaction.
10. Tests never place real orders, open broker sockets or read `.env`. Live code is exercised only against `wiremock` fakes (`bz-testing`).
11. No hardcoded market facts: tick, lot, circuit bands, freeze quantity, square-off time, broker rate limits come from the instrument master or config.

More on this area: `just docs "bz-execution: <topic>"`.
