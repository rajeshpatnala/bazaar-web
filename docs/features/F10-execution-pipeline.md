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
- Fix round (PR 105): `SafetyGuard::peek` (non-consuming, for the pipeline) vs crate-private `check` (consuming; only `GuardedExecutor::place`), so one order uses one slot (CR-105-1, RISK-105-3). `reduce_only` skips the per-minute and per-day caps and never consumes the entry budget; exits keep only a per-second loop guard on `max_orders_per_second`, no new key (CR-105-2, RISK-105-2). The breaker latch is restored at construction from the `breaker_tripped` audit rows (`until` in the payload), so a restart on a tripped day stays tripped (RISK-105-1, SEC-105-1: restart-loss of the latch is closed). `DailyLossBreaker::new` is async, takes `now` and the hard ceiling, rejects `max_loss_pct` outside `(0, ceiling]` and fails if the audit log cannot be read. A failed trip audit append is retried on later checks (`audit_pending`, RISK-105-5); trip payload carries pnl, equity base, limit (CR-105-5); the latch is set compare-and-set so concurrent checks write one row (CR-105-3). `equity_base` is start-of-day capital (RISK-105-4). A same-day kill-switch release while tripped is re-engaged and audited on the next check (RISK-105-6). The breaker calendar is equity-only (intent has no segment) and calendar errors are logged (RISK-105-7).
- Kill switch: release of a latched-never-persisted engage writes a row (RISK-100-10); repo error category logged (CR-100-5); fake `fail_state_write` (TS-100-3).

## Open acceptance items (from PR 96 review)
- `ApprovedOrder` minted only inside the pipeline module with a gate-pass token (RISK-96-8, slice 3).
- Real-store trigger/`Corrupt` test is a slice 7 merge condition (SEC-96-3, RISK-96-9).

## Pre-live checklist: all new `[execution]` keys are UNVERIFIED (owner to confirm; no `source`/`verified_on` yet)
`approval_ttl_secs`, `max_quote_drift_bps`, `max_orders_per_second`, `kill_switch_release_phrase`, `kill_allow_reduce_only`, `breaker_engages_kill_switch`, `paper_slippage_bps.*`, `paper_max_participation`, `paper_stale_after_secs`, `paper_mis_square_off_ist`, and the ceilings `hard_limits.max_approval_ttl_secs`, `hard_limits.max_quote_drift_bps` (SEC-96-4).

## Paper account (slice 4, `paper::account`)
Cash is derived by replaying the current epoch's append-only ledger (kinds opening, buy, sell, charge); a fill books its charge and trade lines in one all-or-nothing `PaperLedgerRepo::append_many`. Charges come only from `ChargeSource` (zero or negative is refused). The funds check refuses a fill that would take cash below zero; a sell above the held quantity (passed in by the caller from the position book) is refused. `reset` uses only the configured `paper_starting_cash_inr`. One `PaperAccount` per ledger (single writer).

## Slice 5 (paper fills, `paper::{fill,executor,squareoff}`)
`FillSimulator` is pure and seeded (splitmix64; the seed only scales partial sizes to 50-100% of the participation cap). Market orders fill on the first tick strictly after acceptance, slipped against the trader and tick-aligned, clamped inside the circuits; limit orders fill at the limit only when the tick trades strictly through; no fill on stale or future ticks (order flagged), outside the normal session (decided inside the executor from the injected `MarketCalendar`, never by the caller; calendar error = closed), or for a buy at the upper / sell at the lower circuit. `PaperExecutor::place` validates (lot, tick, freeze, position net of working sells, funds estimate) and never fills; fills book through `PaperAccount::apply_fill`; a square-off fill is audited write-ahead (audit failure undoes the fill, it retries on the next tick). `SquareOff::on_clock` cancels working MIS orders and queues opposite market orders once per IST date. The square-off time is a `ValidatedSquareOffTime`, only buildable against the calendar for every segment in use (RISK-119-3); a carried MIS position is queued at the first clock call of a new date (RISK-119-10); a failed queue (missing spec, overflow) is reported and retried. Working buys reserve funds at placement (RISK-119-8). Cancellations, DAY expiries, square-off fills and square-off failures are audited (RISK-119-5); a square-off is never dropped on a ledger or charge error, it retries. `accept` is crate-private; `place(&ApprovedOrder)` is the only public entry (RISK-119-1).

RISK-119-6 decision: paper keeps the audit write-ahead and fails closed (no reduce-only bypass). A square-off blocked by an audit outage appears in `TickOutcome::blocked_square_offs`, is logged at error each tick, and fills on the first tick after the audit recovers; the wiring must alarm on any entry.
Limits: MIS/NRML use full cash (no margin, no leverage), intraday short selling is refused, AMO is not modelled, DAY expiry is by date change only. Tests cannot mint `ApprovedOrder` outside `execution`, so `place` itself is covered through `accept`. `execution::types` now re-exports `Side` (paper may not depend on `risk`).

## Slice 6 (store repos, BZ-203)
- Migrations 0003 orders (+ append-only `order_events`), 0004 approvals (unique index on `order_id`, first decision wins, append-only), 0005 kill_switch (single row, seeded released, undeletable). 0002 was taken by candles.
- `SqliteStore` methods (labels are opaque text; store depends on core only): `order_create/get/orders_in_state/order_events`, `order_advance` (version check + event + audit row in one transaction, via `OrderTransition`), `approval_decide` (CR-123-1: approval row, order advance, event and audit row in ONE transaction; unique `order_id` so the first decision wins, a second gets `Conflict`; the bare insert is test-only), `approvals_reconcile` (startup guard: a still-pending order that has an approval row is expired with an `order.expired_inconsistent` audit row and never re-presented), `approval_get`, `kill_switch_get/set`. Stale version or duplicate decision is `RepoError::Conflict`; a missing kill-switch row is `Corrupt` (callers fail closed). Empty ids/actor/reason map to `Other`, over-long to `TooLarge`; `orders_in_state` is capped (10 000, overflow is `TooLarge`).
- Migrations 0006 `audit_log` (hash chain per ADR-0001, append-only triggers; created here because the atomic audit write needs it, slice 7 reuses it and owns verification) and 0007 `kill_switch_events` (SEC-123-1: append-only history written in the same transaction as each set) plus an `orders` no-delete trigger. T0 passed after BZ-106 (system DuckDB).

## Deferred
- SEC-123-4 fixed in BZ-204 (continuity trigger 0008, field caps, `open_verified`). Chain verify on open is opt-in: the composition slice must call `SqliteStore::open_verified` with the execution chain fn. `append_many` implemented as `ledger_append_many` (migration 0009). TS-123-5 covered by a rewind-to-v7 reopen test.
- TS-123-5 -> next store slice (migration-on-existing-DB test). TS-123-1: real multi-connection races are unprovable while the pool is max_connections(1); revisit if that changes.
- Slice 3 review (PR 109) carry-overs for slice 8: RISK-109-3 - the real `Gate` adapter must check `reduce_only` against the live position and Block otherwise (adapter test: an entry with the flag set is blocked while killed); the flag stays unreachable from any endpoint until then. RISK-109-7 - the adapter and its test must treat a `None` quote as Block for entries (G15). TS-109-1 - add Drift and ExpireStale ops to the AC1 property test if cheap.
- Slice 4 follow-ups (RISK-113-3, RISK-113-6): the wiring slice must audit and journal every `PaperAccount::reset` with a typed reason, and keep daily/weekly loss-limit state across an epoch reset (or refuse reset during a lockout). Slice 5 owns MIS/NRML margin and intraday short selling (an explicit acceptance item). The store slice (7) implements `append_many` in one SQL transaction.
- Slice 3 pipeline gate is a local `GatePort` (Pass / Block / Drifted). Slice 8 (AC8) owns the real-`Gate` adapter: building `PortfolioSnapshot` and `GateInputs`, mapping `execution::OrderIntent` to `risk::OrderIntent`, and the drift check. `expire_stale` covers the in-memory queue only; reloading pending intents after a restart needs an `OrderRepo` list method (slice 6/8). The guard `peek` and `GuardedExecutor::place` both read the kill switch, breaker and limiter, so a guard flip between them is refused by the second layer (outcome `ExecFailed`).
- RISK-109-fix: `risk_inr` is missing from the approval audit payload (needs the slice 8 adapter; `GatePort::Pass` carries no risk figure). `io_timeout` needs a config key for the composition slice (suggest `execution.io_timeout_secs`, with `source`/`verified_on`); the pipeline clamps it (zero becomes 5 s, ceiling 30 s) until then.
- RISK-100-10 fixed in slice 2. SEC-100-4: cap `actor`/`reason` at 500 chars at the `PipelineHandle` SetKillSwitch edge (slice 3). SEC-100-5, CR-100-2: gate called via `spawn_blocking` (slice 3). TS-100-3 fixed in slice 2.
- Breaker uses `Segment::Equity` for the trading-day calendar (limit, RISK-105-7). The latch restart-loss is closed (RISK-105-1).
- TS-105-6 deferred to slice 3: AC1 property test and the pipeline-layer guard tests (both-layers rule), including pipeline `peek` then guarded place counting one slot at pipeline level.
- defer: slice 8 audit anchor (hard readiness condition: no paper day counts until it merges).
- Real-SQLite proof lives in slices 6 (AC3), 7 (AC4 triggers, chain verify on reopen) and 8 (AC4 on real chain, AC8 composition); T2 tester checks these on a temp DB.
- Kill-switch in-process latch (set on every `engage`, cleared by a successful `release`) is lost on restart; only the persisted state survives.
- RISK-100-2: slices 2/3 derive `reduce_only` from the `ApprovedOrder`, not request data, with a test that a non-exit intent is refused when engaged. SEC-100-4: cap `actor`/`reason` length at the route (later slice). SEC-100-5: slice 3 runs the gate in `spawn_blocking` and tests it.
- Owner to confirm defaults (RISK-96-11): 300 s approval TTL, 50 bps drift, `breaker_engages_kill_switch = false`. All stay UNVERIFIED.
- RISK-105-9: owner to confirm whether a breaker trip should always engage the persisted kill switch (`breaker_engages_kill_switch`, default stays `false`, UNVERIFIED; reviewer recommends `true`). Not blocking: the latch is restored from the audit log regardless.
- `GuardedExecutor` (slice 2) and the pipeline (slice 3) must both call `KillSwitch::admits`.
- Defer RISK-96-8 to slice 3; RISK-96-5 to slice 2; SEC-96-2 anchor of latest (seq, hash) to slice 8; CR-96-5 to slice 5.
- Bound `paper_mis_square_off_ist` against the session calendar (CR-96-5); slice sizes for later F10 slices <= 400 lines (CR-96-6).
- Rows-only chain cannot detect tail truncation (see ADR).
- Slice 5: MIS/NRML margin from a `MarginSource` and intraday shorting (no source exists, so refused); AMO queue; session-end (not date-change) DAY expiry; the wiring supplies the `SimConfig::from_section` seed from the app; real-store ledger/audit adapters are slice 7.
- RISK-119-6: server wiring raises the alarm on `blocked_square_offs` and the UI shows a banner (BZ-511, BZ-516 status).
- RISK-119-6: live path design question, exits must not depend on audit availability (back-filled row); decided with the owner in BZ-602.
- Slice 5 (CR-119-2): a limit order fills at its limit only when the tick trades strictly through it, with no price-versus-participation check beyond the volume cap; documented simplification.
- TS-119-2 / BZ-205: position and cash never negative across random order streams needs the real ledger; belongs in the composition test (BZ-205).
- TS-119-4: `Executor::place(&ApprovedOrder)` is not covered here (tests cannot mint `ApprovedOrder`); carry a `place` test (error mapping) into slice 8 or a `test-util` constructor in `execution`.
