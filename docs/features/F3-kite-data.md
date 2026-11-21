# F3 - Real market data (Kite read-only, NSE archives)

Status: building
Summary: owner-run Kite login, instruments, candles, bhavcopy and F&O ban list, REST quotes and margins on real data; no orders.
Owner area: `crates/data`, `config/data.toml`, `core/src/{config/data,secret,ports}.rs`, `server/routes/kite.rs`, store ban-list table.

<!-- Plan: .agents/plans/F3-kite-data.md (tickets BZ-404..413; each ticket appends its section). -->

## Acceptance criteria
1. Fake Kite server: every call has a timeout, respects the rate limit, retries only idempotent GETs with backoff; no token in logs, errors, Debug or DTOs.
2. Callback with a wrong, missing or reused `state` stores nothing; a valid one stores the token (0600); the UI shows "connected until ~06:00 IST".
3. Expired or absent token: every adapter returns `Unavailable`; `/status` says "login required".
4. Resumed fetch: no candle duplicated or skipped, gaps reported, bars stored through `CandleSink`.
5. Missing, undated or stale NSE ban list is reported unchanged (scanner fails closed).
6. Hosts, URLs, rate limits, timeouts, staleness and chunk sizes come from `data.toml`; unlisted hosts are refused, cross-host redirects not followed.
7. `data` depends only on `core`; `cargo test` needs no network.

## What was built
- BZ-404 (contracts): `core::config::DataConfig` (typed `[kite]`, `[nse]`, `[staleness]`, `[history]`; loader rejects missing `verified_on`, non-https URLs, hosts off the allowlist, userinfo/port in a host, zero timeouts/limits/spans, oversize caps). `core::secret::{Secret, AccessToken}` (redacting Debug, no Display/Serialize/Clone). `core::ports::{TokenStore, TokenError}`.
- Config keys: `data.toml` `kite.*`, `kite.span_days.*`, `nse.*`, `staleness.*`, `history.*`, `unverified`. `config/data.toml` is no longer a placeholder.

## Decisions
- `data.toml` `verified_on` means "checked against the provider docs": while `unverified` is non-empty it is only the authoring date, the file is reported UNVERIFIED (never fresh) by `stale_warnings` and `check-connections` until the owner check (BZ-490/BZ-491) clears the list.
- Token storage: file under `secrets/`, 0600, atomic write, never read by agents: `docs/adr/0002-kite-token-storage.md`.
- Read-only client, REST polling for M1, host allowlist from config.

## Known limits
- Every value in `config/data.toml` was written from memory without network access; all are listed in `unverified` and need the owner's check against Kite and NSE docs before the first real fetch. The ban list URL is the least certain.
- The staleness `ban_list_max_age_hours` is not yet cross-checked against `risk.toml` `hard_limits.max_ban_list_age_hours` (add where the scanner consumes it, BZ-409/519).

## Deferred
- BZ-411 (WebSocket ticks, optional).

## History
- 2026-11-21: BZ-404 contracts.
