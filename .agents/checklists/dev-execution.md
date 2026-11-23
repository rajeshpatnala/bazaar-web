# Top 10: dev-execution (from 124 findings, 2026-11-23; execution, paper, broker)
1. `reduce_only` is derived from held positions plus pending exits (sum of open exits <= held), never read from the intent [RISK-134-3, RISK-109-3].
2. A check that consumes a rate slot runs once per order; test the pipeline-then-executor double call [RISK-105-3].
3. A breaker trip or kill-switch engage is persisted before it returns; test a restart with the default config [RISK-105-1, RISK-100-1].
4. Use the decision the repo returns (first wins), not the one you passed in [RISK-109-1].
5. When `place` is Ok and a later local write fails, report "placed, bookkeeping failed", never a plain failed order [RISK-109-2].
6. Every await on the approval path (quote read, gate, send) is bounded; test with a hanging fake [RISK-109-10].
7. The gate adapter passes the fresh quote snapshot through; a missing reference price with a drift limit set blocks [RISK-134-1, RISK-134-2].
8. Paper sells check inventory (no naked short credited as cash); `reset(opening)` is bounded and tied to config [RISK-113-1, RISK-113-3].
9. Time and session inputs (`square_off_at`, `session_open`) are validated in the constructor or computed from the calendar, not trusted from the caller [RISK-119-3, RISK-119-4].
10. Every fill outcome is audited (user fills, cancels, expiries, drops); a drop never follows a written audit row; lock tests run multi-thread [RISK-119-5, RISK-119-2, TS-113-3].
