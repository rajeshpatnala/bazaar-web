---
name: bz-testing
description: Testing standard for the Bazaar Rust + axum + React trading app - reference-value tests, proptest look-ahead, fixture adapters (wiremock, loopback WebSocket), fake IST clock, insta, golden backtests, in-process router and WebSocket security tests, Vitest + MSW UI tests, Playwright smoke, coverage. Use when writing or reviewing any test.
---

# Bazaar Testing

Every PR ships unit tests for the code it changes (`just gate` runs them); a bug fix ships a test that failed before the fix. Money, risk, look-ahead and browser-security bugs are the expensive ones, so those areas get the strictest tests. Conventions for code under test come from `bz-core`; layering from `bz-architecture`; layout and recipes from `bz-repo-structure`; routes, auth and the OpenAPI contract from `bz-web-api`; the UI from `bz-web-ui`; per-area facts from `bz-market-data`, `bz-quant`, `bz-backtest-risk`, `bz-execution`; market facts from `bz-india-markets`; the rules the gate must enforce from `bz-trading-discipline`; evidence expectations from `bz-market-strategy`; browser-security rules from `bz-security`.

**Kit guarantees for tests.** Tests never call the live Kite API, a real exchange site or a real broker. They never read `.env`, the macOS Keychain or real tokens; credentials in tests are obvious fakes (`"test_api_key"`), secrets come from an in-memory `SecretStore` fake. Stores are ephemeral (in-memory SQLite, temp dirs for Parquet/DuckDB), never the user's `data/`. The only connection check is `just db-check` (`bazaar-cli check-connections`), never a test. Server tests run the router in-process or on a loopback port chosen by the OS (`127.0.0.1:0`), never on 8787. T1 developer checks use fixtures and temp dirs (`just cli-fixture ...`). Human-only commands (`just run`, `just ui-dev`, `refresh-instruments`, `import-bhavcopy`, `backtest` against `data/`) are never run by agents. Agents do not record fixtures from live services; they write synthetic ones in the documented vendor shape, or ask the user for a scrubbed recording.

---

## 1. Test pyramid

| Level | Where | Proves | Tools |
|---|---|---|---|
| Unit | `#[cfg(test)] mod tests` | Pure logic: sizing, costs, gate rules, indicators, state machines | `cargo nextest`, `rstest`, `dec!` |
| Reference-value | same crate, data in `fixtures/reference/` | Indicators and option pricing match published or TA-Lib values | tolerance helpers |
| Property | same crate | No look-ahead, cash conservation, no panic on any input | `proptest` 1.x |
| Fixture integration | `crates/<crate>/tests/*.rs` | Adapters and pipelines against recordings and fakes | `wiremock` 0.6, loopback WS server, in-memory SQLite |
| Snapshot / golden | `tests/` + `fixtures/golden/` | Recommendation text, Why panels, backtest stats, OpenAPI paths do not drift | `insta` 1.x, JSON baseline |
| Server router | `crates/server/tests/*.rs` | Auth, Host/Origin/CSRF, headers, approval flow, WebSocket, "no route places an order" | `tower::ServiceExt::oneshot`, fake ports, loopback `tokio-tungstenite` |
| UI component | `web/src/**/*.test.tsx` | Components and hooks against mocked REST and WebSocket | Vitest 5, Testing Library, MSW 3 |
| E2E smoke | `web/e2e/*.spec.ts` | Screens work in a real browser on the mocked app (CI only) | Playwright on `dev:mock` |

Rule of thumb: about 65% unit/property, 20% fixture and server integration, 15% UI. Logic is tested without I/O; adapters against realistic recordings; there is no live-API level.

## 2. Tools

| Tool | Use |
|---|---|
| `cargo-nextest` | Default runner (`.config/nextest.toml`); doc tests via `cargo test --doc --workspace`. |
| `proptest`, `insta`, `wiremock`, `tempfile` | Generators (commit `proptest-regressions/`); snapshots (review diffs, CI never updates); Kite REST stand-in; temp dirs. |
| `tower` (`util`), `http-body-util` | Dev-dependencies of `bazaar-server`: `ServiceExt::oneshot` and body collection for router tests. |
| `tokio-tungstenite` | Server side of the Kite ticker fake, and the client in server WebSocket tests. Keep the same minor as the one `axum` pulls in (`cargo tree -i tungstenite`) so there is one `Message` type. |
| `cargo-llvm-cov`, `cargo-mutants`, `criterion` | Coverage with nextest; periodic mutation run on `risk`, cost model, order state machine; benchmarks only. |
| Vitest 5, `@testing-library/react`, `user-event`, `jest-dom`, `@vitest/coverage-v8` | UI tests under jsdom. Config in `web/vitest.config.ts` (Vitest 5 does not search parent folders; `clearMocks` now defaults to true). |
| MSW 3 | REST and WebSocket mocks for Vitest (`msw/node`), `dev:mock` (`msw/browser`) and Playwright. Needs Node 22+. Details: `references/ui-mocked-api.md`. |
| Playwright (1.65 line) | Browser smoke against `dev:mock`, CI only. |

`.config/nextest.toml` (flaky tests are fixed, not retried):

```toml
[profile.default]
fail-fast = true

[profile.ci]
retries = 0
fail-fast = false
slow-timeout = { period = "30s", terminate-after = 4 }

[profile.ci.junit]
path = "junit.xml"
```

## 3. Golden rules

1. A test never depends on wall-clock time, the current date, the network, env vars, a fixed port or test order. Inject `Clock`; load calendars, expiries and lot sizes from fixtures (`bz-india-markets`).
2. Money is `Decimal`. Assert exact equality on Decimal results; tolerances only for f64 indicator/Greeks math (`bz-quant`). In the UI money is a decimal string and is asserted as a string.
3. Indicators and pricing are verified against external reference values, not against the implementation.
4. Every strategy and the backtester carry a look-ahead property test named with the `lookahead_` prefix (5.2); `just lookahead` fails when nothing matches.
5. Every DecisionGate rule has a passing and a failing test, and a test that no path skips the gate (`bz-trading-discipline`).
6. Config keys behave: a test per `risk.toml`/`charges.toml` key proving the value is read (change it in a test config, behaviour changes).
7. Cost-arithmetic tests use a synthetic `charges.toml` fixture (round numbers); a separate test checks the real `config/*.toml` files parse, validate and carry `verified_on`. Real rates are never typed from memory into expectations.
8. No `sleep`. Use `FakeClock`, `#[tokio::test(start_paused = true)]`, injected sleepers, or Vitest fake timers.
9. Test names state behaviour: `rejects_order_when_daily_loss_limit_reached`.
10. `unwrap`/`expect` are fine in tests (prefer `expect("why")`); never in library code.
11. Snapshots hold no secrets, account ids or tokens; redact volatile fields.
12. `live-trading` is off by default: CI compiles and clippy-checks it but no test reaches a real broker. Packages in `-p` flags are always `bazaar-<dir>`.
13. Each security check gets its own test where every other check passes (valid session, CSRF and Origin, then break one thing). A test that fails for two reasons proves neither.
14. The UI test network is closed: MSW runs with unhandled requests set to `error`, so a call nobody anticipated fails the test.
15. Mock handlers and fixtures are typed from the generated `schema.d.ts`; never hand-write a DTO shape in a test.

## 4. Fixtures, fakes and the test kit

The fixture tree is defined here and nowhere else (provenance in `fixtures/README.md`: source, capture date, what was scrubbed):

```
fixtures/
  config/              synthetic copies of all 8 config files, used with --config-dir (calendar lives here)
  kite/rest/ kite/ws/  REST JSON (instruments, historical, quote, margins, orders, error envelopes); ws/*.hex frames
  bhavcopy/            small NSE/BSE UDiFF samples (equity, F&O), plus a malformed variant per parser
  reference/indicators/ reference/options/   input series, expected output, source (CSV/JSON)
  charges/             synthetic rate sets for cost-arithmetic tests
  paper/               paper-fill scenarios
  golden/              backtest baselines and snapshot inputs
```

Tolerances (only here): closed-form values (SMA/EMA/RSI after warm-up, Black-Scholes/Black-76 prices, Greeks) `1e-8` absolute; iterative solvers (implied volatility) `1e-6`. Reference tests take expiry/close times from `fixtures/config/calendar.toml`, never a literal.

Builders (`BarSeriesBuilder`, `OrderIntentBuilder`, `FakeClock`, `fixtures_dir()`, in-memory `FakeExecutor`/`FakeMarketData`/`FakeStore`, `InMemorySecretStore`) live in `crates/core` behind the `test-util` feature, enabled only from `dev-dependencies` (`bz-core` `references/money-time-config.md`). A separate testkit crate needs an ADR (`bz-architecture`). The UI equivalents are `web/src/test/factories.ts` and `web/src/mocks/fixtures.ts`.

**Fake clock** for IST and session logic:

```rust
let clock = FakeClock::at_ist(2026, 11, 18, 15, 29, 0);   // fixture calendar closes this segment at 15:30
let gate = DecisionGate::new(cfg, clock.clone());
clock.advance(Duration::minutes(2));
assert!(matches!(gate.check(&intent), Err(Reject::MarketClosed)));
```

Cover (times from the fixture calendar): pre-open, last minute, exactly the close, non-F&O vs F&O close, MIS square-off, holiday and weekend rollover, expiry shifted by a holiday, UTC vs IST date boundary (around 18:30 UTC).

**Kite REST with `wiremock`.** The client takes its base URL from config so tests point it at `MockServer::uri()`; fake token values. Cover: auth header present, success decoded from a fixture, Kite error envelope mapped to typed errors, `429` backs off per the limiter, `5xx` retried with bounded attempts, malformed JSON gives `Err` not panic, empty candle list, expired token moves to "login required" instead of a retry loop. The same mock server stands in for the token exchange in the Kite callback route test.

```rust
Mock::given(method("GET")).and(path("/instruments/historical/738561/5minute"))
    .and(header_exists("Authorization"))
    .respond_with(ResponseTemplate::new(200)
        .set_body_string(include_str!("../../../fixtures/kite/rest/historical_5min.json")))
    .expect(1).mount(&server).await;
```

**Kite WebSocket ticker** (`bz-market-data`): loopback server on `127.0.0.1:0` replays `fixtures/kite/ws/` frames, then scripts failures: drop mid-stream, truncated binary packet, silence past the heartbeat timeout, out-of-order timestamps. Assert reconnect with resubscribe, no panic, stale flag, de-duplicated ticks. Test the backoff schedule as a pure function; real sockets and paused time mix badly (auto-advance can fire timeouts early), so socket tests use short real timeouts.

**Stores.** SQLite: in-memory pool (single connection) or `#[sqlx::test]` with migrations applied; DuckDB/Parquet: write into `tempfile::tempdir()` from a fixture and query through the store API. **Secrets:** code takes `SecretStore`; tests inject the in-memory fake; add a test that `Debug`/log output of config and token types is redacted (`bz-security`).

## 5. Rust patterns

### 5.1 Reference-value tests

Input and expected output live in `fixtures/reference/{indicators,options}/<name>.csv` with a header comment citing the source (TA-Lib output, textbook example, exchange value). Compare with a helper taking an absolute tolerance that reports the first differing index. Cover warm-up length, flat series, single bar, NaN-free output, streaming vs batch equality. Options: Black-76 and Black-Scholes vs published values, put-call parity, IV round trip, deep ITM/OTM, near-expiry, zero time to expiry. Max pain, PCR and OI build-up use small hand-computed chains.

### 5.2 Property tests: look-ahead and invariants

Look-ahead is the most dangerous silent bug: results look better than reality. Assert that changing future bars never changes past decisions, for every strategy and the backtester. Prefix `lookahead_` (`just lookahead` = `cargo nextest run --workspace -E 'test(/lookahead_/)' --no-tests=fail`, so a rename that matches nothing fails). Design side (`DataView`): `bz-backtest-risk`.

```rust
proptest! {
    #[test]
    fn lookahead_decisions_up_to_t_ignore_future_bars(
        bars in arb_bars(60..300), t in 30usize..59, noise in arb_bars(1..100),
    ) {
        let t = t.min(bars.len() - 1);
        let base = run_strategy(&bars);
        let mut perturbed = bars.clone();
        perturbed.truncate(t + 1);
        perturbed.extend(noise.into_iter().map(|b| b.rebased_after(&bars[t])));
        let alt = run_strategy(&perturbed);
        prop_assert_eq!(&base.decisions[..=t], &alt.decisions[..=t]);
    }
}
```

Also `lookahead_streaming_equals_batch` (bar-by-bar signals equal the batch driver) and `lookahead_indicator_value_ignores_later_bars` (`value[i]` unchanged by truncating after `i`). Other invariants:
- `cash_is_conserved` (paper cash + position cost + charges == starting cash + realized P&L), `equity_curve_matches_ledger`, `no_fill_outside_session`, `no_fill_at_circuit`, `limit_order_fills_only_if_traded_through`.
- `position_size_never_exceeds_risk` (`qty * stop_distance <= risk_per_trade * capital`, qty a lot multiple, `0` allowed), `costs_non_negative_and_monotonic_in_turnover`, `adjusted_prices_continuous_across_split`.
- `order_state_machine_has_no_illegal_transition`; terminal states absorbing.
- Parsers (bhavcopy, instrument CSV, Kite packets, WebSocket messages): `never_panics_on_arbitrary_input`, plus round trip where an encoder exists.

Generators stay realistic (high >= max(open, close) >= min(open, close) >= low, positive prices, tick multiples). Raise `PROPTEST_CASES` in nightly runs.

### 5.3 DecisionGate and discipline tests

Table-driven (`rstest`) per `risk.toml` rule: evidence bar (29 vs 30 OOS trades), per-trade risk, daily loss at exactly the limit, max positions, cool-off, no stop widening, no averaging down, entry zone, ban list and event days, override needs a typed reason and writes an audit record. Boundary values: limit, limit - 1 paisa, limit + 1 paisa. Structural tests: `Executor::place` is reachable only after `DecisionGate` plus approval; default `ExecutionMode` is `paper`; `auto` stays locked without prerequisites; kill switch blocks sends even with an approved intent.

### 5.4 Snapshots (`insta`)

Snapshot Recommendation DTOs (all fields present, low-evidence flag under 30 trades), Why? text, gate rejection messages, exports, and the sorted list of `(method, path)` pairs from the OpenAPI document (a new route is a reviewed diff; see 5.6). Wording is product: a text diff gets human or strategist review. `assert_json_snapshot!` with redactions for ids and timestamps; inputs from fixtures and `FakeClock`. Never `INSTA_UPDATE=always` in scripts.

### 5.5 Golden backtest regression

`fixtures/golden/<strategy>_<dataset>.stats.json` holds committed stats (trades, win rate, profit factor, expectancy, max drawdown, net P&L, total charges) from a fixed dataset and params. Integers and Decimals exact, ratios within a small tolerance. A diff is a bug or an intended change; intended changes update the baseline in the same PR with an explanation and quant-reviewer sign-off. Never regenerate just to go green. `just golden` runs it.

### 5.6 Server router tests (`crates/server/tests/`)

Build the router the production way: `app.rs` exposes a function that takes the composed `AppState` and returns `Router`; tests call it with fakes (`FakeExecutor`, `FakeMarketData`, `FakeStore` or in-memory SQLite, `InMemorySecretStore`, `FakeClock`, Kite base URL pointing at wiremock) and a `tempfile::TempDir`. Drive it with `tower::ServiceExt::oneshot` on a clone per call (`oneshot` consumes the service). If a handler reads `ConnectInfo`, add `MockConnectInfo` in the test app.

```rust
// tests/common/mod.rs
pub struct TestApp { pub router: Router, pub executor: Arc<FakeExecutor>, pub token: String, _tmp: TempDir }
pub struct Session { pub cookie: String, pub csrf: String }

pub async fn send(app: &TestApp, req: Request<Body>) -> Response<Body> {
    app.router.clone().oneshot(req).await.expect("router is infallible")
}
// login(): GET /auth?token=<t> (Host 127.0.0.1:<port>) -> 303 + Set-Cookie bz_session;
// then GET /api/v1/auth/me with the cookie -> csrf_token. Returns Session.
// req(method, path, &Session) builds a request with Host, Origin, Cookie, X-CSRF-Token all valid;
// each security test then removes or corrupts exactly one of them.
```

Mandatory security tests (this table is the only list; `bz-security` section 12 points here; names are behaviours; run them against every route from the generated OpenAPI document, not a hand-kept list, so a new route is covered automatically; exemptions are an explicit short list):

| Test | Asserts |
|---|---|
| `rejects_wrong_host` | Host `evil.test`, `127.0.0.1.evil.test:8787`, a wrong port, missing Host: 403 on API, `/healthz` and static paths; `localhost:<port>` and `127.0.0.1:<port>` pass |
| `rejects_missing_or_foreign_origin_on_post` | every non-GET (POST, PUT, DELETE): no Origin, `http://evil.test`, `null`, same host other port: 403 even with valid session and CSRF |
| `rejects_missing_or_foreign_origin_on_ws_upgrade` | a correct upgrade request (Upgrade, Connection, Sec-WebSocket-Version/Key, valid cookie) with bad or no Origin: 403 from the check, not from a malformed handshake |
| `rejects_missing_or_wrong_csrf` | no header, wrong value, token of another session: 403 on every non-GET; GET needs none |
| `requires_session_except_exempt` | no cookie or unknown cookie: 401 everywhere except `/healthz`, `/auth`, `/api/v1/kite/callback` (and static UI files) |
| `launch_token_is_single_use` | first `/auth?token=` gives 303 to `/`, cookie `HttpOnly; SameSite=Strict; Path=/` without `Secure`, no token in `Location`; second use, wrong token and truncated token are rejected with no `Set-Cookie` |
| `never_sends_cors_headers` | no header starting `access-control-` on 2xx, 3xx, 401, 403, 404, 413 and on an `OPTIONS` preflight with a foreign Origin |
| `security_headers_present` | on API, static, error and 404 responses: exact CSP string from the shared constant, `nosniff`, `Referrer-Policy: no-referrer`; `Cache-Control: no-store` on `/api/*` |
| `approve_twice_returns_409` | create intent from a fixture recommendation, approve (200), approve again: 409 `application/problem+json`; executor shows one placement |
| `no_route_places_an_order` | (a) OpenAPI `(method, path)` snapshot has no place/order-send operation; (b) POST/PUT to `/api/v1/orders`, `/api/v1/orders/place` and similar guesses give 404/405; (c) `FakeExecutor` placement count is 0 after create-intent, after approve with a failing gate, after approve with the kill switch engaged, and after approve with an expired approval |
| `body_limit_and_problem_json` | 1 MiB + 1 byte body gives 413; errors are `application/problem+json` with no stack, path or secret text |
| `kite_callback_nonce` | callback works without a session only with a matching unused nonce; reuse, missing and expired nonce are rejected; token exchange hits wiremock; the access token goes to the fake `SecretStore` and appears in no response body or log capture |
| `bind_is_loopback_only` | `ensure_loopback` accepts `127.0.0.1` and rejects `0.0.0.0`, `::`, `::1` and a LAN address |
| `launch_token_not_logged` | the token text never appears in captured tracing output (with `TraceLayer` on), and `/auth` rejects a token older than 10 minutes (`FakeClock`) |
| `no_secret_in_any_response` | the fake token and API secret strings are absent from every response body and header in a scripted session |

Other server tests: each route's happy path and error mapping (`GateRejected` to 422 problem with the failing rules, unknown id 404), `GET /api/v1/status` has no secrets, kill-switch `PUT` is idempotent and audited, kill switch engaged blocks approve, the server refuses a non-loopback bind (pure function on the address config), and `ServeDir` serves a temp `web/dist` with the same security headers.

### 5.7 WebSocket tests

Two layers, because `oneshot` has no real connection to upgrade:

1. **Handshake rejections via `oneshot`:** bad Host, bad or missing Origin, no session. The checks run before the upgrade, so a 403/401 response is observable in-process (see the table above).
2. **Accepted connections on a real loopback port:** bind `TcpListener` on `127.0.0.1:0`, read `local_addr()`, build the app with that port as the allowed Host/Origin port (the Host header a client sends is the ephemeral one, so a hard-coded 8787 would make every test 403), run `axum::serve` in a spawned task with graceful shutdown, connect with `tokio_tungstenite::connect_async` using a request built by `IntoClientRequest` and extended with `Origin` and `Cookie` headers. This is the pattern axum's own `testing-websockets` example uses.

```rust
let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
let port = listener.local_addr().unwrap().port();
let app = TestApp::with_port(port).await;                       // allowed Host/Origin use `port`
let (stop, rx) = tokio::sync::oneshot::channel::<()>();
let server = tokio::spawn(axum::serve(listener, app.router.clone())
    .with_graceful_shutdown(async { let _ = rx.await; }).into_future());

let mut req = format!("ws://127.0.0.1:{port}/api/v1/ws").into_client_request().unwrap();
req.headers_mut().insert("Origin", format!("http://127.0.0.1:{port}").parse().unwrap());
req.headers_mut().insert("Cookie", session.cookie.parse().unwrap());
let (mut ws, _) = connect_async(req).await.expect("upgrade accepted");
app.market.push_tick(fixture_tick("NIFTY 50"));                 // FakeMarketData feeds the hub
let msg = timeout(Duration::from_secs(2), ws.next()).await.expect("tick in time").unwrap().unwrap();
// axum 0.8 / tungstenite 0.2x: Message::Text holds Utf8Bytes -> msg.to_text()
```

Assert: message is `{ "type", "data" }` with a known type; bursts of ticks are throttled to the configured interval (set it short in the test config); `gate_alert` and `order_update` arrive after the matching action over REST; after an abnormal close the client logic calls `GET /api/v1/auth/me` (there is no auth close code: 401 means session ended, anything else means reconnect); a client message that is not valid JSON or too large does not crash the server; unknown client message types are ignored. Always wrap reads in `timeout` and end with `stop.send(())` plus `server.await`, so a failing test cannot hang CI. The `axum-test` crate (`ws` feature) is a possible convenience; the default stays `oneshot` plus loopback so the kit has no extra dependency.

### 5.8 API contract drift

`just openapi` writes `web/openapi.json` and regenerates `web/src/api/schema.d.ts`. `web/openapi.json` is gitignored (a build intermediate). Two guards: the server's `insta` snapshot of the utoipa document (`bz-web-api` section 10), and the CI `openapi-drift` job that runs `just openapi` then `git diff --exit-code web/src/api/schema.d.ts`. UI mocks and factories import types from `schema.d.ts`, so a DTO change fails `pnpm -C web typecheck` in the handlers before it fails a screen.

## 6. UI tests (Vitest + MSW)

The single definition of the mock layer, handlers, setup, `dev:mock`, Playwright config and CI steps is `references/ui-mocked-api.md`; `bz-web-ui` and `bz-repo-structure` link to it. Open it when writing the first UI test, a hook test, a WebSocket test or the Playwright config.

Essentials:
- Shared handlers in `web/src/mocks/handlers.ts` (REST and the `/api/v1/ws` link) serve deterministic fixtures; the mock enforces the same `X-CSRF-Token` rule as the server, so the UI's CSRF middleware is exercised. Per-test behaviour uses `server.use(...)`; `afterEach` resets it.
- `server.listen({ onUnhandledFrame: "error" })` (MSW 3 name; it was `onUnhandledRequest` in MSW 2): an unanticipated API call fails the test.
- Fresh `QueryClient` per test (`retry: false`), rendered through one `renderWithProviders` helper (router + query client).
- Query by role and visible text, not class names or test ids unless necessary. Assert request bodies and headers by recording requests inside the override handler.
- lightweight-charts needs a canvas: `vi.mock` the module in component tests and unit-test the pure mappers separately (`bz-web-ui` `references/lightweight-charts-v5.md`).
- Must-have UI assertions: disclaimer on every route and in error state; "data delayed" badge when the socket drops or ticks stop; approval dialog shows PAPER/LIVE label and rupee risk before Approve is enabled, and exactly one approve request is sent per click (a 409 from a second attempt shows a plain message); low-evidence badge under 30 trades; Why? panel renders each field; a 401 from `/auth/me` shows the signed-out state; no order-related request fires without the user's click.
- Formatting tests (`en-IN` grouping, rupee sign, IST via `Asia/Kolkata`) per `bz-web-ui` section 7, run with `TZ=UTC` and `TZ=America/New_York`.
- Fake timers (`vi.useFakeTimers({ shouldAdvanceTime: true })`, `await vi.advanceTimersByTimeAsync(...)`) for throttles, reconnect backoff and stale timers.

Playwright smoke runs against `dev:mock` (port 5173) in CI only; agents capture one screenshot of the changed screen from `just app-dev` for T1 evidence instead. One spec per screen: loads with no console errors, shows the disclaimer, renders the recommendation card from fixtures, paper approval flow; plus a CSP spec on a production-style build. Spec list and config: `references/ui-mocked-api.md`.

## 7. Coverage targets

The enforced floor is **75% lines**: workspace in CI via `--fail-under-lines 75`; `web/` via the Vitest coverage threshold (`vitest run --coverage`). Review targets above the floor:

| Scope | Review target |
|---|---|
| `core` (Money, calendar, clock), `risk`, `execution`, `paper`, `backtest` cost model | >= 90%, plus property tests |
| `indicators`, `options` | >= 90%, plus reference-value tests |
| `data` parsers and decoders | >= 90%, plus never-panic property test |
| `server` `security.rs` and auth/session code | >= 90%; every row of the 5.6 table present |
| everything else | >= 75% |

Coverage is a floor, not a goal; reviewers check error paths and boundaries are asserted. Run `cargo mutants -p bazaar-risk` (and the cost model, order state machine, `security.rs`) before releases and kill surviving mutants in money and security logic.

## 8. Commands

```bash
just gate                                              # fmt, clippy, nextest, web typecheck/lint/test:ci
cargo nextest run -p bazaar-server                     # server router + WebSocket tests
cargo nextest run -p bazaar-risk                       # one crate
just lookahead                                         # look-ahead suite, fails if none ran
just golden                                            # golden backtest regression
cargo nextest run --workspace --profile ci             # CI profile
cargo test --doc --workspace                           # doc tests
cargo llvm-cov nextest --workspace --no-tests=pass --lcov --output-path lcov.info --fail-under-lines 75
cargo insta test --review                              # human review of snapshot diffs
just openapi && git diff --exit-code web/src/api/schema.d.ts   # contract drift (CI)
pnpm -C web test:ci                                    # vitest run --coverage --passWithNoTests (MSW, jsdom)
pnpm -C web e2e                                        # Playwright smoke on dev:mock (CI)
```

Tiers: T1 per slice runs the changed unit against a fixture (`cargo nextest run -p bazaar-<crate> <test>`, `just cli-fixture ...`) or one mocked screenshot (`just app-dev`); T2 per feature adds the full gate, `just check`, `just lookahead`, golden regression, server security tests, Vitest, and reviewer sign-off (quant-reviewer, risk-guardian where the paths apply). Playwright runs in CI.

## 9. Red flags in tests

Reject in review (ripgrep patterns): `reqwest::get\(|api\.kite\.trade|kite\.zerodha\.com` in tests; `dotenv|\.env` reads; `std::env::var\("(KITE|API)`; `thread::sleep|tokio::time::sleep` without paused time; `Utc::now\(\)|Local::now\(\)|SystemTime::now` in logic under test; `f64` for rupee amounts; `#\[ignore\]` live tests added by an agent; `INSTA_UPDATE=always`; golden file edited without explanation; hardcoded real STT/brokerage rates or expiry weekdays; `skip_gate` or gate-bypass helpers outside `test-util`; tokens or account ids in fixtures; `-p` without `bazaar-`; look-ahead test without `lookahead_`; `127.0.0.1:8787` or `bind(.*:8787)` in server tests; a router test that calls a handler directly instead of going through the middleware stack; `onUnhandledFrame: "bypass"|"warn"` or `onUnhandledRequest` in `web/src/test/`; `http://127\.0\.0\.1:8787` or `localhost:8787` in `web/src` tests or mocks; hand-written DTO literals in handlers instead of factories typed from `schema.d.ts`; `vi.mock\("openapi-fetch"` (mock the network, not the client); `waitForTimeout` in Playwright. The full security scan is `bz-security`.

## 10. PR checklist

- [ ] Unit tests for every changed function or component; bug fix has a regression test that fails without the fix.
- [ ] Boundary values: zero, one paisa over/under a limit, empty series, single bar, tick-size and lot-size edges.
- [ ] Indicator/pricing change: reference-value test with cited source; streaming equals batch.
- [ ] Strategy or backtester change: `lookahead_` test passes (`just lookahead`); golden baseline unchanged or updated with explanation.
- [ ] Gate/risk/execution change: pass and fail test per rule; no bypass; audit record asserted; risk-guardian reviewed.
- [ ] Adapter change: wiremock/loopback fixtures cover success, error envelope, rate limit, malformed payload, reconnect; no live API.
- [ ] Route, middleware or WebSocket change: router test for success and each rejection (401/403/409/problem+json); security table in 5.6 still green; OpenAPI snapshot and `schema.d.ts` regenerated.
- [ ] Time logic uses `FakeClock`; IST boundaries covered; no sleeps.
- [ ] Config keys: value read from config and validated; `verified_on` present.
- [ ] UI: component test with MSW (typed handlers, unhandled requests error); disclaimer, stale badge and approval assertions where touched; en-IN and IST formatting checked.
- [ ] Snapshots reviewed line by line; fixtures scrubbed and provenance noted.
- [ ] `just gate` green; `just check` green for features.
