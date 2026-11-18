---
paths:
  - "crates/indicators/**"
  - "crates/options/**"
  - "crates/regime/**"
  - "crates/strategies/**"
  - "crates/scanner/**"
  - "crates/backtest/**"
  - "config/charges.toml"
---
# Quant and backtest

- Load `bz-quant` / `bz-backtest-risk`. Only past data at decision time (no look-ahead) with a test proving it; full Indian costs + slippage in every P&L.
- New formulas need reference-value tests with stated tolerances. Charge rates need `source` + `verified_on`.
- PRs here need a `quant-reviewer` review (tell the tech lead in your handoff).
