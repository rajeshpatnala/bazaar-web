---
name: bz-architecture
description: Bazaar architecture for a local-only, single-user web app - Cargo workspace crate map and dependency rule, ports and adapters, order pipeline and execution modes, tick-to-WebSocket event flow, laptop process model (sleep/wake, catch-up), in-server scheduler, resilience, NFRs, ADRs in DECISIONS.md. Use when designing a feature, adding a crate, vendor, store, job or route, or writing a design or ADR.
---

# Bazaar Architecture

Act as the architect: simple, testable designs with explicit trade-offs and capital protection built into the structure, not bolted on. Implementation idioms: `bz-core`. Tests: `bz-testing`. Secrets, launch token and browser security: `bz-security`. HTTP routes, WebSocket, OpenAPI, sessions: `bz-web-api`. React screens and API client: `bz-web-ui`. Physical layout and manifests: `bz-repo-structure`. Slice detail: `bz-market-data`, `bz-quant`, `bz-backtest-risk`, `bz-execution`. Domain facts: `bz-india-markets`. Strategy and rules content: `bz-market-strategy`, `bz-trading-discipline`. Names and shapes shared across skills: `CONTRACTS.md` (authoritative; if this skill disagrees, the contract wins).

## 1. Style and process model

In short: A **modular monolith**: one Cargo workspace, one server process (`bazaar-server`, axum, serving the built React UI from `web/dist` on the same origin), plus a headless `bazaar-cli` over the same crates. Hexagonal layering: pure domain and ...
Full detail, examples and patterns: `references/style-and-process-model.md` - read it before changing this area.

## 2. Crate map and dependency rule

In short: Rules, enforced by crate boundaries and checks, not by review alone:
Full detail, examples and patterns: `references/crate-map-and-dependency-rule.md` - read it before changing this area.

## 3. Ports and adapters

In short: Define a port from the consumer's need, in domain words. Port traits shared by several crates live in `core::ports`; a port used by one family lives with its owner (`MarketData` in `data`, `Executor` in `execution`).
Full detail, examples and patterns: `references/ports-and-adapters.md` - read it before changing this area.

## 4. Order pipeline (identical in every mode)

In short: Signal -> Recommendation -> OrderIntent -> DecisionGate -> Approval -> Executor -> Fill -> Position + Journal + Audit
Full detail, examples and patterns: `references/order-pipeline.md` - read it before changing this area.

## 5. Execution modes

`ExecutionMode` lives in `config/risk.toml` as `[mode] execution_mode`: `paper` (default) | `semi_auto` | `auto`; never set by env var, CLI flag or HTTP request.

| Mode | Executor | Approval | Entry condition |
|---|---|---|---|
| `paper` | `PaperExecutor`, virtual ₹1,00,000 behaving like real cash | user click | default |
| `semi_auto` | `LiveExecutor` | user click on every order | `live-trading` feature built, pre-live checklist (`bz-security` `references/sebi-algo-compliance.md`), paper readiness bar met (`bz-trading-discipline`), broker rules looked up |
| `auto` | `LiveExecutor` | auto policy | locked: compliance + explicit opt-in + readiness bar + broker permits it for personal use |

Mode changes are an ADR trigger and an audit event; the UI shows a permanent PAPER or LIVE label from the executor kind. A downgrade (auto to semi_auto to paper) is always allowed instantly; an upgrade needs the checks and a server restart. Live design detail: open `references/live-trading-phase.md` before touching `broker/` or the mode switch.

## 6. Event flow (live data to the browser)

In short: Kite WS ticks -> reconnecting client -> bounded mpsc -> BarBuilder (tick -> 1m/5m/...)
Full detail, examples and patterns: `references/event-flow.md` - read it before changing this area.

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

In short: **Timeouts** on every outbound call (connect and total). Kite REST client: one shared `reqwest::Client`, token-bucket rate limiter matching the broker's current documented limits (look them up; `config/data.toml`, with a `DECISIONS.md` ...
Full detail, examples and patterns: `references/resilience.md` - read it before changing this area.

## 9. Scheduled jobs and config refresh

In short: Nothing market-specific is compiled in (`bz-india-markets`). Recurring work (instrument refresh, EOD bhavcopy import, F&O ban-list and holiday-calendar refresh, scans, approval expiry sweep) runs **inside `bazaar-server`** under a small ...
Full detail, examples and patterns: `references/scheduled-jobs-and-config-refresh.md` - read it before changing this area.

## 10. NFR template (fill for every feature)

In short: These numbers are starting points for the first session; measure, then record real targets in `DECISIONS.md`.
Full detail, examples and patterns: `references/nfr-template.md` - read it before changing this area.

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

In short: **Dependency test** in `crates/cli/tests` or a `xtask`: parse `cargo metadata`, assert each workspace crate's dependencies are within the section 2 table (in particular no domain crate depends on `bazaar-server`); assert `bazaar-broker` is ...
Full detail, examples and patterns: `references/tests-for-architecture.md` - read it before changing this area.

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
