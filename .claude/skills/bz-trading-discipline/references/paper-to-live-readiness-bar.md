# 4. Paper-to-live readiness bar

(From `bz-trading-discipline` SKILL.md; open when the task touches this area.)


Evaluated by code from the ledger, journal and audit log (`bazaar-cli readiness`), never self-declared. The report is stored with its inputs hash. Thresholds sit in `risk.toml` under `[readiness]` and may only be raised in the first release.

| Gate to open | Requirements (defaults) |
|---|---|
| `paper` -> `semi_auto` | 40 or more trading days **and** 50 or more closed paper trades; net expectancy after real charges and slippage > 0 and profit factor >= 1.3; max paper drawdown within 10% of capital; 0 hard-limit breaches; 0 gate bypasses; override rate <= 10% of trades; journal completion >= 95%; paper fills reconcile with the backtest cost/slippage model within tolerance; kill switch and daily-loss lockout drills passed (recorded); `DECISIONS.md` ADR; owner types the opt-in phrase |
| `semi_auto` -> `auto` | 60 or more further trading days of `semi_auto` with 50 or more live trades; all of the above on live results; zero reconciliation mismatches in the last 20 sessions; SEBI retail-algo compliance checklist complete (static IP whitelisted for order APIs - a home connection needs a static IP from the ISP, an owner decision - daily 2FA, Algo ID or broker permission for personal automation, OPS limit under broker's - see `bz-security`, `bz-india-markets`); explicit owner opt-in plus ADR |

Rules: paper must behave like real cash (charges, slippage, partial fills, no fill at circuit), otherwise paper results do not count. Paper wins of fewer than 30 trades per strategy remain "low evidence". `live-trading` is never in `default`; passing the readiness row only allows enabling it at build or run time.
