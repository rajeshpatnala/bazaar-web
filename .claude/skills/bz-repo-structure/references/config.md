# 4. `config/` (eight files)

(From `bz-repo-structure` SKILL.md; open when the task touches this area.)


Every file starts with `schema_version`, `verified_on = "YYYY-MM-DD"` and `source = "<url | circular no. | owner>"` (`source = "owner"` for owner-set thresholds). There is no `reviewed_on`.

| File | Holds | Schema owner |
|---|---|---|
| `charges.toml` | effective-dated rate sets per segment and side | `bz-backtest-risk` `references/charges-toml.md` |
| `risk.toml` | `[mode]`, `[limits]`, gate thresholds, `[override]`, `[hard_limits]`, readiness | `bz-trading-discipline` (G-table is the only key list) |
| `strategies.toml` | enabled strategies and params | `bz-quant` |
| `watchlists.toml` | named symbol lists, universe filters (user-owned) | this skill |
| `events.toml` | dated events (results, RBI, Budget, ex-dates) | `bz-india-markets` |
| `calendar.toml` | holidays, sessions per segment, special sessions | `bz-india-markets` (schema); API in section 5 |
| `data.toml` | broker/vendor rate limits, URL templates, refresh windows (each with `verified_on`) | `bz-market-data` |
| `compliance.toml` | live-phase checklist state (phase 9 only) | `bz-security` `references/sebi-algo-compliance.md` |

Corporate actions are not a config file: they live in the SQLite store, imported with `bazaar-cli import-actions <csv>`.

Conventions:
- Money, rates and percentages are **strings** that deserialize into `Decimal`; TOML floats are rejected for them.
- `#[serde(deny_unknown_fields)]` on every config struct; a typo fails startup.
- Market-fact numbers (rates, lot sizes, holidays, session times) are never typed from memory. If you cannot cite a source, stop and ask; record changed sources in `DECISIONS.md`.
- The loader warns when `verified_on` exceeds the per-file limit (`charges` 90 days; `calendar` needs the current year). Staleness shows in the UI and in `check-connections` as a `stale` category, never a silent pass.
- Defaults for the owner profile (IST, risk %, daily loss %, capital, `execution_mode = "paper"`) are values in `risk.toml`, never code constants. Mode is never set by env var or CLI flag.
- Config path = `--config-dir`, else `BAZAAR_CONFIG_DIR`, else `./config`; data path likewise with `--data-dir` / `BAZAAR_DATA_DIR` / `./data`. Paths are not secrets. Credentials never live in `config/`.
- Loading lives in `bazaar_core::config` (`Config::load(dir)`, typed structs, `validate()`) so cli, server and tests share it. Sketch: `references/config-and-calendar.md`.

---
