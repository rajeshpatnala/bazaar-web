# 4. Fixtures, fakes and the test kit

(From `bz-testing` SKILL.md; open when the task touches this area.)


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
