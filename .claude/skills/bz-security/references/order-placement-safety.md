# ## 7. Order-placement safety (live phase and its scaffolding)

(From `bz-security` SKILL.md; open when the task touches this area.)


Detail and tests live in `bz-execution` and `bz-trading-discipline`; the reviewer verifies:

- `broker` compiles only behind `live-trading`; default builds and tests never link a live executor; CI builds the feature but never calls live APIs.
- Mode changes (`paper -> semi_auto -> auto`) are explicit typed confirmations, audited, and impossible by config edit alone for `auto`.
- Kill switch: global, persisted in SQLite, checked in the executor just before every send, set and released only by `PUT /api/v1/kill-switch` from a logged-in session (never Telegram, CLI flag or env var). The daily-loss breaker trips the same switch. Hard ceiling rows in the `bz-trading-discipline` table.
- Rate limiter in front of the broker client, far below the broker limit (as of 2026-11-18 Zerodha reports 10 orders per second per client; verify in `sebi-algo-compliance.md`). Client order tag per intent; after a timeout reconcile via the order book, no blind retry. Reconciliation on start and reconnect freezes orders on mismatch.
- Audit rows hold ids, times, actor, mode, intent hash, verdict, override reason, broker order id, result, `prev_hash` and `hash`; no secrets or full broker payloads. Agents never set `ok = true` in `config/compliance.toml`.
- **Live-phase prerequisite (owner decision, not a kit feature):** SEBI's retail algo framework requires order APIs to be called from a whitelisted static IP (reported, as of 2026-11-18). A home connection needs a static IP from the ISP, or another owner decision, before phase 9. The owner records the choice, date and evidence in `DECISIONS.md`; never route orders through a third-party relay. Details, checklist item `static_ip` and the verification sources: `references/sebi-algo-compliance.md`. Paper and data phases are unaffected.
