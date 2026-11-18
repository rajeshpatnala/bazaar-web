---
paths:
  - "crates/backtest/**"
  - "crates/risk/**"
  - "config/charges.toml"
  - "config/risk.toml"
---
# You are in dev-backtest's area (bz-backtest-risk)

1. **No look-ahead by construction.** The strategy receives a `DataView` whose backing slice ends at the current bar. There is no accessor that reaches later data. A `lookahead_` property test proves it (section 10).
2. **Decide at bar close, fill later.** Signal on bar `i` close; the order fills at bar `i+1` open (or a configured later point) plus slippage. Never fill at the close that produced the signal.
3. **Every P&L is net of costs.** Gross P&L may be shown but metrics, R-multiples, equity and the evidence bar use net P&L.
4. **Rates come from `config/charges.toml`**, each with `verified_on` and `source`; nothing numeric about charges in code or tests-of-app-code. Missing rate set is an error, never a default.
5. **Money is `Decimal`.** Prices round to the instrument tick, quantities are integers (units, lots x lot size), lot and tick sizes come from the instrument master as of the trade date.
6. **Risk never rounds up.** Sizing floors to whole lots; if one lot exceeds the risk budget the answer is "no trade", not one lot anyway.
7. **Backtest and live share code.** The same sizing function and the same `DecisionGate` run in backtest, paper and live. Backtest runs without overrides.
8. **Gate fails closed and cannot be bypassed.** Missing input is a failure; the pipeline only accepts a `GatedIntent` that only the gate can mint (execution then mints `ApprovedOrder` from it; see `bz-execution`).
9. **Deterministic.** Same bars + config + code gives byte-identical results: ordered maps, no wall clock, no unseeded randomness, fixed reduce order under rayon.
10. **Pessimistic ties.** Stop and target both inside one bar: the stop fills. Gap through a stop: fill at the open, not the stop price. Limit orders fill only if price trades through the limit.
11. **Thresholds are config.** Limits live in `risk.toml`, not in code. A threshold literal in `crates/risk` is a review blocker.
12. **Results carry provenance**: config hashes, `verified_on` range of charges used, data range, universe definition, parameter-trial count.

More on this area: `just docs "bz-backtest-risk: <topic>"`.
