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

```rust
// app.rs
pub fn build(state: AppState) -> Router {
    let (api, _doc) = openapi::router().split_for_parts();          // /api/v1/** (session routes + /ws)
    let api = api
        .route_layer(from_fn_with_state(state.security.clone(), security::csrf_guard))     // inner
        .route_layer(from_fn_with_state(state.security.clone(), security::session_guard)); // outer: runs first

    let public = Router::new()
        .route("/healthz", get(status::healthz))
        .route("/auth", get(security::auth))                          // launch-token exchange
        .route("/api/v1/kite/callback", get(routes::kite::callback))  // nonce-checked, no cookie
        .route("/api/{*rest}", any(error::unknown_api));              // 404 problem+json, never index.html

    Router::new()
        .merge(public)
        .merge(api)
        .nest_service("/assets", static_assets(&state))               // real 404 for a missing asset
        .fallback_service(spa(&state))                                // ServeDir + ServeFile(index.html) fallback
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .layer(
            ServiceBuilder::new()                                     // first = outermost
                .layer(TraceLayer::new_for_http().make_span_with(security::path_only_span)) // never the query: /auth?token=
                .layer(security::security_headers())                  // CSP, nosniff, no-referrer, no-store on /api
                .layer(TimeoutLayer::with_status_code(StatusCode::REQUEST_TIMEOUT, Duration::from_secs(30)))
                .layer(from_fn(error::problem_backstop))              // empty 4xx/5xx (408, 405, 413) -> problem+json
                .layer(from_fn_with_state(state.security.clone(), security::host_guard))
                .layer(from_fn_with_state(state.security.clone(), security::origin_guard)), // non-GET + WS upgrade
        )
        .with_state(state)
}
```

Notes: `route_layer` (not `layer`) so 404/405 do not demand a session; later `route_layer` calls wrap earlier ones, hence CSRF first. Headers sit outside the Host/Origin checks so rejections also carry them. There is no `CorsLayer`, `Access-Control-*` header or `cors` feature. Hashed `/assets/*` may get `Cache-Control: public, max-age=31536000, immutable`; `index.html` gets `no-cache`. A missing `web/dist` logs a warning, the API still runs and `/` returns a plain hint ("run `pnpm -C web build`"). `TimeoutLayer::new` is deprecated in tower-http 0.7; use `with_status_code`. Compile-check `method_not_allowed_fallback` (problem+json for 405) against the pinned axum.

## 5. Handlers, state, errors

```rust
#[utoipa::path(post, path = "/order-intents", tag = "orders",
    request_body = CreateOrderIntentRequest,
    responses((status = 201, body = OrderIntentDto), (status = 404, body = Problem, content_type = "application/problem+json"),
              (status = 409, body = Problem, content_type = "application/problem+json")))]
async fn create(State(s): State<AppState>, ApiJson(req): ApiJson<CreateOrderIntentRequest>)
    -> Result<(StatusCode, Json<OrderIntentDto>), ApiError>
{
    let created = s.orders.create_intent(req.recommendation_id).await?;   // port: intent -> gate -> approval queue
    Ok((StatusCode::CREATED, Json(OrderIntentDto::from(&created))))
}
pub fn router() -> OpenApiRouter<AppState> { OpenApiRouter::new().routes(routes!(create)) }
```

- `AppState` is `Clone` (Arcs): ports from `bazaar_core::ports` (recommendations, approvals/`ApprovalQueue`, kill switch, ledger, journal, candles, instruments, `MarginSource`, `Clock`, `SecretStore`), the `Hub`, `Jobs`, `security: Security` (guards take `State<Security>`, so pass `state.security.clone()` to `from_fn_with_state`), the server origin and `web_dist`. `GET /api/v1/auth/me` lives in `routes/auth.rs` with `#[utoipa::path]` and returns `AuthMeDto { csrf_token, mode, live_enabled }` (CONTRACTS section 4). It is built by `build_state(config, adapters)`; `main` passes real adapters, tests pass fakes. Never put a `sqlx` pool, `reqwest::Client` or `Executor` in a handler signature.
- `ApiJson`, `ApiQuery`, `ApiPath` wrap the axum extractors (`#[derive(FromRequest)] #[from_request(via(axum::Json), rejection(ApiError))]`) so malformed input becomes problem+json: syntax 400, schema 422, wrong content type 415, oversize 413.
- `ApiError` variants map one-to-one to problem `code`s (table in the reference). `From<ApprovalError>`, `From<GateError>` etc. are explicit matches, no catch-all; `Internal` logs once with `tracing::error!` and returns a generic detail. `?` on a domain error is the normal path.
- List routes cap `limit` server-side and use cursors (`{ items, next_cursor }`); instrument keys and enums are parsed into domain types before the port is called (unknown instrument is 404 `instrument_not_found`, bad enum 422).

## 6. OpenAPI to UI types

- `openapi.rs` builds the document from the same `OpenApiRouter` that serves traffic (`nest("/api/v1", routes::api())`); `info.version` is the fixed string `"1"` so releases do not churn the types. Because `nest` prefixes the document, `paths` keys include `/api/v1` (`/api/v1/recommendations`): the UI client uses `baseUrl: location.origin` and full paths, and a server test asserts every document path starts with `/api/v1/`. The WebSocket envelope types (`WsOut`, `WsIn` and their data types in `dto/ws.rs`) are registered with `components(schemas(...))` on `ApiDoc` so the UI gets them generated too. `bazaar-server openapi` parses the CLI first and prints `document().to_pretty_json()` without config, Keychain, data dir or socket (`just openapi` pipes it to `web/openapi.json`, then `pnpm -C web gen:api` writes `schema.d.ts`; CI `openapi-drift` regenerates and fails on a diff).
- Every handler has `#[utoipa::path]` with all status codes it can return, `Problem` bodies as `application/problem+json`, and a `tag`. Register enums and shared DTOs via `ToSchema`; rename with `#[schema(...)]`, not by hand-editing output.
- `decimal` makes `rust_decimal::Decimal` a string in the schema; never enable `decimal_float`. Prefer the `DecimalStr` newtype so the wire form is explicit; a test asserts the schema type is `string`.
- The `RecommendationDto` fields come from `bz-quant`; do not copy them into this skill, the reference or a test name list.

## 7. WebSocket hub (`ws.rs`; messages in the reference)

- Upstream: the feed port delivers raw ticks into a bounded `mpsc`. One **coalescer** task keeps the latest tick per symbol and every ~250 ms (`tokio::time::interval`, `MissedTickBehavior::Skip`, period injectable for tests) publishes the batch on `Hub.ticks` (`broadcast`). Order, gate, job and status events go on a second `broadcast` (`Hub.events`) so a tick burst can never evict them.
- Each upgrade runs after `host_guard`, `origin_guard` and `session_guard`; reject over the connection cap (4) with 429 problem+json before upgrading. `max_message_size` 4 KiB. The hub also caches the last tick per symbol so a new `subscribe` gets a snapshot immediately.
- Session loop: split the socket; `select!` over shutdown token, a 20 s ping interval (close if nothing heard for 60 s), client frames, `ticks.recv()` filtered by that session's subscription set, `events.recv()`. Sends use a 5 s timeout; a slow or dead client is closed and the browser reconnects and refetches over REST.
- `RecvError::Lagged` is handled explicitly: send `{"type":"resync",...}` (UI invalidates the listed query groups) and continue; `Closed` ends the session. A gap in the coalescer clock (wall time jumped past a few seconds, i.e. the Mac slept) also emits `data_status` stale plus `resync`.
- Client frames: only `subscribe`/`unsubscribe`; malformed JSON, binary frames, more than 200 symbols or any order-like message close the socket with 1008. The socket never carries an action that changes state.
- Shutdown: `axum::serve` waits for open sockets, so sessions must watch the shutdown token and close with 1001; cap the whole drain at about 10 s.

```rust
loop {
    tokio::select! {
        _ = shutdown.cancelled() => { let _ = tx.send(close(1001)).await; break; }
        _ = ping.tick() => { if last_heard.elapsed() > STALE { break; } if send(&mut tx, Message::Ping(Bytes::new())).await.is_err() { break; } }
        frame = rx.next() => match frame { Some(Ok(m)) => { last_heard = Instant::now(); if !client_frame(m, &mut subs) { break; } } _ => break },
        t = ticks.recv() => match t {
            Ok(m) if subs.contains(&m.symbol) => if send_json(&mut tx, &m).await.is_err() { break },
            Ok(_) => {}
            Err(RecvError::Lagged(_)) => if send_json(&mut tx, &WsOut::resync("lagged")).await.is_err() { break },
            Err(RecvError::Closed) => break,
        },
        e = events.recv() => { /* same shape; every event goes to every session */ }
    }
}
```

## 8. Backtest jobs (`jobs.rs`)

`POST /backtests` validates (enabled strategy, params through the strategy's own `validate()`, date range inside stored data) and returns **202** with `Location` and a `BacktestJobDto`; one job runs at a time (second start is 409 `backtest_busy`). The job runs in `spawn_blocking` with a child `CancellationToken` and a throttled progress sink (at most ~4 updates/s) that publishes `job_progress` on `Hub.events` and updates the table that `GET /backtests/{id}` reads, so a missed WebSocket message costs nothing. `POST /backtests/{id}/cancel` cancels cooperatively (the runner checks `is_cancelled()` between bars or symbols) and is idempotent. Panics in the join handle become `failed` with a generic message. Results are persisted by the backtest repository, not the server; jobs found `running` at startup are marked `failed(interrupted)`. The runner/sink traits belong to `bz-backtest-risk`; agree the signature with that developer, do not invent one in `server`.

## 9. Startup, serving, sleep

`main.rs`: `dotenvy::dotenv().ok()` (before clap so `.env` reaches flag defaults; empty values count as unset; harmless for `openapi`) -> parse the CLI (`openapi` returns here) -> tracing -> load and validate config (fail fast) -> `SecretStore`/Keychain -> `build_state` -> `expire_stale()` approvals, audit-chain verify, mark interrupted jobs -> start feed, coalescer, jobs -> bind `SocketAddr::from(([127, 0, 0, 1], port))` (a fixed loopback constant, no address option; after bind assert `local_addr().ip().is_loopback()` and exit with a clear message if the port is taken) -> mint the launch token, write the launch URL once to stdout with `writeln!` (not `tracing`, so it never lands in a log file; no `println!`): `http://127.0.0.1:<port>/auth?token=<t>`, or `http://localhost:5173/auth?token=<t>` in debug builds; unless `--no-open`, run `open <url>` best effort -> `axum::serve(...).with_graceful_shutdown(shutdown.cancelled())`, where Ctrl-C or SIGTERM cancels the token -> drain, flush the store, exit 0.

After the Mac sleeps or wakes: assume sockets and the Kite feed are dead and timers fired late. Compute staleness from the injected `Clock` and `last_tick_at` when reading, never from a timer having run; `expire_stale()` also runs on every approval read and approve so TTLs hold across sleep; the feed adapter reconnects and gap-fills (`bz-market-data`); the hub publishes `data_status` (delayed) then `resync`; browsers reconnect on their own. `GET /status` must be truthful immediately after wake (kite token day, feed state, stale threshold).

## 10. Tests (unit tests in every PR; fakes and temp dirs only)

- `tests/common/mod.rs`: `TestApp::new()` builds `AppState` from fakes (`FakeClock`, in-memory ports, `FakeFeed`, `FakeExecutor` that counts calls), temp dirs and a temp `web/dist`, calls `app::build`, performs the real `/auth?token=` exchange and keeps the cookie and CSRF token; requests send `Host: 127.0.0.1:8787` and `Origin: http://127.0.0.1:8787`. Drive with `tower::ServiceExt::oneshot` (clone the router per request) and read bodies with `http_body_util::BodyExt`.
- Per route: happy path, 401 without session, 403 without CSRF on non-GET, validation 422, 404, and one domain-failure mapping; assert the problem `code`, never the English `detail`. Snapshot (`insta`) response JSON of representative DTOs.
- Contract tests that need no per-route work: every path in the OpenAPI document returns 401 without a cookie; no operation path contains `place` and none except the allowed list is a mutation; no mutation exists under `/settings`; schema of `DecimalStr` is `string`; the document equals a committed snapshot; unknown `/api/...` is 404 problem+json while unknown non-API paths serve `index.html`; `/assets/missing.js` is 404.
- Order flow with `FakeExecutor` and a fake gate: gate fail leaves `place` count 0; approve twice yields 200 then 409 and one `place`; expired approval 409 `approval_expired`; kill switch engaged blocks approve; drifted quote returns `quote_drifted` with the new risk. Needs `risk-guardian` review.
- WebSocket: start `axum::serve` on `127.0.0.1:0` inside the test and connect with `tokio-tungstenite` plus cookie and Origin. Check foreign Origin and missing cookie are refused; a tick burst yields one `tick` per symbol per window (coalescer period set to ~20 ms in the test config); unsubscribed symbols arrive never; a tiny broadcast capacity forces `resync`; shutdown closes with 1001. Unit-test the coalescer alone with `tokio::time::pause()` (no sockets).
- Jobs: fake runner reports progress and honours cancel; assert 202, `job_progress`, cancel to `cancelled`, busy 409, panic to `failed`.
- Startup: the bind-address function is loopback for any port; `openapi` subcommand output parses and contains `/api/v1`. Security tests (Host, Origin, CSRF, headers, token reuse, cookie flags, log redaction) are listed in `bz-testing` and owned by `bz-security`; keep them green, do not duplicate them.
- Run `cargo nextest run -p bazaar-server`; never `just run`, never the live broker, `just db-check` for connection status.

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
