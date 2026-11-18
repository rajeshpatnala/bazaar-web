---
name: bz-execution
description: Bazaar order pipeline in Rust - OrderIntent state machine, ExecutionMode (paper, semi_auto, auto), ApprovalQueue, Executor trait, PaperExecutor/PaperAccount (virtual rupee cash), kill switch, daily-loss circuit breaker, rate limiter, hash-chained audit log, future broker crate behind `live-trading`. Use when touching crates execution, paper or broker.
---

# Bazaar execution (crates `execution`, `paper`, `broker`)

Everything between an approved idea and a fill. Policy (what a trade must satisfy) lives in `bz-trading-discipline`; `DecisionGate`, sizing and `CostModel` live in `bz-backtest-risk` and are called here, never re-implemented. Types, Decimal money, IST clock and config rules are in `bz-core`; layering in `bz-architecture`; secrets and red-flag regexes in `bz-security`; test tooling in `bz-testing`. Fill realism and live-broker detail are in `references/` (open them when the task touches fills, margin, charges, or the `broker` crate).

## 1. Stack and files

| Concern | Choice | Note |
|---|---|---|
| Money / qty | `rust_decimal::Decimal` via `core::Money`; quantities are integers (lots x lot size from instrument master) | never `f64` for rupees |
| Persistence | ports (intent, approval, ledger, audit, kill-switch repos) are traits in `bazaar_core::ports`; `store` implements them over sqlx + SQLite (`order_intents`, `intent_events`, `approvals`, `paper_ledger`, `audit_log`, `kill_switch`, `ops_counters`); roots wire them. `execution` depends on `core` + `risk` only | all writes for one transition share one transaction |
| Async trait | `async-trait` so `Arc<dyn Executor>` works (native `async fn` in traits is not dyn-compatible) | check current version |
| Hashing | `sha2` + `hex` for the audit chain | check current versions |
| Time | injected `Clock` + `MarketCalendar` from `core`; never `Utc::now()` in logic | tests use a fake clock |
| Config | `config/risk.toml` (`[mode]`, `[limits]`, `[hard_limits]`; keys only as in the `bz-trading-discipline` G-table), `config/charges.toml`, `config/compliance.toml` (live-phase checklist state, phase 9 only), each with `verified_on` + `source` | no thresholds in code; no other config files |
| Feature | `live-trading` (cargo, off by default) gates the whole `broker` crate | see section 10 |

## 2. Golden rules

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

## 3. OrderIntent and the state machine

In short: `OrderIntent` is the immutable *proposal*: id (UUIDv7), `recommendation_id`, instrument, side, qty, order type/price/trigger, product (`CNC|MIS|NRML`), stop and target, `risk_rupees`, `expires_at`, `mode_at_creation`, optional ...
Full detail, examples and patterns: `references/orderintent-and-the-state-machine.md` - read it before changing this area.

## 4. ExecutionMode

In short: pub enum ExecutionMode { Paper, SemiAuto, Auto }   // Auto is unconstructible without AutoUnlock
Full detail, examples and patterns: `references/executionmode.md` - read it before changing this area.

## 5. ApprovalQueue

In short: pub struct PendingApproval { intent: OrderIntent, gate_snapshot: GateVerdict, created: Ts, expires: Ts }
Full detail, examples and patterns: `references/approvalqueue.md` - read it before changing this area.

## 6. Executor trait

In short: This crate owns the trait and the state machine (section 3); `bz-architecture` and `references/live-phase.md` link here and do not restate them.
Full detail, examples and patterns: `references/executor-trait.md` - read it before changing this area.

## 7. Paper trading

`PaperAccount` = an append-only **ledger** (`paper_ledger`: seq, ts, kind, amount, intent_id, note); balances are derived, never stored as a mutable number.

| Entry kind | Effect |
|---|---|
| `Deposit` | initial virtual cash from config (default 1,00,000 INR); reset = new account epoch, old ledger kept |
| `MarginBlock` / `MarginRelease` | moves cash between `available` and `blocked` |
| `FillDebit` / `FillCredit` | trade value on buys and sells (CNC cash legs) |
| `Charge` | per fill from `CostModel` (`bz-backtest-risk`): brokerage, STT/CTT, exchange, SEBI, stamp, GST |

Invariants (property-test them): `available = deposits - blocked + net trade cash - charges >= 0`; every order passes a funds check before it is accepted; a rejected order leaves the ledger untouched; replaying the ledger reproduces the balance; paper P&L equals sum of ledger movements. Realised and unrealised P&L shown in the UI include charges.

`PaperExecutor` implements `Executor` with a `FillSimulator` (pure, deterministic given ticks and a seeded RNG for partials). Rules in `references/paper-fills.md`: next tick plus slippage, limit orders fill only when traded through, no fill at circuit limits, partial fills on thin strikes, MIS auto square-off at the configured time, margin from the broker margins API or the order is blocked.

## 8. Kill switch, circuit breaker, rate limiter

In short: pub struct SafetyGuard { kill: KillSwitch, breaker: DailyLossBreaker, limiter: RateLimiter }
Full detail, examples and patterns: `references/kill-switch-circuit-breaker-rate-limiter.md` - read it before changing this area.

## 9. Audit log

Table `audit_log(seq INTEGER PK, ts_utc, ts_ist, actor, kind, intent_id, payload_json, prev_hash, hash)`; `hash = sha256(prev_hash || canonical_json(seq, ts_utc, actor, kind, intent_id, payload))`, genesis `prev_hash = 64 zeros`.

- Repository exposes only `append(entry)` and `verify_chain() -> Result<(), ChainBreak{seq}>`. No update/delete method exists; SQLite triggers `BEFORE UPDATE` and `BEFORE DELETE` call `RAISE(ABORT, 'audit_log is append-only')`; the server's DB role/path never runs `DROP`.
- Appends happen inside the transition transaction; serialise appenders (single writer task) so `prev_hash` is race-free.
- What is logged: intent created, gate verdicts (full reasons), overrides (typed reason), approvals/declines/expiries, guard denials, sends, acks, fills, rejects, cancels, mode and kill-switch changes, unlock opt-in, reconciliation results, square-offs. `actor` is `User`, `System(component)` or `Policy(id)`.
- Payloads carry ids, prices, quantities and reasons, never tokens, API keys or `.env` values (`bz-security`). Run `verify_chain` at startup and expose it in `bazaar-cli check-connections` (`OK audit-chain` / `FAILED audit-chain <category>`); a break engages the kill switch. The hash chain is tamper-evident, not tamper-proof; periodic export of the head hash to the journal is a cheap extra.

## 10. The `broker` crate and `live-trading`

In short: `crates/broker/Cargo.toml` is a workspace member whose real code is `#[cfg(feature = "live-trading")]`; `default = []` in every crate. `crates/server` and `cli` forward the feature (`live-trading = ["bazaar-broker/live-trading"]`) and ...
Full detail, examples and patterns: `references/the-broker-crate-and-live-trading.md` - read it before changing this area.

## 11. Exit monitor (suggestions, not orders)

`ExitMonitor` watches open positions and emits `Suggestion` values (trail stop to X, book partial, time-based exit, MIS square-off warning N minutes before the configured time). A suggestion never becomes an order by itself: the user clicks, which creates a fresh `OrderIntent` through the normal pipeline. Trailing suggestions only **tighten** stops (no widening, `bz-trading-discipline`); each has plain-English Why text with the rupee effect. Delivery through the `Notifier` port.

## 12. Tests (see `bz-testing`; unit tests are required in every PR)

- State machine: table test of all legal transitions; `proptest` over random event sequences: never leaves a terminal state, never skips approval, `Filled.qty <= intent.qty`.
- Pipeline with fakes: `FakeClock`, `FakeGate` (pass/fail/error), `FakeExecutor` (records calls, can return `Network`, `Rejected`), in-memory SQLite. Assert: gate fail means executor call count 0; stale approval cannot be approved; double approve places one order; kill switch engaged means 0 calls through both the pipeline and the decorator; breaker latches and resets next trading day; limiter allows exactly N then denies, recovers after the window; `Unknown` never retries.
- `PaperAccount`: ledger property tests (no negative cash, replay equals balance, charges reduce cash, reject leaves ledger unchanged); insta snapshot of a day's ledger.
- `FillSimulator`: fixture tick sequences per rule in `references/paper-fills.md`; seeded RNG.
- Audit: append N, `verify_chain` OK, mutate a row via raw SQL in a test, `verify_chain` reports its seq; UPDATE/DELETE raise.
- Mode: `auto` without unlock is unconstructible (compile-fail or runtime test); bad config value falls back to `paper`; default feature set excludes `live-trading` (test `cfg!(feature = "live-trading")` is false in the default build).
- Broker (feature on): `wiremock` Kite fixtures from `fixtures/`: place ok, `MarginException`, 429, timeout then reconcile finds the tag, `TokenException`.

## 13. Red flags (ripgrep before approving a diff)

`skip_gate|bypass|force_send|auto_approve|approve_all|unchecked` near orders; `default = \[.*live-trading` ; `ExecutionMode::Auto` constructed outside unlock code; gate-bypass scans (`fn place\(`, `Executor::`, `ApprovedOrder \{` / `GatedIntent \{` outside their owning crates: `execution`/`paper`/`broker` for the first two, `execution` for `ApprovedOrder`, `risk` for `GatedIntent`); `f64` in paper ledger or order price; `Utc::now|SystemTime::now` in execution logic; `UPDATE audit_log|DELETE FROM audit_log`; `unwrap()` on guard or audit results; retry loops around `place`; kill-switch state cached without a DB re-read; broker limits as literals (`10`, `400`, `5000`); `tag` built from random per-attempt data; tokens or `access_token` in `tracing`/audit payloads; tests that reach `api.kite.trade`.

## 14. Done checklist

- [ ] Paper and live paths differ only in the Executor; no mode branching upstream
- [ ] State machine pure, exhaustive, with property tests; audit row per transition
- [ ] Approval re-checks gate with fresh quote; stale and duplicate approvals handled
- [ ] Kill switch persisted and enforced twice (pipeline + decorator); breaker latched; limiter caps from config
- [ ] Paper ledger invariants tested; charges and margin handled as in `references/paper-fills.md`
- [ ] Audit log append-only, chain verified at startup, no secrets in payloads
- [ ] `live-trading` off by default; `auto` unconstructible without unlock
- [ ] Unit tests added; no test touches a real broker; `just gate` green
