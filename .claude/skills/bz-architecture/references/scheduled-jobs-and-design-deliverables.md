# Scheduled jobs, config refresh and design deliverables (optional depth)

Open this when adding or changing a scheduled job, a config refresh, wake/catch-up behaviour, or when asked for a feature design document. Overview and rules of thumb are in `SKILL.md` sections 8, 9 and 12.

## 1. Scheduler design

Recurring work runs inside `bazaar-server` under a small scheduler. It reads the injected `Clock` and `MarketCalendar`: a deterministic loop ticks every minute and checks which jobs are due. An off-the-shelf cron crate that reads the system clock cannot be driven by `FakeClock`; adopting one needs an ADR that shows how tests control time. The same use cases are exposed as `bazaar-cli` subcommands (`refresh-instruments`, `import-bhavcopy`) for manual human-run invocations.

The server lives on a laptop, so "the timer fired" is not a reliable trigger. Compute due-ness from state: for each job and each relevant trading date, is there a successful `job_runs` row, and is its window open or past? The loop runs this check:

- at server start (after startup recovery, before accepting intents),
- on every tick,
- immediately on a **wake event**: the loop remembers the previous tick's `Clock` time, and a gap larger than the configured threshold (a few minutes; tokio timers may pause during sleep, so only the wall clock tells) means the Mac slept. Wake also triggers the feed reconnect and stale marking described in `SKILL.md` section 8.

| Job | When (IST, trading days via `MarketCalendar`) | Idempotency key | Missed-run policy (start or wake) |
|---|---|---|---|
| Instrument refresh | before pre-open | `(instrument_refresh, trading_date)` | catch up once if today's is missing and the session has not ended; otherwise F&O stays blocked until done |
| F&O ban list refresh | before pre-open (and again at start or wake during the session) | `(ban_list, trading_date)` | fetch if today's is missing; unknown on a trading day means every F&O symbol is gate-blocked |
| EOD bhavcopy and corporate-action import | after the source publishes | `(eod_import, trading_date)` | backfill every missing trading date within a configured window, oldest first; beyond it, report and ask the owner |
| EOD universe scan | after a successful EOD import | `(eod_scan, trading_date)` | run once for the latest date; never for past dates |
| Holiday and calendar check | on start, daily | `(calendar_check, trading_date)` | warn when `calendar.toml` lacks the current or next year, or `verified_on` is older than the configured age; never rewrites the file |
| Intraday scans | on bar close during the session | `(scan, instrument_set, bar_close)` | no catch-up: skip, record `missed`, mark data stale |
| Approval expiry sweep, data-freshness check | every minute and on wake | n/a (pure state transitions) | n/a |

Rules:
- Every run writes a row to `job_runs` (`job`, `key`, `scheduled_for`, `started_at`, `finished_at`, `status`, `error`) through a `core::ports` repo before work starts. A unique `(job, key)` plus single-process execution means a restart, wake or double tick cannot run the same unit twice. A `running` row left by a crash is marked `abandoned` at startup and handled by the missed-run policy.
- Job bodies are idempotent on their own (upsert by natural key such as instrument token or `(symbol, date)`), so a retry after a partial run is safe.
- Data touched by a missed or failed job stays marked stale on `data_status` (with the reason) until a later run succeeds; the UI shows it, nothing is silently skipped.
- Jobs never place orders or touch approvals, the kill switch or `risk.toml`. Heavy jobs run off the reactor and are cancellable. One failing job records its error, shows on `data_status`, and does not stop others. Network not yet back after wake: retry with backoff inside the job's budget, then record `failed`.
- Holidays and special sessions come only from the calendar; a day the calendar does not know is not run and the owner is asked.
- Tests drive the scheduler with `FakeClock`: advance across the trigger, jump the clock across a whole session (simulated sleep), restart mid-run, skip a holiday, miss a window, run start-up catch-up with several missing dates.

## 2. Config and market facts

| Data | Source | If stale or missing |
|---|---|---|
| Instrument master (lots, ticks, expiries, segments) | broker dump, validated | F&O recommendations and options screens blocked; equity analysis continues with a warning |
| Holidays and session timings | `config/calendar.toml`, owner-maintained from the exchange circular and checked by the calendar job | refuse to run session-sensitive logic; ask the user |
| F&O ban list, price bands | official exchange files | gate treats the symbol as blocked when unknown on a trading day |
| Charges (`charges.toml`) | zerodha.com/charges and exchange circulars | P&L shows "costs unverified" badge when `verified_on` is older than the configured age |
| Events (`events.toml`) | user and official calendars | penalty not applied; Why panel says events unchecked |

## 3. Design deliverables

When asked to design a feature, produce (short; link to `DECISIONS.md` entries):

1. Context, goals, non-goals, and which phase of the build plan it belongs to.
2. Mermaid container/flow diagram: browser tab, server, core crates, ports, adapters, external systems.
3. Component view: crates touched, new ports, new adapters, new DTOs, routes and WebSocket messages.
4. Main-path and main-failure sequence (stale data, token expiry, gate block, reconnect, sleep/wake, server restart).
5. Data model and storage choice (table or Parquet layout), with migration plan.
6. Where each market fact comes from (instrument master, config, official file) so nothing is hardcoded.
7. Order-pipeline impact: does it touch Gate, approval, executor? If yes, `risk-guardian` reviews.
8. Exposure impact: new route, auth change, any non-loopback bind? If yes, ADR and `bz-security` review (a bind beyond 127.0.0.1 also needs a security redesign).
9. NFR table (`SKILL.md` section 10) and test plan (unit, property, fixture, UI) per `bz-testing`.
10. Risks, open questions, ADR(s).
11. Slices mergeable independently, mapped to developer roles: data (`bz-market-data`), quant (`bz-quant`), backtest/risk (`bz-backtest-risk`), execution (`bz-execution`), API (`bz-web-api`), UI (`bz-web-ui`), core/repo (`bz-repo-structure`).
