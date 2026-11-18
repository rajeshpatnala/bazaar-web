# 9. Scheduled jobs and config refresh

(From `bz-architecture` SKILL.md; open when the task touches this area.)


Nothing market-specific is compiled in (`bz-india-markets`). Recurring work (instrument refresh, EOD bhavcopy import, F&O ban-list and holiday-calendar refresh, scans, approval expiry sweep) runs **inside `bazaar-server`** under a small scheduler driven by the injected `Clock` and `MarketCalendar`, never by ad hoc sleep loops or the system clock. Because the Mac may be asleep or off at the scheduled time, every job is **catch-up capable**: due-ness is computed from "what is missing for which date", not from "did the timer fire". Catch-up runs at server start and on wake. The same use cases are `bazaar-cli` subcommands for manual human runs. Full job table, `job_runs` schema, config-staleness table: `references/scheduled-jobs-and-design-deliverables.md`.

- **Idempotency**: every run first records `(job, key)` in a unique `job_runs` row (key is the trading date, or the bar close for scans); a restart or double tick cannot run the same unit twice, and job bodies upsert by natural key so a retry after a partial run is safe.
- **Missed-run policy** (Mac asleep, server not running, or the clock jumped): instrument refresh catches up once if today's is missing and the session has not ended, else F&O stays blocked; ban list for today missing on a trading day means every F&O symbol is treated as blocked until fetched; EOD import backfills missing trading dates oldest first within a configured window, then asks the owner; the EOD scan runs once for the latest date only; intraday scans are never caught up, they are recorded `missed` and the data shows stale. A `running` row left by a crash is marked `abandoned` at startup.
- Data stays marked stale (`data_status`) until the catch-up that covers it succeeds; the UI shows what is stale and why.
- Jobs never place orders or touch approvals, the kill switch or `risk.toml`; one failing job is recorded and surfaced on `data_status` without stopping the others.
- Tests drive the scheduler with `FakeClock` (cross the trigger, jump the clock forward across a whole session to simulate wake, restart mid-run, holiday, missed window).

Every config file carries `schema_version`, `verified_on` and `source`; loaders validate schema and sanity ranges and fail closed. `just db-check` (runs `bazaar-cli check-connections`) is the only way agents check `sqlite`, `duckdb`, `kite-session`, `instruments` and `audit-chain`. Secrets reach the program only through `SecretStore` (Keychain, or env vars from an untracked `.env` the bin loads); agents never open `.env`, and no design may require them to.
