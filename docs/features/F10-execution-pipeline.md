# F10: Execution pipeline

Plan: `.agents/plans/F10-execution-pipeline.md`. Status: in progress.

## Slice 0 (shared contracts)
- `core::ports`: `IdGen`, `OrderRepo`, `ApprovalRepo`, `AuditRepo`, `KillSwitchRepo`, `PaperLedgerRepo`, `MarginSource`, `QuoteSource`, plain-data records, `RepoError::Conflict`.
- `config/risk.toml` `[execution]`: approval, drift, order-rate, kill-switch phrase and paper keys. All new values are UNVERIFIED, owner to confirm.
- `execution`: `IntentId`, `OrderIntent`, `OrderState::apply`/`ExecEvent`, `ExecError`, `Executor`, `ApprovedOrder` (`pub(crate)` constructor), audit hash and `verify_chain`.
- ADR: `docs/adr/0001-audit-hash-chain.md`.

## Ordering rule for slice 1
TTL check, fresh-quote re-gate and drift check happen while the order is still `PendingApproval`
(a failure ends `Blocked` via `GateBlocked`), then `Approve` and `Send`. `Approved` only exits via
`Send` or `GuardBlocked`.

## Slice 1 (kill switch + chain audit sink)
- `execution::killswitch`: `KillSwitch<R>`; `admits(reduce_only)` is the one decision for pipeline and `GuardedExecutor`; unreadable state refuses everything incl. reduce-only (RISK-96-6). `engage`/`release` write state + audit row together; wrong phrase writes nothing.
- `execution::sink`: `ChainAuditSink` (`risk::AuditSink`) over `AuditRepo` via a writer task and bounded `recv_timeout`; needs a multi-thread runtime or `spawn_blocking`, else it fails closed by timeout.
- Fake-repo chain test: sink + kill-switch rows verify with `verify_chain`; wrong hash fails closed (SEC-96-3, fake part).

## Slice 2 (guards)
- `ratelimit::RateLimiter` (per-second, per-minute, per-IST-day; all caps always apply, RISK-96-5), `breaker::DailyLossBreaker` (`PnlSource`, latched to the next trading day, fail closed), `guarded::{SafetyGuard, GuardedExecutor}` (kill switch, breaker, rate limit; second layer before `inner.place`).
- Kill switch: release of a latched-never-persisted engage writes a row (RISK-100-10); repo error category logged (CR-100-5); fake `fail_state_write` (TS-100-3).

## Open acceptance items (from PR 96 review)
- `ApprovedOrder` minted only inside the pipeline module with a gate-pass token (RISK-96-8, slice 3).
- Real-store trigger/`Corrupt` test is a slice 7 merge condition (SEC-96-3, RISK-96-9).

## Pre-live checklist: all new `[execution]` keys are UNVERIFIED (owner to confirm; no `source`/`verified_on` yet)
`approval_ttl_secs`, `max_quote_drift_bps`, `max_orders_per_second`, `kill_switch_release_phrase`, `kill_allow_reduce_only`, `breaker_engages_kill_switch`, `paper_slippage_bps.*`, `paper_max_participation`, `paper_stale_after_secs`, `paper_mis_square_off_ist`, and the ceilings `hard_limits.max_approval_ttl_secs`, `hard_limits.max_quote_drift_bps` (SEC-96-4).

## Deferred
- RISK-100-10 fixed in slice 2. SEC-100-4: cap `actor`/`reason` at 500 chars at the `PipelineHandle` SetKillSwitch edge (slice 3). SEC-100-5, CR-100-2: gate called via `spawn_blocking` (slice 3). TS-100-3 fixed in slice 2.
- Breaker latch is in-process (lost on restart); breaker uses `Segment::Equity` for the trading-day calendar.
- defer: slice 8 audit anchor (hard readiness condition: no paper day counts until it merges).
- Real-SQLite proof lives in slices 6 (AC3), 7 (AC4 triggers, chain verify on reopen) and 8 (AC4 on real chain, AC8 composition); T2 tester checks these on a temp DB.
- Kill-switch in-process latch (set on every `engage`, cleared by a successful `release`) is lost on restart; only the persisted state survives.
- RISK-100-2: slices 2/3 derive `reduce_only` from the `ApprovedOrder`, not request data, with a test that a non-exit intent is refused when engaged. SEC-100-4: cap `actor`/`reason` length at the route (later slice). SEC-100-5: slice 3 runs the gate in `spawn_blocking` and tests it.
- Owner to confirm defaults (RISK-96-11): 300 s approval TTL, 50 bps drift, `breaker_engages_kill_switch = false`. All stay UNVERIFIED.
- `GuardedExecutor` (slice 2) and the pipeline (slice 3) must both call `KillSwitch::admits`.
- Defer RISK-96-8 to slice 3; RISK-96-5 to slice 2; SEC-96-2 anchor of latest (seq, hash) to slice 8; CR-96-5 to slice 5.
- Bound `paper_mis_square_off_ist` against the session calendar (CR-96-5); slice sizes for later F10 slices <= 400 lines (CR-96-6).
- Rows-only chain cannot detect tail truncation (see ADR).
