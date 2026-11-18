---
paths:
  - "crates/**/*.rs"
---
# Rust files

- Follow `bz-core` (load it if it isn't preloaded). Area skills: `bz-market-data`, `bz-quant`, `bz-backtest-risk`, `bz-execution`, `bz-web-api`; market facts in `bz-india-markets`.
- `Decimal` for every ₹ value, quantity and charge; IST via the `Clock` trait; no `unwrap`/`expect` outside tests; no blocking in async; timeouts on outbound calls.
- Never hardcode lot/tick sizes, expiries, holidays, ban lists or rates; never log tokens or secrets.
- Strategy modules carry the doc comment: idea, entry, exit, stop, best regime, known weaknesses.
- After adding or moving modules, run `just codemap`.
