# 5. Configuration

(From `bz-core` SKILL.md; open when the task touches this area.)


- Files in `config/` (eight; schema owners in `bz-repo-structure` section 4): `charges.toml`, `risk.toml`, `strategies.toml`, `watchlists.toml`, `events.toml`, `calendar.toml`, `data.toml`, `compliance.toml`. Each starts with `schema_version`, `verified_on = "YYYY-MM-DD"` and `source = "<url | circular no. | owner>"`. Corporate actions are store data (`bazaar-cli import-actions <csv>`), not a config file.
- Load with `serde` + `#[serde(deny_unknown_fields)]`, then a `validate()` that checks ranges and cross-field rules. Fail at startup naming the key, never the value of a secret.
- The loader resolves the directories as `--config-dir` flag, else `BAZAAR_CONFIG_DIR`, else `./config`; and `--data-dir`, else `BAZAAR_DATA_DIR`, else `./data`. Both are paths, not secrets, and are the only env-driven config besides secrets (`BAZAAR_PORT`, `RUST_LOG` are read by the server entrypoint). Credentials never live in `config/`.
- Defaults for the user profile (capital 1,00,000, 1% risk per trade, 3% daily loss, paper mode) live in the shipped `config/*.toml`, not in code. Code reads config; it does not carry the numbers.
- Charge rates, STT/CTT, exchange fees, brokerage and Kite API fees: never type them from memory or a blog. Fill `charges.toml` from zerodha.com/charges and exchange circulars, set `verified_on`, and put the URLs in `source`. Third-party figures conflict (the F&O STT changed on 1 April 2026), so a rate without a dated official source is rejected in review.
- Staleness: a file whose `verified_on` is older than its configured limit shows a warning badge and a `warn!`; it is never silently trusted for live decisions.
- Execution mode is `[mode] execution_mode` in `risk.toml`, `paper` by default, never set by env var or CLI flag. Key names come from `bz-trading-discipline` only. `auto` is locked; changing the default or unlocking it is an ADR (`bz-architecture`).
- Refreshable market data (instrument master, holiday list, ban list) is data, not config: it lives in the store with a `fetched_at` and is refreshed daily (`bz-market-data`).
