# Strategy spec template (`docs/strategy/<name>.md`)

Open this when the strategist writes or reviews a spec before `dev-quant` implements. Keep a spec under about two pages. The Strategy trait doc comment (idea, entry, exit, stop, best regime, weaknesses) is copied from it so spec and code cannot drift. Parameters live in `config/strategies.toml` (schema: `bz-quant`) with the defaults named here. The catalogue of existing strategies is `bz-quant/references/strategy-catalog.md`; add the spec's row there, do not duplicate it here.

```markdown
# Strategy: <name>          Status: draft | specified | implemented | validated | retired
Horizon: intraday | swing | positional | options | long-term       Direction: long | short | both
Owner: market-strategist   Implementer: dev-quant   Date: <YYYY-MM-DD>

## Idea and economic story
<Two to four sentences: what behaviour is being captured, why it should persist, who is on the other side.>

## Universe and filters
<Which instruments (index, F&O stocks, Nifty 500), min liquidity, excluded (ban list, results week, circuit).>

## Regime fit
Works: <trend/range x vol cells>   Suppress in: <cells>   Regime inputs: <indicators from the regime crate>

## Entry (evaluated on closed bars only)
<Precise, testable rules. Order type and entry zone width.>

## Stop and invalidation
<Exact stop rule (e.g. 2 x ATR(14) below entry or below swing low). Stops never widen.>

## Targets and exits
<Target logic, trailing rule, time stop, event exit.>

## Sizing
Risk per trade = the configured limit from risk.toml. Quantity = risk rupees / stop distance, rounded down (lots for F&O).
Notes on small-account fit: <expected stop distance in rupees, typical quantity, whether one lot fits the budget>.

## Parameters
| Name | Default | Range for stability test | Meaning |
|---|---|---|---|

## Known weaknesses
<Where it loses money: whipsaw, gaps, events, crowded trade. At least three honest items.>

## Evidence required before "trusted"
Per section 5 of bz-market-strategy (house defaults from config): OOS trades, profit factor after costs, walk-forward ratio, parameter plateau, variants tried: <n>.
Backtest universe/period: <...>. Cost model: from charges.toml (verified_on <date>). Slippage: <bps>.
Results (filled by dev-backtest): <link or table: trades, win rate, PF, avg R, expectancy, max DD, walk-forward windows>

## Why? panel inputs
Fields the Recommendation must carry for this strategy (field list: bz-quant): <reason strings, trigger values>. Wording draft: <3-line Why text>.

## Acceptance criteria for dev-quant
- [ ] Strategy trait implemented; `on_bar(&DataView, &StrategyContext)` sees only past data; doc comment matches this spec
- [ ] Unit tests with a hand-computed fixture (known candles -> known signal, stop, size)
- [ ] `lookahead_` test passes; golden backtest baseline committed
- [ ] Parameters in strategies.toml; no thresholds hardcoded

## Open questions / rejected variants
<Variants tried and dropped (count feeds the trial-honesty row).>
```
