---
name: bz-architecture
description: Bazaar architecture for a local-only, single-user web app - Cargo workspace crate map and dependency rule, ports and adapters, order pipeline and execution modes, tick-to-WebSocket event flow, laptop process model (sleep/wake, catch-up), in-server scheduler, resilience, NFRs, ADRs in DECISIONS.md. Use when designing a feature, adding a crate, vendor, store, job or route, or writing a design or ADR.
---

# Bazaar Architecture

Act as the architect: simple, testable designs with explicit trade-offs and capital protection built into the structure, not bolted on. Implementation idioms: `bz-core`. Tests: `bz-testing`. Secrets, launch token and browser security: `bz-security`. HTTP routes, WebSocket, OpenAPI, sessions: `bz-web-api`. React screens and API client: `bz-web-ui`. Physical layout and manifests: `bz-repo-structure`. Slice detail: `bz-market-data`, `bz-quant`, `bz-backtest-risk`, `bz-execution`. Domain facts: `bz-india-markets`. Strategy and rules content: `bz-market-strategy`, `bz-trading-discipline`. Names and shapes shared across skills: `CONTRACTS.md` (authoritative; if this skill disagrees, the contract wins).

## 1. Style and process model

A **modular monolith**: one Cargo workspace, one server process (`bazaar-server`, axum, serving the built React UI from `web/dist` on the same origin), plus a headless `bazaar-cli` over the same crates. Hexagonal layering: pure domain and maths at the centre, I/O behind traits, two composition roots (`server`, `cli`). No microservices, no broker, no cloud services, no containers. A second process or network service needs an ADR.

```
 Browser tab (React/TS, web/) <-HTTP /api/v1 + WebSocket /api/v1/ws-> crates/server (axum) --+
                                     127.0.0.1:8787 only                                      +--> wires adapters into ports
 bazaar-cli (headless, human-run) ----------------------------------------------------------+
        scanner -> strategies -> indicators/options/regime -> core
        execution -> risk -> core        paper | broker implement Executor
        data (Kite, bhavcopy) | store (SQLite, Parquet/DuckDB) implement core ports
```

Bazaar is **local-only and single-user**: it runs on the owner's Mac, `just run` starts `bazaar-server` on `127.0.0.1:8787`, and the UI is a browser tab. The same binary and config serve analysis, paper trading and (much later) live orders; only the wired `Executor` changes. Server and UI are built and versioned together. Route, session and CSRF design: `bz-web-api` and `bz-security`; do not restate them in designs.

Process model rules (a laptop is not a server):

1. The server is up only while the Mac is awake and `just run` is running. Nothing may assume 24x7 uptime; a design that needs it is wrong.
2. **The UI is a viewer.** Closing the tab does not stop the feed, scans, paper trading, stops or jobs. Nothing safety-relevant (kill switch, daily-loss lockout, approval expiry, MIS square-off suggestions, stale-data blocking) depends on a browser being open. Alerts use the browser Notification API while a tab is open (Telegram optional); the server keeps its own persisted state regardless.
3. Several tabs are fine: every tab is a read-mostly view of server state, and approvals are single-use server-side (first approve wins, the second gets a conflict).
4. Server-side state that matters is persisted: kill switch, daily-loss lockout (scoped to the IST trading day), approvals, intents, audit chain, `job_runs`. Only sessions are in memory; a restart logs the browser out (a new launch token is printed).
5. **Binding beyond `127.0.0.1` (LAN, VPS, tunnel, reverse proxy) is out of scope.** It needs an ADR and a security redesign (TLS, real login, rate limits) before any code; the server refuses non-loopback addresses.

## 2. Crate map and dependency rule

| Crate | Owns | May depend on |
|---|---|---|
| `core` | domain types, `Money`/Decimal, IST `Clock` + `MarketCalendar`, config types, shared ports in `core::ports` (candles, persistence repos, `MarginSource`, `Notifier`, `SecretStore`) | std, serde, rust_decimal, chrono(-tz), thiserror |
| `indicators`, `options` | pure maths, no I/O | `core` |
| `regime` | trend/range + volatility state | `core`, `indicators` |
| `strategies` | `trait Strategy`, one module each | `core`, `indicators`, `options`, `regime` |
| `data` | `MarketData` trait, Kite read-only client, bhavcopy, instrument master, corporate actions, WS client, bar builder | `core` |
| `store` | SQLite (sqlx) + Parquet/DuckDB repositories; implements the `core::ports` persistence repos (intent, approval, ledger, audit, kill switch, job runs) | `core` |
| `risk` | sizing, limits, `DecisionGate`, behaviour guards | `core` |
| `backtest` | event loop, `CostModel`, slippage, walk-forward | `core`, `strategies`, `risk`, `indicators` |
| `scanner` | run strategies over universe, score, rank, explain | `core`, `strategies`, `regime`, `risk`, `backtest` (stats types) |
| `execution` | pipeline, `ApprovalQueue`, `trait Executor` + order state machine (owned by `bz-execution`), kill switch, rate limiter, audit-chain logic, `ExecutionMode`; persists only through `core::ports` repos | `core`, `risk` (never `store`) |
| `paper` | `PaperExecutor`, `PaperAccount` | `core`, `execution` |
| `broker` | `LiveExecutor` (Kite orders), cargo feature `live-trading`, off by default | `core`, `execution`, `data::kite::session` only |
| `cli`, `server` (`bazaar-cli`, `bazaar-server`) | the only composition roots; `server` (`crates/server`) also owns routes, DTOs, security layer, WebSocket hub, scheduler. The UI lives in `web/`, not in a crate | everything |

Rules, enforced by crate boundaries and checks, not by review alone:

1. Dependencies point toward `core`. Domain crates (everything above `cli`/`server`) never depend on `server`. `indicators`/`options` never see `data`, `store`, tokio or I/O.
2. **No crate below `execution` can place an order.** `data`'s Kite client exposes read endpoints only (instruments, quotes, historical, margins, profile). Order endpoints exist only in `broker`.
3. Only `cli` and `server` pick concrete adapters (SQLite vs in-memory, Paper vs Live, Kite vs fixture feed).
4. `data` and `store` never depend on each other, and `execution` never depends on `store`; they meet through ports in `core::ports` (`CandleSource`, `CandleSink`, the persistence repos) and the composition root wires the `store` implementations in and runs ingestion.
5. `broker` is compiled into the server only behind `live-trading`; default features never enable it. Restrict who may depend on it with cargo-deny `bans` plus `wrappers` (check the current cargo-deny docs for exact syntax) and a CI step that compiles the feature but never runs live calls.
6. Prefer enforcement that fails the build: a test (section 14) that parses `cargo metadata` and asserts the allowed-edges table above.
7. New crate or new edge = ADR (section 11). Resist `utils`/`common` crates; shared things go to `core` only if two or more crates need them.

## 3. Ports and adapters

Define a port from the consumer's need, in domain words. Port traits shared by several crates live in `core::ports`; a port used by one family lives with its owner (`MarketData` in `data`, `Executor` in `execution`).

| Port | Defined in | Adapters (real / test) |
|---|---|---|
| `Clock` | core | `SystemClock` (IST) / `FakeClock` |
| `MarketData` (subscribe ticks, historical, quotes) | data | Kite / `ReplayFeed` over `fixtures/`, local WS test server |
| `InstrumentMaster` (lot, tick, expiry, segment) | data | Kite CSV dump / fixture file |
| `CandleSource`, `CandleSink` | core::ports | Parquet+DuckDB / in-memory `Vec` |
| Persistence repos: intent, approval, ledger/journal, audit, kill switch, job runs | core::ports (implemented in `store`) | SQLite (sqlx) / in-memory SQLite or fake |
| `Executor` | execution | `PaperExecutor` / `LiveExecutor` / `RecordingExecutor` fake |
| `MarginSource` (the one margin seam; not on `Executor`) | core::ports | Kite margins API / fixed fake; no estimate by hand |
| `Notifier` | core::ports | WebSocket `gate_alert` push, Telegram (optional) / `VecNotifier` |
| `SecretStore` | core::ports | macOS Keychain (`keyring-core`); fallback env vars from an untracked `.env` the bin loads / in-memory map |

- `Executor` (trait, `ExecError`, order state machine, `ApprovedOrder`) is defined once, in `bz-execution` section 6 and section 3; do not copy it into designs, link it. The UI's PAPER/LIVE label comes from its `kind()`.
- `ApprovedOrder` has a constructor reachable only from `execution` after `DecisionGate` + approval; `GatedIntent` is minted only in `risk`. Without that value nobody can call `place`; this is the type-level guard behind "no gate bypass".
- Dyn ports use `async-trait` (native async fn in traits is not dyn compatible); use generics only where profiling proves vtable cost.
- Inject `Clock`; never call `Utc::now()` or `Local::now()` in domain code. The scheduler, session expiry and approval TTLs all read the same `Clock`.
- Stores are chosen in ADRs, not assumed: the default (SQLite for state, Parquet partitioned by symbol/year via DuckDB for candles) is recorded in `DECISIONS.md` once the user confirms it. Store specifics live in `bz-market-data`.

## 4. Order pipeline (identical in every mode)

```
Signal -> Recommendation -> OrderIntent -> DecisionGate -> Approval -> Executor -> Fill -> Position + Journal + Audit
 (strategies)  (scanner)     (execution)     (risk)      (user click)  (Paper|Live)
```

| Stage | Contract |
|---|---|
| Signal | strategy output from past data only; no sizing, no money |
| Recommendation | fields owned by `bz-quant` (`RecommendationDto`); includes a "low evidence" flag under about 30 trades |
| OrderIntent | what the user may approve: concrete price, qty, product, validity, rupee risk, `intent_id` (the broker idempotency tag is derived from it, see `bz-execution` `references/live-phase.md`) |
| DecisionGate | rules-only checklist from `config/risk.toml`; returns a `GateVerdict` (`bz-backtest-risk` `references/decision-gate.md`); no ML, no discretion; override needs typed reason and is audited |
| Approval | `paper` and `semi_auto`: user click per order. `auto`: policy engine, locked (section 5). Gate runs again at approval time with fresh prices; approvals expire after `approval_ttl_secs` |
| Executor | one trait, two impls; the pipeline cannot tell them apart |
| Fill | updates position, journal entry, audit record in one SQLite transaction |

Design points:
- A **single-writer pipeline task** (tokio task owning an `mpsc` command queue) serialises intents, gate checks, kill-switch reads and sends. Daily-loss and position-count checks race otherwise. HTTP handlers send commands to it and await the reply; with two tabs open, two approvals for one intent resolve as first wins, the second gets a conflict.
- Kill switch and daily-loss lockout are persisted flags read by the pipeline before every `place`; they are server state, not a UI nicety.
- Every state change appends to the hash-chained audit log before the side effect it describes (write-ahead).
- Each stage is a plain function over values, so a property test can feed random intents through the pipeline with a `RecordingExecutor` and assert nothing was placed after a `Block`, after kill switch, or without approval.

## 5. Execution modes

`ExecutionMode` lives in `config/risk.toml` as `[mode] execution_mode`: `paper` (default) | `semi_auto` | `auto`; never set by env var, CLI flag or HTTP request.

| Mode | Executor | Approval | Entry condition |
|---|---|---|---|
| `paper` | `PaperExecutor`, virtual ₹1,00,000 behaving like real cash | user click | default |
| `semi_auto` | `LiveExecutor` | user click on every order | `live-trading` feature built, pre-live checklist (`bz-security` `references/sebi-algo-compliance.md`), paper readiness bar met (`bz-trading-discipline`), broker rules looked up |
| `auto` | `LiveExecutor` | auto policy | locked: compliance + explicit opt-in + readiness bar + broker permits it for personal use |

Mode changes are an ADR trigger and an audit event; the UI shows a permanent PAPER or LIVE label from the executor kind. A downgrade (auto to semi_auto to paper) is always allowed instantly; an upgrade needs the checks and a server restart. Live design detail: open `references/live-trading-phase.md` before touching `broker/` or the mode switch.

## 6. Event flow (live data to the browser)

```
Kite WS ticks -> reconnecting client -> bounded mpsc -> BarBuilder (tick -> 1m/5m/...)
   -> streaming indicators -> strategies.on_bar -> scanner (score/rank) -> server WebSocket hub (coalesced ticks) -> browser
                                  \-> PaperAccount mark-to-market -> position P&L -> WebSocket hub
```

(`regime` sits between indicators and strategies where a strategy needs it.) The flow ends at the server's WebSocket hub, which fans out to each connected tab (message types in `bz-web-api` `references/api-contract.md`). The flow runs whether or not any tab is connected; the hub just has zero subscribers.

- Bounded channels everywhere. Policy per hop: **ticks may be conflated** (keep latest per instrument, pushed at a fixed cadence, for example 4 to 10 Hz); **bar-close and order events are never dropped** (apply backpressure or log and halt the stage).
- Each WebSocket session has its own bounded outbound queue. A slow or dead tab loses conflated ticks, is closed when its queue of must-deliver messages overflows, and never blocks the pipeline or other sessions. A reconnecting tab refetches state over REST, then resumes the stream.
- Strategies run on **bar close** only; never per tick. Intraday risk checks (stop hit, trailing stop) may use ticks, in `paper`/`execution`, not in strategies.
- Fan-out with `tokio::sync::broadcast` for in-process observers (WebSocket hub, journal); each subscriber handles `Lagged` explicitly.
- CPU-heavy work (universe scan, backtest) runs on rayon or `spawn_blocking`, never on the async reactor; it is cancellable (check a token between symbols) and reports progress.
- EOD universe scans read Parquet through DuckDB or polars in batch; the live path is streaming. Indicators have one implementation with two drivers (`bz-quant`).
- Everything is IST at the edge: store timestamps with a clear zone convention (UTC instants, IST for display and session logic via `MarketCalendar`).

## 7. HTTP and WebSocket boundary

Owner: `bz-web-api` (routes, OpenAPI, error mapping, WebSocket messages) and `bz-security` (launch token, Host/Origin checks, cookie, CSP). Do not restate routes or the security template here; the architectural rules are:

| Concern | Rule |
|---|---|
| Handlers | thin: validate input, call one use case through state-held ports, map the error to `application/problem+json`. No logic, no SQL, no broker calls inline |
| Types | DTOs live in `crates/server` (a `dto` module) with `serde` + `utoipa::ToSchema`; OpenAPI is generated from code and `web/src/api/schema.d.ts` is generated from it, never hand-edited (CI fails on diff). Money crosses the boundary as strings or integer paise, never floats |
| State | `AppState` holding `Arc<dyn Port>`s built in one `build_core(config, adapters)` function that `cli` and tests also call |
| Push | one WebSocket per tab; throttled, conflated ticks and must-deliver order and gate messages (section 6) |
| Long jobs | request returns a job handle at once; progress and result per `bz-web-api`; cancellable |
| Safety | the session requirement, Host/Origin checks and the short list of public routes are in `bz-security`; order-related routes exist only as create intent, approve, reject and kill switch; there is **no** place-order route |

UI details (screens, charts, Why panels, disclaimer, API client): `bz-web-ui`.

## 8. Resilience

- **Timeouts** on every outbound call (connect and total). Kite REST client: one shared `reqwest::Client`, token-bucket rate limiter matching the broker's current documented limits (look them up; `config/data.toml`, with a `DECISIONS.md` note), 429 handling honouring retry hints.
- **Retries**: idempotent reads only, exponential backoff with jitter, bounded budget. Never auto-retry `place`; reconcile instead (query order book by tag, then decide).
- **Kite feed**: reconnect with backoff and jitter, resubscribe from the desired-subscription set, heartbeat/idle detection, a gap-fill via historical candles after reconnect so bars stay complete.
- **Sleep and wake** (the normal laptop failure): `tokio::time::Instant` timers may not advance during sleep on macOS and the TCP socket is usually dead afterwards, so do not trust timers to notice. The scheduler tick compares the injected `Clock` with the previous tick; a gap beyond a configured threshold (a few minutes) is a **wake event**. On wake: mark all data stale, drop and re-establish the Kite WS, gap-fill candles, run job catch-up (section 9), expire approvals older than their TTL by `Clock`, re-read the kill switch and daily-loss state, and push `data_status` so open tabs show "data delayed" until fresh ticks arrive. Network may return a few seconds after wake: retry with backoff, never crash.
- **Stale data**: each instrument carries `last_tick_at`; if older than the configured threshold during market hours (and always right after wake), the UI shows a "data delayed" badge and the scanner marks signals stale. Stale prices block order approval (gate rule).
- **Mac asleep means unmonitored**: app-managed stops, trailing logic and MIS square-off suggestions cannot run while the Mac sleeps. Design position handling so this is explicit: paper positions are marked at the next fresh tick after wake; for live, the broker-side versus app-managed stop decision is an open item (`references/live-trading-phase.md` section 5). The owner may keep the Mac awake during market hours (for example `caffeinate`), but no safety property may rely on it.
- **Partial failure**: a failed symbol, strategy or job is recorded and skipped, the scan completes with a visible warning count; one broken adapter never crashes the server. Kite session expiry (daily) degrades to EOD/analysis mode with a clear re-login prompt.
- **Kite session**: the access token expires daily; the server detects token errors (403), flips to "logged out", stops live feeds and blocks live modes, never loops on retries. The token is stored through `SecretStore` (macOS Keychain; never SQLite, never the browser), so a restart on the same day can reuse it; the daily login is the redirect flow in `bz-web-api`/`bz-security`.
- **Server restart** (crash, quit, reboot, `just run` again): all state is reconstructable from SQLite. Startup order: open stores, load config (fail closed), load the **kill switch** (an unreadable value counts as engaged), run `reconcile` (orders, positions, audit-chain check), expire stale approvals, run missed-job catch-up (section 9), only then bind `127.0.0.1` and accept intents. `/healthz` reports liveness only; readiness detail is behind the session in `GET /api/v1/status`.
- **Pending approvals expire**: an approval older than `approval_ttl_secs` by `Clock` is marked expired (audit event) on a timer, at startup and on wake; approving an expired or restart-orphaned approval fails, and the user creates a new intent. The gate re-runs on every approve regardless.
- **Kill switch and daily-loss lockout persist**: engaged state is written through the repo before the response returns, survives restarts and sleep, and only a user's explicit disengage (`PUT /api/v1/kill-switch`) clears the kill switch; the lockout clears only at the next IST trading day. No job, startup path, wake handler or crash recovery may clear either.
- **Shutdown** (SIGINT/SIGTERM, Ctrl-C in `just run`): stop accepting requests, cancel jobs, flush audit log and bars, persist kill switch state, leave open paper positions intact in SQLite, close WebSockets with a close frame.
- **Circuit breaker** around vendor adapters; open state shows as a connection badge.

## 9. Scheduled jobs and config refresh

Nothing market-specific is compiled in (`bz-india-markets`). Recurring work (instrument refresh, EOD bhavcopy import, F&O ban-list and holiday-calendar refresh, scans, approval expiry sweep) runs **inside `bazaar-server`** under a small scheduler driven by the injected `Clock` and `MarketCalendar`, never by ad hoc sleep loops or the system clock. Because the Mac may be asleep or off at the scheduled time, every job is **catch-up capable**: due-ness is computed from "what is missing for which date", not from "did the timer fire". Catch-up runs at server start and on wake. The same use cases are `bazaar-cli` subcommands for manual human runs. Full job table, `job_runs` schema, config-staleness table: `references/scheduled-jobs-and-design-deliverables.md`.

- **Idempotency**: every run first records `(job, key)` in a unique `job_runs` row (key is the trading date, or the bar close for scans); a restart or double tick cannot run the same unit twice, and job bodies upsert by natural key so a retry after a partial run is safe.
- **Missed-run policy** (Mac asleep, server not running, or the clock jumped): instrument refresh catches up once if today's is missing and the session has not ended, else F&O stays blocked; ban list for today missing on a trading day means every F&O symbol is treated as blocked until fetched; EOD import backfills missing trading dates oldest first within a configured window, then asks the owner; the EOD scan runs once for the latest date only; intraday scans are never caught up, they are recorded `missed` and the data shows stale. A `running` row left by a crash is marked `abandoned` at startup.
- Data stays marked stale (`data_status`) until the catch-up that covers it succeeds; the UI shows what is stale and why.
- Jobs never place orders or touch approvals, the kill switch or `risk.toml`; one failing job is recorded and surfaced on `data_status` without stopping the others.
- Tests drive the scheduler with `FakeClock` (cross the trigger, jump the clock forward across a whole session to simulate wake, restart mid-run, holiday, missed window).

Every config file carries `schema_version`, `verified_on` and `source`; loaders validate schema and sanity ranges and fail closed. `just db-check` (runs `bazaar-cli check-connections`) is the only way agents check `sqlite`, `duckdb`, `kite-session`, `instruments` and `audit-chain`. Secrets reach the program only through `SecretStore` (Keychain, or env vars from an untracked `.env` the bin loads); agents never open `.env`, and no design may require them to.

## 10. NFR template (fill for every feature)

| NFR | Starting target | How verified |
|---|---|---|
| Tick to screen latency | under 250 ms p95 from tick receipt to WebSocket send on the watchlist at the push cadence | replay fixture benchmark |
| API latency | read endpoints under 100 ms p95 and writes (intent, approve, kill switch) under 300 ms p95, excluding broker round trips; heavy reads paginated | in-process `oneshot` timing tests |
| Concurrent tabs | one owner, a few tabs with no pipeline slowdown; slow tabs isolated; double-approve safe | multi-client WebSocket test; approval race test |
| Bar close to recommendation refresh | under 2 s for the F&O universe | criterion bench on fixture universe |
| EOD scan, full NSE equity universe | under 60 s on the owner's Mac | bench, scan progress |
| Backtest throughput | 1 strategy, 1 instrument, 10 years of daily bars in under 1 s | criterion bench; golden regression |
| Cold start to ready | under 5 s without network, including reconcile and catch-up checks | manual + smoke test |
| Memory | under 1 GB resident with a full-universe EOD scan (it shares the Mac with a browser) | profile run |
| Data freshness | live tick staleness alert at the configured threshold; instrument master dated today; everything marked stale after wake until fresh | fake clock tests |
| Correctness | no look-ahead; Decimal for money; reference-value tests green | `bz-testing` suites |
| Safety | no order without Gate + approval; kill switch and lockout effective in one tick, across restarts and wake, with no UI open | pipeline property tests; restart and wake tests |
| Availability | runs while the Mac is awake; restart or wake keeps state and leaves no missed job silent; broker outage degrades analysis and paper, never crashes | scheduler, recovery and failing-adapter tests |
| Disk | Parquet size per symbol-year and SQLite growth recorded; retention and backup policy stated (backups of `data/` are the owner's, for example Time Machine) | `du` in notes |

These numbers are starting points for the first session; measure, then record real targets in `DECISIONS.md`.

## 11. ADRs in DECISIONS.md

The project keeps one append-only `DECISIONS.md` (the prompt's design log) with ADR-style entries. Write an entry when a decision is costly to reverse or touches money and risk. Triggers:

- A new data vendor or a change of plan or tier (cost, limits, legality).
- Any change to `ExecutionMode`, approval policy or live feature flag defaults.
- **Exposing the server beyond 127.0.0.1** (LAN, VPS, tunnel, reverse proxy, any non-loopback bind) needs an ADR **and a security redesign** (`bz-security`) first.
- **Auth and browser-security changes**: launch token or session lifetime, cookie flags, CSRF, Host/Origin checks, CSP, rate-limit or lockout rules (also `bz-security` review).
- **A new public route**: any route reachable without a session beyond `/healthz`, `/auth` and the Kite callback, or any new write route.
- A new crate, a new dependency edge, or a relaxed rule from section 2.
- Storage format or schema direction changes (SQLite layout, Parquet partitioning, DuckDB vs polars), or a new scheduled job or scheduler dependency.
- A new major dependency, or a major-version jump of axum, React, Vite or Rust edition.
- Any statutory or broker-rule interpretation (charges, SEBI algo framework, rate limits, static IP) with its source link and look-up date.
- Loosening a `risk.toml` default or a DecisionGate rule (also needs `risk-guardian` sign-off).

```markdown
## ADR-007: Kite Connect paid plan for live data
- Status: Proposed | Accepted | Superseded by ADR-0NN     - Date: 2026-MM-DD (IST)
- Context: forces, constraints, NFRs, sources with look-up dates
- Options: A (pros/cons), B (pros/cons)
- Decision: what and why
- Consequences: positives, negatives, follow-ups, how we would know it was wrong
```

Seed ADRs: modular monolith with hexagonal layering; axum server + React/TS UI served same origin; local-only single-user deployment (loopback bind, launch token, Mac Keychain, no remote access); data vendor (Kite Connect for live and historical, official bhavcopy for EOD, CSV fallback; confirm plan price and limits from the current Kite support pages); storage (SQLite via sqlx plus Parquet/DuckDB); Decimal for money; IST handling; pipeline and execution modes; OpenAPI-generated UI types; in-server scheduler over `Clock` with catch-up on start and wake; audit log design.

## 12. Design deliverables

Short document, linked to `DECISIONS.md`: context, goals, non-goals and phase; container/flow diagram (browser tab, server, crates, ports, external systems); crates, ports, DTOs, routes and WebSocket messages touched; main-path and failure sequences (including server restart and sleep/wake); data model and migration; source of each market fact; pipeline impact (Gate, approval, executor means `risk-guardian` review); exposure impact (bind, auth, public route means ADR and `bz-security`); NFR table (section 10) and test plan; risks and ADRs; independently mergeable slices per developer role. The numbered template is in `references/scheduled-jobs-and-design-deliverables.md` section 3.

## 13. Anti-patterns to reject

- Any path to an exchange or broker order outside the `Executor` trait; a `skip_gate`, `force`, or `auto_approve` flag; a place-order route.
- `f64` for rupee amounts or prices in domain types; wall-clock calls instead of `Clock`; UTC or local time in session logic.
- Strategies reading future bars, calling I/O, or holding state across symbols; market facts (lot sizes, expiry weekdays, holidays, charge rates, rate limits) in source code.
- Handlers with business logic; React components calling `fetch` ad hoc instead of the generated typed client; hand-edited generated types.
- Unbounded channels, per-tick WebSocket messages, blocking work on the tokio reactor, a slow tab able to stall the pipeline.
- Safety logic that lives in the browser or runs only while a tab is open (kill switch, lockout, expiry, stale-data blocking).
- Scheduled jobs outside the scheduler (ad hoc `tokio::spawn` loops with `sleep`), jobs that are not idempotent or cannot catch up, or startup/wake paths that clear the kill switch.
- Binding beyond `127.0.0.1`, a bind-address flag, a CORS layer, or unauthenticated routes beyond the contract list, without an ADR.
- A shared `utils` crate; a trait with a single impl and no test fake or planned adapter ("we might switch vendor" is not a reason beyond `MarketData`/`Executor`/`Store` seams listed above).
- Kite types outside `data`/`broker`; scraping exchange or third-party pages.

## 14. Tests for architecture

- **Dependency test** in `crates/cli/tests` or a `xtask`: parse `cargo metadata`, assert each workspace crate's dependencies are within the section 2 table (in particular no domain crate depends on `bazaar-server`); assert `bazaar-broker` is absent from default-feature builds of the server.
- **Pipeline property tests** with `RecordingExecutor`, `FakeClock`, fixed `risk.toml` fixtures: no placement after Block, kill switch, daily-loss breach, or expired approval; every placement has an audit record first.
- **Restart, wake and scheduler tests**: rebuild the core over the same SQLite file mid-flow; kill switch still engaged, orphaned approvals expired, `job_runs` prevents double runs, catch-up follows the missed-run table under `FakeClock`; advance `FakeClock` across hours (simulated sleep) and assert the wake handler marks data stale, expires approvals, triggers a feed reconnect and keeps the kill switch.
- **Adapter contract tests**: the same suite runs against the in-memory SQLite adapter and fakes; Kite adapters only against wiremock or a local WS server with recorded frames from `fixtures/`.
- **Server and API-types tests**: in-process `tower::ServiceExt::oneshot` on temp dirs (no real port, no real Keychain); the server refuses a non-loopback bind; session, Host and Origin behaviour per `bz-security`; generated OpenAPI and `web/src/api/schema.d.ts` match the committed files (`bz-web-api`, `bz-testing`).
- No live API, no real `.env`; connection status only through `just db-check`. Unit tests are required in every PR (`bz-testing`).

## 15. Security red flags (ripgrep)

```
rg -n 'fn place\(|Executor::|ApprovedOrder \{|GatedIntent \{' crates --glob '!**/tests/**'   # outside their owning crates
rg -n 'f64' crates/core crates/risk crates/execution crates/paper
rg -n 'Utc::now|Local::now|SystemTime::now' crates --glob '!**/clock.rs'
rg -n '0\.0\.0\.0|\[::\]|bind_addr|Cors|cors' crates/server                                 # loopback only, no CORS
```

Findings are design violations, not style issues. Full scan (secrets, bypass flags, `live-trading` default, auth and route checks): `bz-security`.

## 16. Checklist

- [ ] Change fits the crate map; any new edge or crate has an ADR in `DECISIONS.md`; no domain crate depends on `server`.
- [ ] New external system sits behind a port with a fake and a fixture-driven test; no vendor types leak.
- [ ] Order-related code goes through Gate, approval and `Executor`; audit written first; live code behind `live-trading`.
- [ ] Market facts come from instrument master or `config/*.toml` with `verified_on`.
- [ ] Channels bounded, WebSocket pushes throttled, heavy work off the reactor and cancellable.
- [ ] Failure paths designed: stale data badge, reconnect, sleep/wake, token expiry, partial scan failure, server restart; nothing safety-relevant needs an open tab.
- [ ] New scheduled job: idempotency key, catch-up and missed-run policy, `FakeClock` test including a wake jump.
- [ ] Handlers thin; DTOs typed; OpenAPI and UI types regenerated; any exposure change (non-loopback bind, auth, public route) has an ADR.
- [ ] NFR table filled; unit tests included; `just gate` green; `risk-guardian` / `quant-reviewer` consulted where their areas are touched.
