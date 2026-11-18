# 5. F&O ban, circuits, price bands

(From `bz-india-markets` SKILL.md; open when the task touches this area.)


- Ban: a stock enters the ban period when aggregate open interest crosses an entry percentage of its market-wide position limit (MWPL); during the ban only position-reducing trades are allowed, and normal trading resumes when OI falls below an exit percentage. Index derivatives have no MWPL ban. Reported by broker explainers (https://support.zerodha.com/category/trading-and-markets/trading-faqs/f-otrading/articles/why-do-futures-and-option-scrips-enter-ban-period-what-does-it-mean); verify the percentages at the exchange's F&O ban circular. They are config values (`ban_entry_pct`, `ban_exit_pct` with `verified_on`), not literals, and the test fixtures use fictional values around them. Approaching-ban stocks (say above ~80% MWPL) deserve a "ban risk" tag, with the cut-off configurable.
- Banned stocks: scanner marks "no new F&O positions" and DecisionGate (`bz-backtest-risk`) rejects openings; closing orders stay allowed. Paper trading enforces the same rule.
- Price bands: each cash stock has a daily band (percentage varies per stock and changes), carried in the exchange's daily band file and on the tick. F&O stocks and indices follow different (dynamic) mechanisms. Market-wide index circuit breakers halt the whole market at several levels. Backtest/paper fills must not execute at or beyond the band, and a locked circuit (no counter-side liquidity) means no fill. Never encode "10%/20%" in code.
- Stock under surveillance (ASM/GSM), T2T segment, and trade-for-trade lists restrict intraday and leverage; treat as `restrictions` flags from the exchange files, surfaced in Why? panels.
