---
name: bz-web-api
description: Bazaar local web server in Rust (crate server, axum 0.8) - router and layers, thin handlers, DTOs, utoipa OpenAPI to UI types, problem+json, WebSocket hub with tick coalescing, backtest jobs, startup on 127.0.0.1, static UI serving, in-process tests. Use when touching crates/server, any route, DTO, WebSocket message or OpenAPI.
---

# Bazaar web API (crate `server`, package `bazaar-server`)

Scope: `crates/server` only, the composition root next to `bazaar-cli`. It wires ports to adapters, exposes JSON under `/api/v1` and one WebSocket, and serves `web/dist`. Local-only, single-user: it binds `127.0.0.1` and defends against the owner's browser, not a network. Neighbours: `bz-security` (the `security.rs` template: launch token, Host/Origin, session, CSRF, CSP, Kite callback nonce; linked, never copied here), `bz-architecture` (layering, event flow, resilience), `bz-execution` (approval queue, kill switch), `bz-quant` (the only `Recommendation` field list), `bz-core` (Money, Clock, errors), `bz-web-ui` (the client), `bz-testing` (fakes, MSW, security tests), `bz-repo-structure` (justfile, CI). Open `references/api-contract.md` for the route table, DTO shapes, problem codes and WebSocket messages; it is the single source of truth, and CONTRACTS.md section 4 only summarises it.

## 1. Default stack (checked 2026-11-18; run `cargo outdated`/`cargo tree -d` before bumping, pins live in `Cargo.toml`)

| Concern | Choice | Note |
|---|---|---|
| HTTP | `axum` 0.8.9, features `ws`, `macros` | 0.8: `{id}` path syntax, no `#[async_trait]` on extractors |
| Cookies | `axum-extra` 0.12, feature `cookie` | session cookie only; no signed/private jar needed (random id, server-side map) |
| Middleware | `tower` 0.5.3, `tower-http` 0.7.1 | features `trace`, `timeout`, `fs`, `set-header`, `compression-gzip`; never `cors` |
| OpenAPI | `utoipa` 6.0 (`chrono`, `uuid`, `decimal`), `utoipa-axum` 0.3 | `OpenApiRouter` + `routes!`; no Swagger UI (assets, CSP, no need) |
| Runtime | `tokio` 1.54 (`rt-multi-thread`, `macros`, `signal`, `sync`, `time`, `net`), `tokio-util` 0.7 | `CancellationToken` for shutdown and jobs; LTS lines exist (1.51, 1.53) if you prefer a `~` pin |
| WebSocket | axum `ws` + `futures-util` (`split`) | text frames only; 0.8 text is `Utf8Bytes` |
| CLI | `clap` 4 (derive) | `--port`, `--config-dir`, `--data-dir`, `--web-dist`, subcommand `openapi` |
| Secrets/RNG | `SecretStore` (Keychain) from `bz-core`; OS RNG via `getrandom`/`rand`; `subtle` | token and session ids per `bz-security` |
| Dev only | `http-body-util`, `insta`, `tokio-tungstenite` (client), `bazaar-core` feature `test-util` | no live broker, no network beyond `127.0.0.1:0` |

## 2. Golden rules

1. Handlers are thin: extract, convert the request DTO to a domain value (`TryFrom`), call one method on a port held in `AppState`, convert the result to a response DTO. No SQL, no HTTP client, no pricing/risk/gate logic, no `Utc::now()`.
2. DTOs live in `dto/`, derive `Serialize`/`Deserialize`/`ToSchema`, and are never domain types. Request DTOs use `deny_unknown_fields`. Response `From` impls destructure the domain struct **without `..`**, so a new domain field breaks compilation until the DTO is decided.
3. Money, prices, P&L and percent-of-capital are decimal strings (`DecimalStr` newtype with a string schema); quantities and counts are integers; dimensionless analytics (indicators, IV, Greeks) are JSON numbers, `null` while warming up. Instants are RFC 3339 UTC (candle `time` is epoch seconds), dates are `YYYY-MM-DD`. IST formatting happens only in the UI.
4. There is no "place order" route and none may be added. Orders come only from `POST /order-intents` then `POST /approvals/{id}/approve`; handlers never import `Executor`, `ApprovedOrder` or `GatedIntent`.
5. Approve re-runs the DecisionGate with a fresh quote and is single use: a repeat returns 409 (body carries the existing outcome) and never a second order. Pending approvals expire by `approval_ttl_secs`.
6. Execution mode, risk limits, thresholds and hard ceilings are never writable over HTTP. `/settings` is GET only. The only writes besides orders are the kill switch, journal notes, backtest start/cancel and the Kite login flow.
7. Every route except `/healthz`, `/auth` and `/api/v1/kite/callback` needs the session; non-GET also needs Origin and CSRF. The router is built in one place (`app::build`) so tests exercise the real layer stack.
8. Routes under `routes/` are registered only with `routes!` so the OpenAPI document cannot drift from the router; `just openapi` and the CI drift check cover the rest.
9. Errors are `application/problem+json` with stable `code`s; internals, paths, SQL, tokens and upstream bodies never reach a response.
10. Nothing blocks the reactor: CPU work uses `spawn_blocking`/rayon with a cancellation token; every channel is bounded; ticks are conflated, order and gate events are never silently dropped (a lagged consumer gets `resync`).
11. Safety never depends on a browser tab being open; the server enforces kill switch, breaker and expiry itself.
12. Secrets reach the process only through `SecretStore` (or the `.env` fallback loaded by `dotenvy` in `main.rs`); agents never read `.env` or run the live server.

## 3. Layout

```text
crates/server/
  Cargo.toml                 literal `version` (release-please)
  src/main.rs                clap, startup sequence, shutdown (nothing else)
  src/lib.rs                 pub mod ...; tests and main both use it
  src/app.rs                 pub fn build(state: AppState) -> Router  (the only router builder)
  src/state.rs               AppState (Arc<dyn Port> fields), build_state(config, adapters)
  src/openapi.rs             ApiDoc, router(), document()
  src/security.rs            template from bz-security: `Security` (launch token, sessions, allowlists; in AppState),
                             host_guard, origin_guard, session_guard, csrf_guard, security_headers, PUBLIC_PATHS, auth
  src/error.rs               ApiError, Problem, IntoResponse, From<DomainError>, ApiJson/ApiQuery extractors
  src/ws.rs                  Hub, coalescer, session loop
  src/jobs.rs                background jobs (backtests), progress, cancel
  src/routes/{mod,auth,status,market,candles,options,recommendations,orders,account,journal,backtests,settings,kite}.rs
  src/dto/{mod,auth,common,market,recommendation,orders,account,journal,backtest,status,ws}.rs
  tests/{common/mod.rs, api_*.rs, ws.rs, openapi.rs}
```

`routes/<area>.rs` exports `pub fn router() -> OpenApiRouter<AppState>`; `routes/mod.rs` merges them. One area per file keeps parallel developers from colliding.

## 4. Router and layers

In short: // app.rs
Full detail, examples and patterns: `references/router-and-layers.md` - read it before changing this area.

## 5. Handlers, state, errors

In short: request_body = CreateOrderIntentRequest,
Full detail, examples and patterns: `references/handlers-state-errors.md` - read it before changing this area.

## 6. OpenAPI to UI types

- `openapi.rs` builds the document from the same `OpenApiRouter` that serves traffic (`nest("/api/v1", routes::api())`); `info.version` is the fixed string `"1"` so releases do not churn the types. Because `nest` prefixes the document, `paths` keys include `/api/v1` (`/api/v1/recommendations`): the UI client uses `baseUrl: location.origin` and full paths, and a server test asserts every document path starts with `/api/v1/`. The WebSocket envelope types (`WsOut`, `WsIn` and their data types in `dto/ws.rs`) are registered with `components(schemas(...))` on `ApiDoc` so the UI gets them generated too. `bazaar-server openapi` parses the CLI first and prints `document().to_pretty_json()` without config, Keychain, data dir or socket (`just openapi` pipes it to `web/openapi.json`, then `pnpm -C web gen:api` writes `schema.d.ts`; CI `openapi-drift` regenerates and fails on a diff).
- Every handler has `#[utoipa::path]` with all status codes it can return, `Problem` bodies as `application/problem+json`, and a `tag`. Register enums and shared DTOs via `ToSchema`; rename with `#[schema(...)]`, not by hand-editing output.
- `decimal` makes `rust_decimal::Decimal` a string in the schema; never enable `decimal_float`. Prefer the `DecimalStr` newtype so the wire form is explicit; a test asserts the schema type is `string`.
- The `RecommendationDto` fields come from `bz-quant`; do not copy them into this skill, the reference or a test name list.

## 7. WebSocket hub (`ws.rs`; messages in the reference)

In short: Upstream: the feed port delivers raw ticks into a bounded `mpsc`. One **coalescer** task keeps the latest tick per symbol and every ~250 ms (`tokio::time::interval`, `MissedTickBehavior::Skip`, period injectable for tests) publishes the ...
Full detail, examples and patterns: `references/websocket-hub.md` - read it before changing this area.

## 8. Backtest jobs (`jobs.rs`)

`POST /backtests` validates (enabled strategy, params through the strategy's own `validate()`, date range inside stored data) and returns **202** with `Location` and a `BacktestJobDto`; one job runs at a time (second start is 409 `backtest_busy`). The job runs in `spawn_blocking` with a child `CancellationToken` and a throttled progress sink (at most ~4 updates/s) that publishes `job_progress` on `Hub.events` and updates the table that `GET /backtests/{id}` reads, so a missed WebSocket message costs nothing. `POST /backtests/{id}/cancel` cancels cooperatively (the runner checks `is_cancelled()` between bars or symbols) and is idempotent. Panics in the join handle become `failed` with a generic message. Results are persisted by the backtest repository, not the server; jobs found `running` at startup are marked `failed(interrupted)`. The runner/sink traits belong to `bz-backtest-risk`; agree the signature with that developer, do not invent one in `server`.

## 9. Startup, serving, sleep

In short: `main.rs`: `dotenvy::dotenv().ok()` (before clap so `.env` reaches flag defaults; empty values count as unset; harmless for `openapi`) -> parse the CLI (`openapi` returns here) -> tracing -> load and validate config (fail fast) -> ...
Full detail, examples and patterns: `references/startup-serving-sleep.md` - read it before changing this area.

## 10. Tests (unit tests in every PR; fakes and temp dirs only)

In short: `tests/common/mod.rs`: `TestApp::new()` builds `AppState` from fakes (`FakeClock`, in-memory ports, `FakeFeed`, `FakeExecutor` that counts calls), temp dirs and a temp `web/dist`, calls `app::build`, performs the real `/auth?token=` ...
Full detail, examples and patterns: `references/tests.md` - read it before changing this area.

## 11. Red flags unique to this skill (the full scan lives in `bz-security`)

```bash
rg -n '\.route\(' crates/server/src/routes                       # routes must use routes! (OpenAPI parity)
rg -n 'Executor|ApprovedOrder|GatedIntent|\.place\(' crates/server/src --glob '!{main,state}.rs'   # no order path in handlers
rg -n 'serde_json::Value|json!\(|\bDecimal\b|f32|f64' crates/server/src/dto   # untyped bodies, raw Decimal or floats on money (analytics f64 needs a comment)
```

## 12. Checklist

- [ ] Route registered with `routes!`, `#[utoipa::path]` lists every status, `just openapi` run and `schema.d.ts` committed.
- [ ] `references/api-contract.md` table updated, the MSW handler (`bz-testing`) added and a test written in the same PR.
- [ ] Handler is thin; DTO separate; money as `DecimalStr`; errors map to documented problem codes.
- [ ] No place-order path, no writable mode or limits, approve still re-runs the gate and is single use.
- [ ] Session, Origin and CSRF behaviour unchanged; layer order in `app::build` unchanged or justified.
- [ ] WebSocket changes keep ticks coalesced, events bounded, `Lagged` handled, sessions closing on shutdown.
- [ ] Tests use fakes and temp dirs; `cargo fmt`, `clippy -D warnings`, `nextest -p bazaar-server` pass; no placeholder text left.
