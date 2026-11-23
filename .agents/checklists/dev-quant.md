# Top 10: dev-quant (from 124 findings, 2026-11-23; indicators, options, regime, strategies, scanner)
1. Every evidence/stat port takes `as_of` and ties to the decision bar [QUAN-102-1].
2. The closed-bar check is two-sided (regime dated on the bar's session, not just not-after) and also run in the runner against the clock [QUAN-74-1, QUAN-114-1].
3. Missing data is `None`, never a stat block of zeros or profit factor 1.0 [QUAN-102-3].
4. `low_evidence` considers trades, out-of-sample flag and costs applied, and the `min_trades = 0` case [RISK-102-3, QUAN-102-2].
5. A caller-supplied rupee risk is recomputed from qty x (entry - stop) in the constructor [RISK-102-4].
6. Warm-up is the max of the lookbacks, not the sum; assert the exact first-`Some` index (ATR needs N+1 bars) [QUAN-70-1, QUAN-72-1].
7. Each indicator has a hand case with N>1 that exercises smoothing, a published reference value, and a test named `lookahead_*` [QUAN-41-1, QUAN-41-2, QUAN-42-1].
8. A guard test must fail when the guard is removed; no panic raised by the test itself inside `if let Err` [CR-77-1].
9. Exit paths check `adjusted` bars and do not need entry-only context such as regime [QUAN-77-1, QUAN-77-2].
10. Near-zero denominators (vega) return `IllConditioned`; input series are checked (interval, strictly increasing) [QUAN-38-1, QUAN-72-2].
