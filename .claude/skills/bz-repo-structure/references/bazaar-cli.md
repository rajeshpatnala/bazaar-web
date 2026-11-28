# 6. `bazaar-cli` (`crates/cli`)

(From `bz-repo-structure` SKILL.md; open when the task touches this area.)


Headless composition root, `clap` derive. Subcommands (contract): `check-connections`, `refresh-instruments`, `import-bhavcopy --date`, `import-actions <csv>`, `import-csv <file>`, `backtest --strategy --from --to`, `readiness`, `live-opt-in --mode` (human-only), `history-checkpoint verify`, `history-checkpoint accept-current` (human-only), `paper-session list`, `paper-session new` / `resume` (human-only; not built yet), `set-secret <name>` (human-only; prompts without echo and writes to the Keychain through `SecretStore`). All accept `--config-dir` and `--data-dir`; T1 runs use `--config-dir fixtures/config --data-dir <tmp>`. The bin entrypoint loads `.env` with `dotenvy` as the secrets fallback (the program loads it; agents never open it), builds config, then calls library crates. No business logic here. Backtest output is deterministic text/JSON for golden tests. There is no password, TOTP or hash command: the web app has no password (launch-token session, `bz-web-api`).

The `bazaar-server` binary has one extra subcommand, `openapi`, which prints the OpenAPI document to stdout (no network, no secrets, no data dir) for `just openapi` and the CI drift check. With no subcommand it starts the server (flag `--port`; no bind-address flag).

`check-connections` is what `just db-check` runs. The `kite-session` check reads the Keychain with a short timeout and prints `FAILED kite-session auth` rather than waiting on a macOS access prompt (a rebuilt unsigned binary can trigger one); agents treat that line as expected, never retry it. Contract:
- One line per check on stdout, fixed order, `OK <check>` or `FAILED <check> <category>`. Checks: `sqlite`, `duckdb`, `kite-session`, `instruments`, `audit-chain`. Other skills reference these names; none adds its own output format.
- `<category>` is a closed enum with fixed strings: `missing`, `auth`, `network`, `stale`, `config`, `io`, `unavailable`. Never print an error message, URL, header, path with user names, token, key or secret; detail goes to `tracing` at debug with redaction (`bz-core`).
- Exit 0 only if all lines are `OK`, otherwise 1. Stdout carries only result lines (logs go to stderr).
- `kite-session` makes one cheap read-only call and never touches orders; no credentials prints `FAILED kite-session missing`. `audit-chain` verifies the append-only audit hash chain (`bz-execution`). Kite client details: `bz-market-data`.
- Output goes through one `output` module taking `&mut impl Write`; tests pass a `Vec<u8>`.

```rust
pub trait Check { fn name(&self) -> &'static str; fn run(&self) -> Result<(), Category>; }
pub fn run_checks(checks: &[Box<dyn Check>], out: &mut impl std::io::Write) -> bool { /* prints lines, returns all_ok */ }
```

**Human-only commands:** the single list is CONTRACTS section 5 (`run`, `ui-dev`, `set-secret`, `refresh-instruments`, `import-bhavcopy`, `backtest` on `data/`, `bazaar-cli live-opt-in`, `bazaar-cli history-checkpoint accept-current`, `bazaar-cli paper-session new` / `resume`). Agents use fixtures and temp dirs (`just cli-fixture`), test the server with in-process `tower::ServiceExt::oneshot` tests, preview the UI with `just app-dev`, and `just db-check` is the only connection check.

Wiring: `bazaar-cli` and `bazaar-server` are the only crates that construct adapters (`Store`, `MarketData`, `Executor`, the Keychain `SecretStore`) and inject them. If duplicated wiring exceeds ~50 lines, propose a `bootstrap` crate by ADR.

---
