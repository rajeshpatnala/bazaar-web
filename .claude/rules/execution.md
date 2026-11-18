---
paths:
  - "crates/risk/**"
  - "crates/execution/**"
  - "crates/paper/**"
  - "crates/broker/**"
  - "config/risk.toml"
---
# Risk and execution (capital protection)

- Load `bz-trading-discipline` and `bz-execution`. Every path to an `Executor` goes through the DecisionGate and, in semi-auto, a user approval; the gate re-runs at approval time.
- Thresholds only from `config/risk.toml`; no bypass flags; each rule keeps its Override level from the `bz-trading-discipline` G-table (Hard ceiling / No / Soft / Paper).
- `live-trading` is never a default feature; tests never place real orders.
- PRs here need a `risk-guardian` review (tell the tech lead in your handoff).
