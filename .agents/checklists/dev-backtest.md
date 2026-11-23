# Top 10: dev-backtest (from 229 findings, 2026-11-23; risk crate included)
1. Folds that missed the in-sample bar or failed the gate are excluded from pooled OOS results [QUAN-91-1].
2. Pooled metrics come from the real concatenated equity curve (drawdown not 0); trial count comes from what ran, not the caller's list [QUAN-91-2, QUAN-91-3].
3. Never `round_dp` before a `> limit` compare or a metric threshold; round for display only [RISK-52-2, QUAN-87-1].
4. The trade-count gate and the trust labels count the same filtered trades [QUAN-87-2].
5. Engine takes segment, instrument kind and slippage class from the instrument, not constants; intraday square-off is at session close, not the entry bar's close [QUAN-88-2, CR-88-1].
6. Fill price is asserted exactly (next-bar open plus slippage); `Signal.valid_until` is honoured [QUAN-89-1, CR-88-3].
7. Last equity mark is taken after the run-end force-close; drawdown peak starts at starting capital [QUAN-88-4, QUAN-86-1].
8. Equity, ref price, lot and tick are derived inside the gate from the snapshot, never taken from a caller field [RISK-52-1, RISK-61-5].
9. Expected rule levels come from a hard-coded table, not from the code under test [RISK-62-1, CR-62-2].
10. Test short entries, flips and duplicate position rows for every risk figure; short risk is not `max(intent, live)` [QR-56-1, RISK-59-2].
