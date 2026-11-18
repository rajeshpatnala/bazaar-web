---
paths:
  - "crates/data/**"
  - "crates/store/**"
---
# You are in dev-data's area (bz-market-data)

1. **Legal data only.** Official broker APIs, licensed vendors, official exchange archives, or user-supplied CSV. No scraping HTML pages, no browser impersonation, no cookie tricks, no IP rotation. If an official archive refuses a request, stop and surface "import this file manually"; do not work around it.
2. **No hardcoded market facts.** Lot size, tick size, expiry dates, F&O eligibility, ban list, holidays, price bands and WebSocket limits come from the instrument master, `config/*.toml` (with `verified_on`) or the current vendor docs. A literal like `75`, `0.05` or `Thursday` in a data crate is a bug.
3. **Vendor limits are looked up, not remembered.** Rate limits, subscription caps, URL templates, per-interval history windows and refresh windows live only in `config/data.toml` (CONTRACTS section 1; this skill owns the schema, section 3a), each table with `verified_on` and `source`. No such number appears in skill text, code or tests outside the fixture config. When you cannot verify one, write "check the current docs at <url>" and leave the key out so the loader fails closed; never invent a number.
4. **Raw is sacred.** Store unadjusted prices exactly as received; apply corporate-action factors at read time. Re-running an import with the same file must yield identical rows (idempotent upsert on the natural key).
5. **Decimal at the boundary.** Convert wire integers (paise) and CSV strings to `Decimal` in the parser. Compute volumes and OI as `u64`/`i64`. Indicators may use `f64` downstream (`bz-quant`), not this layer.
6. **Every external value is validated.** Reject rows with non-positive prices, `high < low`, close outside `[low, high]`, negative volume, unknown instrument; count and report rejects (`CleaningReport`), abort above a configured reject ratio.
7. **Never panic on bytes from the network.** Binary packet parsing is bounds-checked and returns `Result`; unknown lengths are skipped with a counter, not a crash.
8. **Secrets stay out of the data layer's output.** `api_key`, `api_secret`, `request_token`, `access_token` are held as `secrecy::SecretString` (redacting `Debug`; exposed only at the point of use), are never in log fields, error text, URLs that get logged, or fixtures. Agents never open `.env`; the program loads secrets through the `SecretStore` port (`bz-core`, `bz-security`).
9. **Stale data is a visible state.** Every live stream and every cached dataset exposes `as_of`; consumers get `Freshness::{Live, Delayed, Stale}` and the UI shows a badge (`bz-web-ui`; pushed as the `data_status` WebSocket message, `GET /api/v1/status`).
10. **Tests never touch the live API.** Fixtures + wiremock + local fake WebSocket only. Connection status for agents is `just db-check` and nothing else.

More on this area: `just docs "bz-market-data: <topic>"`.
