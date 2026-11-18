# 5. Rust patterns

(From `bz-testing` SKILL.md; open when the task touches this area.)


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
