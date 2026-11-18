# Live phase: broker crate and Kite order mechanics

Open this before writing anything in `crates/broker` or enabling `semi_auto` against a real broker. Facts below were looked up on 2026-11-18; they change. Recheck the linked pages and record the date in `DECISIONS.md` (rate numbers: in `config/data.toml` with `verified_on` + `source`). Never claim they are current without re-checking. The SEBI/NSE retail algo framework and the pre-live checklist are not restated here: see `bz-security` `references/sebi-algo-compliance.md` (checklist state in `config/compliance.toml`).

## 1. Scope and guardrails

- Live code exists only behind cargo feature `live-trading` (off by default, never in `default`) and is still gated at runtime (section 6). Default mode is `paper`; `semi_auto` is the first live mode: the user approves every order with a click.
- Tests use `wiremock` with recorded fixtures in `fixtures/kite/`. No test, agent or CI job calls the real API or reads `.env`. Whether the session is healthy is answered only by `just db-check`.
- Credentials (API key/secret, daily access token) are loaded by the program's secret loader, never logged, never in audit payloads (`bz-security`).

## 2. Kite Connect order API (summary in my own words)

Source: https://kite.trade/docs/connect/v3/orders/ and https://kite.trade/docs/connect/v3/exceptions/

- Place: `POST /orders/:variety` returns an `order_id`. Success only means the order reached the OMS, not that it executed. Always follow with status tracking.
- Varieties: `regular`, `amo`, `co`, `iceberg`, `auction`. Order types: `MARKET`, `LIMIT`, `SL`, `SL-M`. Products: `CNC`, `NRML`, `MIS`, `MTF`. Validity: `DAY`, `IOC`, `TTL` (minutes).
- `tag`: optional, alphanumeric, at most 20 characters, echoed on order responses. This is our idempotency key.
- `market_protection`: for `MARKET` and `SL-M`; a percentage above 0 up to 100, or `-1` for automatic. Always set it for market-type orders; exchange price-protection ranges can still reject.
- `autoslice` splits orders above the freeze quantity into slices and returns an array where failed slices appear as error objects. Off by default for us; sizing in `bz-backtest-risk` should keep quantity under the freeze limit (from instrument master/config).
- Statuses: final `OPEN`, `COMPLETE`, `CANCELLED`, `REJECTED`; interim values such as `PUT ORDER REQ RECEIVED`, `VALIDATION PENDING`, `OPEN PENDING`, `MODIFY PENDING`, `TRIGGER PENDING`, `CANCEL PENDING`. Map unknown strings to "still working", never to filled. `status_message` explains rejections; a null `exchange_order_id` means it never reached the exchange.
- Reading state: `GET /orders` (day's book), `GET /orders/:id` (history of one order), `GET /trades`, `GET /orders/:id/trades`. Postbacks (webhook) exist for updates on long-lived orders; the server could receive them only on a public HTTPS endpoint, which adds attack surface, so prefer order-update WebSocket messages if the ticker provides them and otherwise poll the order book at a modest, rate-limited interval.
- The `guid` returned by the API is documented as unusable for duplicate prevention. There is no server-side idempotency key: duplicates are avoided only by our tag and by checking the order book before any retry.
- Margin and charges endpoints, the adapters behind the `MarginSource` port, also useful for sanity checks: `POST /margins/orders`, `POST /margins/basket`, `POST /charges/orders` (https://kite.trade/docs/connect/v3/margins/).
- Errors: JSON `error_type` values include `TokenException` (session expired, HTTP 403), `InputException`, `OrderException`, `MarginException` (insufficient funds), `HoldingException`, `NetworkException` (could not reach the OMS), `DataException`, `GeneralException`; HTTP 429 is rate limiting. Map each to an `ExecError` variant; `NetworkException` and timeouts map to `Unknown`.
- Documented rate limits: order placement 10 requests/sec, 400 orders/min, 5,000 orders/day per API key, at most 25 modifications per order. Forum reports say the 10/sec limit became a hard limit from 1 April 2026 and that extra API keys do not add capacity; one Zerodha article mentioned a 3,000/day internal rule in 2025, so sources conflict; treat all numbers as unverified until confirmed on the Kite docs/support pages. Put the numbers in `config/data.toml` with `verified_on`, and keep our own caps well below them (e.g. low single digits per second, tens per day for a 1,00,000 INR account).

## 3. SEBI / NSE rules and the pre-live checklist

Not restated here. The framework summary, sources and the checklist are only in `bz-security` `references/sebi-algo-compliance.md`; checklist state is `config/compliance.toml`. Mechanical consequences for this crate:

- Order calls (place, modify, cancel) fail when the egress IP is not the whitelisted one (live phase: the static IP the owner gets from the ISP for the Mac's connection; a changed address means re-registering it): detect that rejection class, engage the kill switch and tell the user in plain words. Reads work from any IP.
- `TokenException` means the daily session ended: block sends, prompt re-login; no unattended overnight sessions, no "perpetual refresh".
- Whether `auto` is allowed for the account is a broker decision recorded in the checklist, not assumed here.

## 4. Placing an order (LiveExecutor::place)

1. Build tag: `"BZ" + 18 chars` derived deterministically from the intent id (base32, no punctuation, <= 20 total). Unique index on `order_intents.tag`. Same intent always yields the same tag.
2. Transition `Approved -> Submitting{tag}` and commit (with audit) **before** the HTTP call.
3. Call the order endpoint with a short timeout (no automatic retry inside `place`).
4. On a returned `order_id`: `Working{broker_id}`. On `MarginException`/`InputException`/`OrderException`: `Rejected`. On 429: `Denied(RateLimited)`, nothing was sent, safe to resubmit manually later. On `TokenException`: engage kill switch, ask for re-login. On timeout/`NetworkException`/unparseable reply: `Unknown`.

## 5. Reconciliation

`reconcile()` runs at startup, after login, after every `Unknown`, and every N minutes while orders are working:

1. Fetch `GET /orders` and `GET /trades` and positions.
2. For each local order in `Submitting`/`Unknown`/`Working`: find the broker order by `tag` (fall back to `order_id`). Found: adopt its status and fills. Not found after the order book is known to be complete for the day: mark `Rejected("not found at broker")`; only a **user-approved new intent** may re-send, never the system.
3. For broker orders with our tag prefix but no local record: create an `Orphan` audit entry and engage the kill switch.
4. Compare net positions broker vs local (ledger of live fills). A mismatch engages the kill switch, writes an audit entry with both views, and blocks new entries until the user acknowledges.
5. Reconciliation is read-only toward the broker; it never places or cancels on its own (except cancel of a confirmed duplicate if the user opted in).

## 6. Runtime lock beyond the feature flag

`LiveExecutor::new(LiveEnabled, Session, ComplianceOk)` only compiles with `live-trading`; `LiveEnabled` is minted from the Settings toggle (audited). Wiring in `crates/server` picks `PaperExecutor` unless `mode != paper` **and** all three tokens exist. UI shows a persistent "LIVE" badge; paper shows "PAPER".

## 7. Tests for this crate

`wiremock` fixtures: success, each exception class, 429 with `Retry-After`-style headers if present, timeout followed by order book containing the tag (adopt) and not containing it (Rejected, no resend), duplicate click (one HTTP call), 403 token expiry. CI: `cargo clippy -p bazaar-broker --features live-trading --all-targets -D warnings` and `cargo nextest` for that crate with the feature, all against fakes.
