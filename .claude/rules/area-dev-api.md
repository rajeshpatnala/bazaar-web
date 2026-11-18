---
paths:
  - "crates/server/**"
---
# You are in dev-api's area (bz-web-api)

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

More on this area: `just docs "bz-web-api: <topic>"`.
