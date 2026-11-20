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

## Slice 1/2 acceptance items (from PR 96 review)
- One execution-owned kill-switch reader maps `Err` to engaged; test with an erroring repo (RISK-96-6).
- `ApprovedOrder` minted only inside the pipeline module with a gate-pass token (RISK-96-8).
- Minute cap still blocks the 5th order with the per-second cap on (RISK-96-5).
- Store test: append via `chain_hash`, then `verify_chain(read_all())`; fail closed with `Corrupt`; trigger test is a merge condition (SEC-96-3, RISK-96-9).
- Anchor the latest audit (seq, hash) outside the table (SEC-96-2, RISK-96-9).

## Pre-live checklist: all new `[execution]` keys are UNVERIFIED (owner to confirm; no `source`/`verified_on` yet)
`approval_ttl_secs`, `max_quote_drift_bps`, `max_orders_per_second`, `kill_switch_release_phrase`, `kill_allow_reduce_only`, `breaker_engages_kill_switch`, `paper_slippage_bps.*`, `paper_max_participation`, `paper_stale_after_secs`, `paper_mis_square_off_ist`, and the ceilings `hard_limits.max_approval_ttl_secs`, `hard_limits.max_quote_drift_bps` (SEC-96-4).

## Deferred
- Owner to confirm defaults (RISK-96-11): 300 s approval TTL, 50 bps drift, `breaker_engages_kill_switch = false`. All stay UNVERIFIED.
- `kill_allow_reduce_only` must be read by both the pipeline and `GuardedExecutor` (slice 1).
- Bound `paper_mis_square_off_ist` against the session calendar (CR-96-5); slice sizes for later F10 slices <= 400 lines (CR-96-6).
- Rows-only chain cannot detect tail truncation (see ADR).
