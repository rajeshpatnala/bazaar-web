---
paths:
  - "crates/indicators/**"
  - "crates/options/**"
  - "crates/regime/**"
  - "crates/strategies/**"
  - "crates/scanner/**"
---
# You are in dev-quant's area (bz-quant)

1. **One implementation, two drivers.** Every indicator is a streaming struct (`update`); the batch function is a fold over it. Never write a second vectorised copy.
2. **No NaN in public output.** Warm-up yields `None`; a value is `Some` only when mathematically defined. Zero denominators (flat range, zero volume) return a documented value or `None`, never `NaN`/`inf`.
3. **No look-ahead by construction.** `on_bar` receives a view that ends at the just-closed bar. No index arithmetic that can reach forward; no centred windows; no pivot detection without the confirmation lag being modelled (a swing high is known only N bars later).
4. **No hardcoded market facts or magic numbers.** Periods, thresholds, weights, min-trade counts and the risk-free rate come from `strategies.toml` / `risk.toml` with defaults in a params struct. Lot, tick, expiry come from the instrument master (`bz-india-markets`).
5. **f64 for math, Decimal for rupees.** Convert at one adapter; round stop away from entry and targets toward entry to the tick size.
6. **Reference-value tests for every indicator and every pricing function**, plus property tests (section 8). A formula without a cited reference value is not done.
7. **Every strategy documents** idea, entry, exit, stop, best regime and known weaknesses in its doc comment, and has a row in `references/strategy-catalog.md`.
8. **A signal is not a recommendation.** Strategies emit `Signal`; only `scanner` builds a `Recommendation`, after regime, scoring, sizing (`risk`) and limits. "No high-quality setups today" is a first-class result.
9. **Never claim certainty.** Scores are ranking aids, not probabilities. PoP is a model estimate and is labelled so.
10. **Defined-risk by default** for options; naked short legs carry a warning flag, worst-case loss and margin from the broker margin API (never hand-estimated).
11. **Deterministic.** Same inputs give same ranking (stable sort with tie-break on symbol); no wall-clock reads inside quant crates, time arrives via the bar or an injected `Clock`.

More on this area: `just docs "bz-quant: <topic>"`.
