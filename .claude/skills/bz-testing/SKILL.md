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

In short: `.config/nextest.toml` (flaky tests are fixed, not retried):
Full detail, examples and patterns: `references/tools.md` - read it before changing this area.

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

In short: The fixture tree is defined here and nowhere else (provenance in `fixtures/README.md`: source, capture date, what was scrubbed):
Full detail, examples and patterns: `references/fixtures-fakes-and-the-test-kit.md` - read it before changing this area.

## 5. Rust patterns

In short: Covers: Reference-value tests; Property tests: look-ahead and invariants; DecisionGate and discipline tests; Snapshots (`insta`); Golden backtest regression; Server router tests (`crates/server/tests/`); WebSocket tests; API contract drift.
Full detail, examples and patterns: `references/rust-patterns.md` - read it before changing this area.

## 6. UI tests (Vitest + MSW)

In short: The single definition of the mock layer, handlers, setup, `dev:mock`, Playwright config and CI steps is `references/ui-mocked-api.md`; `bz-web-ui` and `bz-repo-structure` link to it. Open it when writing the first UI test, a hook test, a ...
Full detail, examples and patterns: `references/ui-tests.md` - read it before changing this area.

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
