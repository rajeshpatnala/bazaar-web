# 7. `justfile`

(From `bz-repo-structure` SKILL.md; open when the task touches this area.)


The kit owns `default`, `gate`, `dev-gate`, `test`, `check`, `db-check`, `codemap`, `promote` and the workflow recipes; never redefine them. Bazaar recipes (authoritative text: `fragments/justfile-extra`; do not invent others):

| Recipe | Does | Who runs it |
|---|---|---|
| `just setup` | `pnpm -C web install --frozen-lockfile` | anyone |
| `just fmt` | `cargo fmt --all` + `pnpm -C web format` | anyone |
| `just run` | `pnpm -C web build`, then `cargo run --release -p bazaar-server` on `127.0.0.1:8787` against `data/` and the Keychain; prints and opens a one-time login link | **human only** |
| `just ui-dev` | debug `bazaar-server --no-open` + `pnpm -C web dev` on :5173 (stop `just run` first; wiring in CONTRACTS section 7) | **human only** |
| `just set-secret <name>` | `bazaar-cli set-secret` (prompts without echo, writes the Keychain) | **human only** |
| `just app-dev` | `pnpm -C web dev:mock` (MSW, no server) for previews, screenshots, Playwright | anyone |
| `just build` | `pnpm -C web build` + `cargo build --release -p bazaar-server` | anyone |
| `just openapi` | `bazaar-server openapi > web/openapi.json`, then `pnpm -C web gen:api` | anyone |
| `just import-bhavcopy <date>` | `bazaar-cli import-bhavcopy --date` (real NSE download, writes `data/`) | **human only** |
| `just refresh-instruments` | `bazaar-cli refresh-instruments` (real Kite call) | **human only** |
| `just backtest <strategy> <from> <to>` | headless backtest on `data/` | **human only** against `data/`; agents use `cli-fixture` |
| `just lookahead` | `cargo nextest run --workspace -E 'test(/lookahead_/)' --no-tests=fail` | anyone |
| `just cli-fixture <args>` | `bazaar-cli` with `--config-dir fixtures/config` and a temp `--data-dir` (removed afterwards) | anyone (the agent-safe CLI entry) |
| `just golden` | `cargo test -q -p bazaar-backtest --test golden` | anyone |
| `just bench` / `just cov` | `cargo bench --workspace` / `cargo llvm-cov nextest --workspace --html` | anyone |

New recipes go below the kit block; the `bazaar-cli` subcommand must exist first. Gate (kit-owned): `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo nextest run --workspace`, then `pnpm -C web typecheck`, `lint`, `test:ci` once `web/package.json` exists. `just check` adds doc tests, `cargo deny check`, `pnpm -C web audit --prod`. CI adds `pnpm -C web build`, the Playwright smoke test, the OpenAPI drift check, and the `live-trading` check and clippy.

---
