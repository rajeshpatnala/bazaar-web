# 8. Tests (required in every PR touching these crates)

(From `bz-quant` SKILL.md; open when the task touches this area.)


- **Unit, reference values:** tolerances come from `bz-testing` (closed-form 1e-8 absolute, iterative solvers such as IV 1e-6; one choice, not per-skill). Each indicator vs a TA-Lib or ChartSchool fixture from the first `Some` (fixtures store full-precision values; a source that seeds differently is compared after the skip window in the indicators reference); Black-76/BS vs machine-generated QuantLib grids at those tolerances; textbook values (Haug, Hull) are published to 2-4 decimals and are checked only to their printed precision; Greeks vs central finite differences of `price`; IV round trip `iv(price(sigma)) == sigma` within the iterative tolerance.
- **Property (`proptest`):** streaming equals batch; RSI/Stochastic within [0,100]; ATR >= 0; Bollinger lower <= mid <= upper; put-call parity; call price monotone in sigma; gamma and vega >= 0; payoff breakevens evaluate to ~0; scanner output sorted and deterministic.
- **No-look-ahead:** test names start with `lookahead_` so `just lookahead` (fails on zero tests) runs them. For each strategy, run on candles `0..=t`, then on candles where everything after `t` is randomised; signals at `<= t` must be identical.
- **Fixtures:** under the tree owned by `bz-testing` (`fixtures/reference/{indicators,options}/`, `fixtures/config/`, ...): recorded candles and option-chain JSON; regime and scanner tests on a small synthetic universe; never live APIs. `FakeClock` for session logic; expiry instants from the fixture calendar's `close_time`.
- **Snapshots (`insta`):** Why text and Recommendation JSON for one buy, one options structure, one low-evidence case, and the no-setups result.
- **Edge cases:** empty/short history, flat prices, zero volume, expired option, IV below intrinsic, empty chain, ban-listed symbol.

Commands: `cargo nextest run -p bazaar-indicators -p bazaar-options -p bazaar-regime -p bazaar-strategies -p bazaar-scanner`, then `just lookahead` and `just gate`. Fixture generation scripts (TA-Lib via Python) are run by a human or in CI tooling once; tests never shell out.
