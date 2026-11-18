# API contract: routes, DTOs, errors, WebSocket

Single source of truth for the HTTP and WebSocket surface (CONTRACTS.md section 4 summarises it). The UI consumes the generated `web/src/api/schema.d.ts`; names below are the Rust DTO names and become `components["schemas"][...]`. Field lists of `RecommendationDto` live only in `bz-quant`; gate rule ids only in `bz-trading-discipline`; auth, CSRF and header mechanics in `bz-security`.

**Adding or changing a route = one PR containing:** (1) a row here, (2) the handler with `#[utoipa::path]` registered via `routes!`, (3) `just openapi` and the regenerated `schema.d.ts`, (4) the MSW handler in `web/src/mocks/handlers.ts` (`bz-testing`), (5) a server test (happy path, 401, 403 CSRF if non-GET, one error code). A new mutation also needs a `risk-guardian` look if it touches orders, approvals, kill switch or mode.

## 1. Conventions

| Topic | Rule |
|---|---|
| Base | JSON under `/api/v1`, same origin, cookie `bz_session`; non-GET sends `X-CSRF-Token` (value from `GET /auth/me`) |
| Names | JSON keys and enum values `snake_case`; ids are UUID strings; paths plural nouns |
| Symbols | query/body field `symbol` (and `underlying`) is the instrument key defined by the instrument master (`bz-market-data`); treated as an opaque string, max 64 chars, looked up, never interpolated |
| Money | decimal **string** (`"1234.50"`): prices, P&L, charges, cash, `max_loss_inr`, percent-of-capital. Quantities, lots, counts are integers |
| Analytics | indicators, IV, Greeks, PCR, ratios are JSON numbers; `null` during warm-up or when undefined (never NaN) |
| Time | instants RFC 3339 UTC (`2026-11-18T03:45:00Z`); dates `YYYY-MM-DD` (IST trading date); candle `time` = bar open, epoch seconds UTC. No IST strings on the wire |
| Lists | small fixed lists return `{ items }`; unbounded lists use `?limit` (server max) and `?cursor`, response `{ items, next_cursor }` (null at the end) |
| Staleness | every market-derived DTO carries `data_as_of`; the delayed threshold comes from `StatusDto.stale_after_secs`, not a UI constant |
| Caching | `Cache-Control: no-store` on every `/api/*` response |
| Unknown fields | request bodies reject unknown fields (422 `validation_error`) |
| Idempotency | writes are safe against double click by state, not by header: repeat approve/reject is 409; `PUT /kill-switch` is idempotent |

## 2. Route table

Auth column: **P** public, **S** session (non-GET additionally needs Origin + CSRF). "Err" lists problem `code`s beyond the universal ones in section 3.

### Entry, session, health

| Method + path | Auth | Request | Response | OK | Err |
|---|---|---|---|---|---|
| `GET /healthz` | P | none | `{ "status": "ok" }` | 200 | none (no data, no auth) |
| `GET /auth?token=` | P | launch token (query; single use) | none; sets `bz_session`; `Location: /` | 303 | `launch_token_invalid` (403) |
| `GET /api/v1/auth/me` | S | none | `AuthMeDto { csrf_token, mode, live_enabled }` | 200 | 401 |
| `GET /api/v1/status` | S | none | `StatusDto` (fields under Settings below) | 200 | none |
| `GET /api/v1/kite/login` | S | none | redirect to the Kite login URL (nonce stored; token never in URL) | 303 | `kite_not_configured` (409) |
| `GET /api/v1/kite/callback` | P (nonce) | Kite query params | redirect to `/settings?kite=connected` or `?kite=failed` (no token text) | 303 | bad/used/expired nonce redirects to `?kite=failed` |

No login form, password, TOTP, logout or refresh route exists: the launch link is single use, so a lost session means restarting the server (`just run`), which prints a fresh link. Paths in the generated document include `/api/v1` (`OpenApiRouter::nest`); the UI calls full paths with `baseUrl: location.origin`.

### Market and charts (read-only)

| Method + path | Request (query) | Response | Err |
|---|---|---|---|
| `GET /market/overview` | none | `MarketOverviewDto { indices[], vix, breadth, sectors[], movers { gainers[], losers[] }, regime: RegimeDto, session: SessionDto, data_as_of }` | none |
| `GET /instruments` | `q`, `segment?`, `limit` (<=50) | `{ items: InstrumentDto[] }` (`symbol`, `name`, `segment`, `lot_size?`, `tick_size`, `expiry?`) | none |
| `GET /watchlists` | none | `{ items: WatchlistDto[] }` (from `watchlists.toml`) | none |
| `GET /candles` | `symbol`, `timeframe`, `from?`, `to?`, `limit` (<=5000, default 500) | `CandlesDto { symbol, timeframe, adjusted, data_as_of, candles: CandleDto[] }`; `CandleDto { time, open, high, low, close, volume, open_interest? }` | `instrument_not_found` (404), `data_unavailable` (503) |
| `GET /indicators` | `symbol`, `timeframe`, `from?`, `to?`, repeated `indicator=ema:20` (max 8) | `IndicatorsDto { symbol, timeframe, series: [{ name, points: [{ time, value|null }] }] }` | `unknown_indicator` (422) |
| `GET /option-chain` | `underlying`, `expiry?` (date; default nearest) | `OptionChainDto { underlying, spot, expiry, expiries[], lot_size, atm_strike, pcr, max_pain, iv_rank, iv_percentile, rows: [{ strike, ce: OptionLegDto?, pe: OptionLegDto? }], data_as_of }`; `OptionLegDto { symbol, ltp, bid, ask, oi, oi_change, volume, iv, delta, gamma, theta, vega }` | `instrument_not_found`, `expiry_not_found` (404), `data_unavailable` |
| `POST /option-payoff` | `PayoffRequest { legs: [{ symbol, side, lots, entry_price? }], spot_min?, spot_max? }` | `PayoffDto { points: [{ spot, pnl }], max_profit?, max_loss?, breakevens[], pop?, margin_required?, margin_status: "broker"|"unavailable" }` | `validation_error` |

`POST /option-payoff` computes only (no state change); it is POST for the body, so CSRF applies. Margin comes from the `MarginSource` port; when unavailable the field is `null` and `margin_status` says so (never an estimate).

### Recommendations and orders

| Method + path | Request | Response | OK | Err |
|---|---|---|---|---|
| `GET /recommendations` | `horizon?`, `limit` (<=100) | `RecommendationListDto { items: RecommendationDto[], verdict: "ok"|"no_high_quality_setups", regime: RegimeDto, reasons[], generated_at, data_as_of }` (empty `items` is a normal result) | 200 | none |
| `GET /recommendations/{id}` | none | `RecommendationDto` | 200 | `recommendation_not_found` (404) |
| `POST /order-intents` | `CreateOrderIntentRequest { recommendation_id }` | `OrderIntentDto { intent_id, state, approval_id?, gate: GateDto, expires_at? }`; `state` is `awaiting_approval` or `blocked` (a blocked intent is audited and has no approval); `Location: /api/v1/approvals/{id}` | 201 | `recommendation_not_found` (404), `recommendation_stale` (409), `intent_already_pending` (409, body has `approval_id`), `kill_switch_engaged` (409), `quote_unavailable` (503) |
| `GET /approvals` | `state?` (default `awaiting_approval`) | `{ items: ApprovalDto[] }` (runs `expire_stale` first) | 200 | none |
| `GET /approvals/{id}` | none | `ApprovalDto` | 200 | `approval_not_found` (404) |
| `POST /approvals/{id}/approve` | `ApproveRequest { override?: OverrideDto { rule_ids[], reason, text } }` | `ApprovalDto` (state `approved`/`submitting`/...; fills arrive as `order_update`) | 200 | below |
| `POST /approvals/{id}/reject` | `RejectRequest { reason? }` | `ApprovalDto` (`declined`) | 200 | `approval_already_decided`, `approval_expired` (409) |
| `PUT /kill-switch` | `KillSwitchRequest { engaged, reason?, confirmation? }` | `KillSwitchDto { engaged, since?, reason?, by? }` | 200 | `confirmation_required` (422 when releasing without the typed phrase) |

Approve outcomes: success 200; unknown id 404 `approval_not_found`; repeat or already approved/declined 409 `approval_already_decided` (extension member `approval` holds the existing outcome, no second order); past TTL 409 `approval_expired`; price moved beyond `max_drift_pct` 409 `quote_drifted` (extension `approval` holds the refreshed risk, nothing was sent, the user must click again); gate re-run fails 422 `gate_blocked` (extension `gate: GateDto`); bad or disallowed override 422 `override_invalid` (reason shorter than `min_reason_chars`, rule not overridable, daily/weekly cap used); kill switch engaged 409 `kill_switch_engaged`; no fresh quote 503 `quote_unavailable`; broker session gone 503 `broker_session_expired`; executor rate limited 429 `rate_limited` with `Retry-After`. The handler calls only `ApprovalQueue::approve`; the pipeline owns the gate re-run, guards and the executor.

`ApprovalDto` carries everything the approval dialog needs and nothing derived in the UI: `id`, `intent_id`, `state` (`OrderStateDto`: the state-machine names of `bz-execution` in snake_case, for example `awaiting_approval`, `approved`, `working`, `filled`, `declined`, `expired`, `blocked`, `rejected`, `unknown`), `mode`, `symbol`, `side`, `quantity`, `lots?`, `order_type`, `price?`, `trigger_price?`, `product`, `stop_loss?`, `target?`, `estimated_charges`, `max_loss_inr`, `max_loss_pct_of_capital`, `gate: GateDto`, `quote_as_of`, `created_at`, `expires_at`, `decided_at?`, `override?`. `GateDto { passed, evaluated_at, rules: [{ rule_id, label, status: "pass"|"fail"|"overridable_fail", detail }] }` is a view of `GateVerdict`; rule ids are the G-table ids.

### Account, positions, orders (read-only)

| Method + path | Request | Response | Err |
|---|---|---|---|
| `GET /account` | none | `AccountDto { kind: "paper"|"live", epoch, starting_cash, cash_available, blocked, equity, realised_pnl, unrealised_pnl, charges_total, day_pnl, as_of }` | none |
| `GET /positions` | none | `{ items: PositionDto[] }` (`symbol`, `product`, `quantity` signed, `avg_price`, `ltp`, `unrealised_pnl`, `realised_pnl`, `charges`, `stop_loss?`, `target?`, `opened_at`) | none |
| `GET /orders` | `state?` (`open`|`all`), `limit`, `cursor` | `{ items: OrderDto[], next_cursor }` | none |

All P&L is net of charges (the ledger does the arithmetic; the server only formats).

### Journal

| Method + path | Request | Response | Err |
|---|---|---|---|
| `GET /journal/trades` | `from?`, `to?`, `strategy?`, `limit`, `cursor` | `{ items: JournalTradeDto[], next_cursor }` (`id`, `symbol`, `strategy`, `side`, `quantity`, entry/exit price and time, `r_multiple`, `pnl_net`, `charges`, `mistake_tags[]`, `note`, `recommendation_id?`) | none |
| `GET /journal/summary` | `from?`, `to?` | `JournalSummaryDto { by_strategy: [{ strategy, trades, win_rate, profit_factor, avg_r, pnl_net }], r_distribution: [{ bucket, count }], mistakes: [{ tag, count }] }` | none |
| `PATCH /journal/trades/{id}` | `JournalNotePatch { mistake_tags?, note? }` (tags from the `MistakeTag` enum, note <= 2000 chars) | `JournalTradeDto` | `journal_trade_not_found` (404) |

The only writable journal fields are `mistake_tags` and `note`; prices, sizes and P&L are immutable here.

### Backtests

| Method + path | Request | Response | OK | Err |
|---|---|---|---|---|
| `GET /strategies` | none | `{ items: StrategyInfoDto[] }` (`id`, `title`, `enabled`, `params` defaults and ranges from the params `validate()`) | 200 | none |
| `POST /backtests` | `StartBacktestRequest { strategy_id, from, to, params? }` | `BacktestJobDto { id, status, progress_pct, created_at, started_at?, finished_at?, error_code? }`; `Location: /api/v1/backtests/{id}` | 202 | `strategy_not_found` (404), `validation_error` (422 incl. bad params/date range), `backtest_busy` (409), `no_data_in_range` (422) |
| `GET /backtests` | `limit`, `cursor` | `{ items: BacktestSummaryDto[], next_cursor }` | 200 | none |
| `GET /backtests/{id}` | none | `BacktestJobDto` plus `result?: BacktestResultDto` once `succeeded` | 200 | `backtest_not_found` (404) |
| `GET /backtests/{id}/trades` | `limit`, `cursor` | `{ items: BacktestTradeDto[], next_cursor }` | 200 | `backtest_not_found` |
| `POST /backtests/{id}/cancel` | none | `BacktestJobDto` (idempotent; terminal jobs unchanged) | 200 | `backtest_not_found` |

`status`: `queued | running | succeeded | failed | cancelled` (terminal states never change). `BacktestResultDto`: `metrics` (trades, win_rate, profit_factor, avg_r, max_drawdown, net_pnl, charges_total, period, costs_applied flag), `walk_forward[]`, `equity_curve: [{ time, equity }]` (downsampled to <= 2000 points), `low_evidence`. Metric definitions come from `bz-backtest-risk`.

### Settings (read-only)

| Method + path | Response |
|---|---|
| `GET /settings` | `SettingsDto { mode, limits, hard_limits, override_policy, execution, alerts: { browser, telegram_configured }, kite: { configured, session_valid_for_today }, config_sources: [{ file, verified_on, source }] }` |

There is no `PUT`/`PATCH`/`POST` under `/settings`, no route that changes `execution_mode` or any `risk.toml` value, and no secret in the DTO (booleans like `telegram_configured` only). Mode and limits change by editing `config/` and restarting.

`StatusDto`: `server_time`, `version`, `mode`, `live_enabled`, `kill_switch: KillSwitchDto`, `market: { session: "pre_open"|"open"|"closed"|"holiday", next_change_at? }` (from `MarketCalendar`), `feed: { state: "connected"|"reconnecting"|"down", last_tick_at? }`, `kite: { connected, session_valid_for_today }`, `data: { eod_as_of?, instruments_as_of?, stale_after_secs }`, `audit_chain_ok`. No tokens, paths or account ids.

## 3. Problem details (`application/problem+json`)

```json
{ "type": "urn:bazaar:problem:gate_blocked", "title": "Order blocked by risk rules", "status": 422,
  "detail": "Daily loss limit reached.", "code": "gate_blocked", "errors": [], "gate": { } }
```

`code` is the stable machine key (UI switches on it, never on `detail`); `title` per code is a fixed string; `detail` is user-safe and never contains paths, SQL, upstream bodies, tokens or `Debug` output. `errors: [{ field, message }]` appears on 422 validation. Extension members (`gate`, `approval`, `approval_id`) are documented in the OpenAPI `Problem` schema variants. Codes are never renamed without bumping the API version.

| Status | Codes |
|---|---|
| 400 | `bad_request` (malformed JSON or query) |
| 401 | `unauthenticated` |
| 403 | `launch_token_invalid`, `forbidden_host`, `forbidden_origin`, `csrf_invalid` |
| 404 | `not_found` (unknown API path), `instrument_not_found`, `expiry_not_found`, `recommendation_not_found`, `approval_not_found`, `backtest_not_found`, `strategy_not_found`, `journal_trade_not_found` |
| 405, 408, 413, 415 | `method_not_allowed`, `request_timeout`, `payload_too_large`, `unsupported_media_type` |
| 409 | `approval_already_decided`, `approval_expired`, `quote_drifted`, `kill_switch_engaged`, `intent_already_pending`, `recommendation_stale`, `backtest_busy`, `kite_not_configured` |
| 422 | `validation_error`, `gate_blocked`, `override_invalid`, `confirmation_required`, `unknown_indicator`, `no_data_in_range` |
| 429 | `rate_limited`, `too_many_connections` (WebSocket cap) |
| 500 | `internal` (generic detail, logged once server-side) |
| 503 | `data_unavailable`, `quote_unavailable`, `broker_session_expired` |

Mapping lives in `error.rs` as explicit `From<DomainError>` matches (no catch-all arm). Kite and other upstream error text is classified, never forwarded.

## 4. WebSocket `/api/v1/ws`

Upgrade is a GET that passes Host allowlist, Origin check (exact server origin; debug builds also the Vite origin) and the session; otherwise the browser sees a failed handshake (it cannot read the 401/403), so the client calls `GET /auth/me` to tell "no session" from "server down". Max 4 concurrent sockets, 4 KiB max frame, text JSON only. Envelope: `{ "type": "<t>", "data": { ... } }`.

There is no auth close code: on any abnormal close the client calls `GET /api/v1/auth/me`; 401 shows the session-ended screen, anything else reconnects with backoff. Envelope types `WsOut` (server to client) and `WsIn` (client to server) and every `data` type are `ToSchema` types in `dto/ws.rs`, registered on `ApiDoc` with `components(schemas(...))`, so the UI uses `components["schemas"]["WsOut"]`.

**Client to server** (everything else closes the socket with 1008):

| type | data | effect |
|---|---|---|
| `subscribe` | `{ symbol }` | adds the symbol (max 200 per socket; unknown symbols are ignored); the latest cached tick is sent immediately |
| `unsubscribe` | `{ symbol }` | removes it |

**Server to client:**

| type | data | delivery |
|---|---|---|
| `tick` | `{ symbol, ltp, change_pct, volume?, ts }` (strings for ltp/change_pct) | conflated: latest per symbol per ~250 ms window, only for subscribed symbols |
| `data_status` | `{ state: "live"|"delayed"|"down", feed, last_tick_at?, stale_after_secs, reason? }` | first frame after upgrade and on every change |
| `order_update` | `{ intent_id, approval_id?, state: OrderStateDto, mode, symbol, side, quantity, filled_quantity, avg_price?, reason?, at }` | never dropped by design: on lag the client gets `resync` and refetches `/orders`, `/approvals`, `/positions` |
| `gate_alert` | `{ kind, severity: "info"|"warn"|"critical", rule_ids[], message, approval_id?, at }` (kill switch engaged/released, daily-loss breaker, a pending approval now failing its gate) | same as above |
| `job_progress` | `{ job_id, kind: "backtest", status, progress_pct, message? }` | throttled to ~4/s per job; the REST status is authoritative |
| `resync` | `{ reason: "lagged"|"wake"|"server_restart", groups: ["status","approvals","orders","positions","recommendations","backtests"] }` | sent when this session missed messages or the server detects a wake from sleep; the UI invalidates those query groups |

