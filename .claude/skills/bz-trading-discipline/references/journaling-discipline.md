# 6. Journaling discipline

(From `bz-trading-discipline` SKILL.md; open when the task touches this area.)


The journal is how rules improve; it is also a gate input (G08 cool-off reads it).

- **Every recommendation gets a record**, including skipped ones and "no trade" days. Skipped-by-gate entries keep the failing rule ids.
- **Pre-trade section is written before approval and is immutable (append-only):** recommendation id, strategy and params, evidence snapshot (trades, PF, expectancy, "low evidence" flag), regime, planned entry/stop/target, ₹ risk, R:R, rule results, config hash, chart-state screenshot. Failure to write it blocks the order (fail closed).
- **Post-trade section:** actual fills, exit reason enum (`target`, `stop`, `trail`, `time`, `manual`, `kill_switch`), R-multiple net of charges, mistake tags (`chased`, `oversized`, `moved_stop`, `ignored_cooloff`, `early_exit`, `none`), optional mood tag (`calm`, `anxious`, `fomo`, `revenge`).
- **Review cadence:** a weekly review screen: planned vs achieved R, rule overrides, mistakes by count and ₹ cost, results by strategy, time-of-day and regime. Plain-English summary first, charts second.
- Journal text never contains tokens, account numbers or secrets (`bz-security`).
