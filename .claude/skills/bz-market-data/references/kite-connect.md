# Kite Connect v3 reference (data side)

Read when building or changing the Kite adapter. Protocol shapes below were read from the official docs on 2026-11-18. Docs and plans change: re-check before relying on anything. **Every numeric limit (rates, batch sizes, caps, history windows) lives only in `config/data.toml` with `verified_on` and `source`** (schema in SKILL.md section 3a); this file names the keys and the page to read, not the numbers.

Docs: <https://kite.trade/docs/connect/v3/> (user/login: `/user/`, market quotes and instruments: `/market-quotes/`, historical: `/historical/`, WebSocket: `/websocket/`, errors and limits: `/exceptions/`). Plan and pricing: unverified here; confirm at the Zerodha support page on Kite API charges and the current plan page (which plan includes live and historical data, and GST).

## 1. Daily login flow

1. The owner (with a session in the local web UI) opens `GET /api/v1/kite/login`, which redirects the browser to `https://kite.zerodha.com/connect/login?v=3&api_key=<key>&redirect_params=nonce%3D<nonce>` (nonce rules: `bz-security` section 5).
2. After login (with TOTP), Kite redirects to the redirect URL registered for the API key with `request_token` in the query. It is short-lived and single use: exchange immediately.
3. `POST /session/token` with form fields `api_key`, `request_token`, `checksum`, header `X-Kite-Version: 3`. `checksum` = SHA-256 hex of `api_key + request_token + api_secret`.
4. Response data holds `access_token` (plus `public_token`, profile). All later calls send `Authorization: token <api_key>:<access_token>` and `X-Kite-Version: 3`.
5. The `access_token` expires daily (the docs give the exact expiry time; confirm there) or earlier on a master logout in the Kite web app. Treat a 403 with `TokenException` as "login required", clear the stored token, stop retrying.

Implementation notes:
- Redirect capture: the redirect URL registered for the API key is the server's `/api/v1/kite/callback` (`http://127.0.0.1:8787/api/v1/kite/callback`; Bazaar is local-only). The handler reads `request_token` and `status`, checks a one-time nonce issued by `/api/v1/kite/login` and round-tripped via Kite's `redirect_params` (verify in the login docs; reject mismatches; the cross-site redirect will not carry a `SameSite=Strict` session cookie), exchanges the token, then redirects to the UI. Ignore anything else; never log the query string.
- The checksum is computed in Rust (`sha2`); `api_key`/`api_secret` come from the `SecretStore` port (macOS Keychain first, `.env` fallback loaded by the program; `bz-security`). Keep the `access_token` in memory with an `obtained_at` marker and in the Keychain via `SecretStore` so a restart the same day needs no re-login. Never write it to SQLite, logs or the Parquet store.
- Hold `api_key`, `api_secret`, `request_token` and `access_token` as `secrecy::SecretString`; expose a value only when building the header or the checksum. `reqwest` request builders that set the header must not be logged at debug level (`tracing` fields exclude headers).
- The SEBI retail algo framework (static IP whitelisting for order APIs, daily 2FA, no perpetual sessions) applies to order placement; this crate only reads data. See `bz-security` `references/sebi-algo-compliance.md` before any live-trading work.

## 2. Instruments dump

- `GET /instruments` (all) or `GET /instruments/<exchange>` returns gzip-compressed CSV, not JSON. Regenerated once a day; the docs advise fetching it once near the start of the day (time: `[refresh] instruments_after_ist` in `data.toml`) and storing it yourself.
- Columns: `instrument_token, exchange_token, tradingsymbol, name, last_price, expiry, strike, tick_size, lot_size, instrument_type, segment, exchange`.
- Natural key is `(exchange, tradingsymbol)`. Tokens can be reused for derivatives after expiry, so do not use `instrument_token` as a long-term key; keep it as a daily lookup column.
- `last_price` in the dump is not live; ignore it for pricing.
- Parse with `csv` + serde into a raw struct, then `TryFrom` into `Instrument` (Decimal tick and strike, `NaiveDate` expiry, enum segment/instrument type with an `Other(String)` fallback so a new type does not break the import).
- UDiFF-driven file format changes were discussed on the Kite forum (<https://kite.trade/forum/discussion/14147/instrument-file-format-changes-based-on-udiff>); keep the parser tolerant of extra columns and verify the header at runtime, failing with a clear `Decode` error if a required column is missing.

## 3. Quotes (snapshots)

- `/quote` (full, with depth and OI), `/quote/ohlc`, `/quote/ltp`; identifiers are repeated `i=EXCHANGE:TRADINGSYMBOL` query params. The docs state a per-request instrument cap per endpoint (`quote_batch_size`, `ltp_batch_size` in `data.toml`). The quote endpoints have a stricter rate limit than the rest (section 6). Chunk requests accordingly and tolerate missing keys in the response (no data for that instrument).

## 4. Historical candles

- `GET /instruments/historical/<instrument_token>/<interval>?from=...&to=...[&oi=1][&continuous=1]`.
- Intervals: `minute, 3minute, 5minute, 10minute, 15minute, 30minute, 60minute, day`.
- `from`/`to` format `yyyy-mm-dd hh:mm:ss` (exchange local time, IST). Candle = `[timestamp, open, high, low, close, volume]`, plus OI as a 7th value when `oi=1`. Timestamps carry a `+0530` offset: parse into `DateTime<FixedOffset>` and convert to `Asia/Kolkata`.
- `continuous=1` gives `day` candles for expired NFO/MCX futures using a live contract's token.
- The maximum number of days per request per interval is not stated on the docs page. Do not hardcode it: check the current docs/forum at <https://kite.trade/forum/> and put the per-interval window in `config/data.toml` (`[kite.historical] window_days.<interval>`, with `verified_on`); the downloader splits ranges by that window and refuses to run for an interval with no value.
- Download plan for backfill: iterate instruments x windows, pace at the documented historical rate, resume from the last stored bar (import log), upsert by `(symbol, interval, ts)`. Intraday bars are candles in IST; the first bar of the day opens at the session open (`bz-india-markets`).
- Never store the last, still-forming candle of the current day as final; mark partial or drop it until the session closes.

## 5. WebSocket ticker

- URL `wss://ws.kite.trade?api_key=<key>&access_token=<token>` (token in the query string, so never log the URL; build it in the connect function and drop it).
- The docs state caps on instruments per connection and connections per API key; they live in `data.toml` (`ws_max_instruments_per_connection`, `ws_max_connections`), checked against the docs on `verified_on`.
- Client messages are JSON `{"a": <action>, "v": <value>}` with actions `subscribe` (token list), `unsubscribe` (token list), `mode` (`[mode, [tokens]]`). Modes: `ltp`, `quote`, `full`. Subscribing sets a default mode; send `mode` after `subscribe` for tokens you want in a different mode, and track the mode per token in `Desired`.
- Server frames:
  - Binary = market data. Layout: `int16` packet count, then for each packet an `int16` length followed by that many bytes. All integers big-endian.
  - 1-byte binary frames are heartbeats (the docs say sent about every couple of seconds when idle): ignore, but count as liveness.
  - Text = JSON `{type, data}` with types `order` (postback), `error`, `message`. Parse defensively; do not log `order` payloads (account data) at info level.
- Packet size identifies the shape:

| Length (bytes) | Shape |
|---|---|
| 8 | LTP: token, last price |
| 28 | Index, quote mode: token, LTP, high, low, open, close, change, (no timestamp) |
| 32 | Index, full mode: as above plus exchange timestamp |
| 44 | Tradable instrument, quote mode (no depth) |
| 184 | Tradable instrument, full mode (44 bytes of quote fields, timestamps, OI, then 120 bytes of depth) |

  The docs describe tradable packets as: token (int32), LTP, last traded quantity, average price, volume, total buy qty, total sell qty, open, high, low, close (all int32), then in full mode last trade time, OI, OI day high, OI day low, exchange timestamp, then depth at bytes 64-184: 10 entries of 12 bytes (5 bids then 5 offers), each `quantity i32, price i32, orders i16, 2 bytes padding`. Verify exact offsets against the live docs page when implementing and encode them as constants next to a test encoder; a mismatch is caught by the round-trip test plus a recorded real frame.
- Prices are integers in paise: divide by 100, except currency instruments, which the docs say use a divisor of 10,000,000 (four decimals). Pick the divisor from the instrument master's segment; unit-test one case per divisor. Convert integer to `Decimal` with `Decimal::new(raw, scale)` (no float).
- Unknown packet length: skip, count `unknown_packet_len`, log once per distinct length.
- Docs give no reconnection protocol; the client implements it (SKILL.md section 5). Any connect failure with an auth cause means re-login, not retry.

## 6. Rate limits

The errors page (<https://kite.trade/docs/connect/v3/exceptions/>) documents separate rates for quote, historical and all other endpoints, plus order-specific caps that matter only to `bz-execution`. Third-party forum posts quote different per-minute and per-day order figures, so those are unverified: confirm on the page. Read the numbers once, put them in `config/data.toml` `[kite.rate_limits]` with `verified_on`, and remember that broker rules under the SEBI retail algo framework can be stricter than the page (`bz-security` `references/sebi-algo-compliance.md`). The docs do not specify 429 backoff: on `429` wait using a capped exponential delay with jitter, lower the bucket rate for a while, and surface `RateLimited`. Order limits matter only to `bz-execution`.

Implementation: one `governor` (or hand-rolled token bucket) per endpoint class, created from `data.toml`; `candles()` awaits the bucket before each request; a small concurrency limit for historical to stay clear of bursts.

## 7. Error mapping

| Signal | Mapping |
|---|---|
| HTTP 403 / `error_type: TokenException` | `SessionExpired` (clear token, no retry) |
| 400 / `InputException` | `BadRequest` (no retry) |
| 429 | `RateLimited` (retry later with backoff) |
| 500, 502, 503, 504, network errors, `NetworkException` | `Unavailable` (bounded retries with jitter) |
| 200 with unexpected JSON | `Decode { path }` via `serde_path_to_error` |
| Plan lacks market data | `NotEntitled` (message from the API mapped to a stable category) |

Error bodies are JSON with `status`, `message`, `error_type`; log `error_type` and HTTP status, not the whole body when it might include account data.

## 8. Fixtures to keep (tree and tolerances owned by `bz-testing`)

Under `fixtures/kite/rest/`: `instruments_sample.csv.gz` (a few NSE equities, an index, NFO options/futures with expiries, one currency contract), `historical_day.json`, `historical_minute_oi.json`, `quote_ltp.json`, `error_token_exception.json`, `error_429.json`. Under `fixtures/kite/ws/`: `*.hex` frames per packet length. Hand-craft them or have the human owner record and scrub them; the agent never calls the live API to record.
