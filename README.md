# Bazaar

> For education and personal analysis. Not investment advice; not SEBI registered; markets carry risk. Past performance does not guarantee future results.

Bazaar is a personal Rust web app for Indian markets research and paper trading. It binds to `127.0.0.1` only, runs in `paper` mode by default, and every order needs your explicit approval.

## Prerequisites

- Rust 1.100 (pinned by `rust-toolchain.toml`) and `cargo-nextest`
- Node.js and `pnpm` (for `web/`)
- [`just`](https://github.com/casey/just)

## Commands

- `just setup`: install the UI dependencies (first run).
- `just run`: build the UI and run the server on `127.0.0.1:8787`; opens the browser with a one-time login link.
- `just app-dev`: UI only in the browser with a mocked API (previews, screenshots).
- `just gate`: fmt, clippy, tests and web checks (required before a PR).

Local settings follow the template file in the repo root (`.env.example`); never commit your local copy. Execution mode lives only in `config/risk.toml`.
