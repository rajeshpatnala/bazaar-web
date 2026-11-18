# Long-term mode (investing, not trading)

Open this when specifying monthly-SIP, fundamentals-screen or long-hold features. Separate bucket from the trading capital: the trading risk limits do not apply, but concentration limits do. No intraday noise, no stop-loss on price, no leverage, no F&O. Thresholds below are starting points that live in config and are tuned by sector; they are house conventions, not facts.

## 1. Fundamentals screen

Data via CSV import first (`bazaar-cli import-csv`, `bz-market-data`), a licensed source later. The Why panel states the data date and source; restated and survivorship-biased fundamentals flatter any screen.

| Factor | Starting filter (non-financial companies) | Note |
|---|---|---|
| ROE, ROCE | Both >= ~15% on a 5-year average | Consistency over one good year |
| Debt/equity | <= ~0.5, interest cover comfortably > 3 | Banks/NBFCs use different measures (asset quality, capital adequacy, margins); never apply D/E to them |
| Growth | Sales and profit CAGR >= ~10-12% over 5 years | Compare with sector, not in a vacuum |
| Cash quality | Operating cash flow positive and roughly tracking profit | Profit without cash is a red flag |
| Valuation | P/E vs the stock's own 5-10 year range and vs sector; avoid the top decile of its own history | A great company at any price is not a great investment |
| Governance | Promoter holding stable, pledging near zero, no auditor or regulatory red flags | Hard exclude on high pledging |

## 2. SIP staggering

1. Fixed rupee amount on a fixed day each month; the plan is a habit, not a market-timing signal.
2. Split each month's amount across 3-6 names (or a core index fund/ETF plus a small satellite of screened stocks). Offer a broad index fund as the default core; stock picks are the satellite and are optional.
3. Stagger entry within the month in 2-4 tranches (a few trading days apart). Optional valuation tilt: buy up to about 25% more when the stock's P/E sits in the bottom quartile of its range, and up to about 25% less in the top quartile; never skip entirely or lump-sum chase.
4. Position cap per stock (default ~10% of the long-term bucket) and sector cap; the planner shows the resulting weights in rupees and percent.
5. Review quarterly, not daily. Sell only on a **thesis break** (balance-sheet deterioration, governance event, earnings power permanently impaired), written down at purchase time as "I will reconsider if ...". Price falling alone is not a thesis break.
6. Averaging down is allowed here only as scheduled SIP buys into a thesis that is intact, never as "adding because it fell" on a broken one.

Tax treatment (STCG/LTCG, holding periods): pointer only, see `bz-india-markets`; never compute tax in strategy logic.

## 3. What to test

- Planner output for a fixed amount: tranche dates come from the calendar (`bz-india-markets`), weights sum to the plan, caps are respected, valuation tilt is bounded.
- Screen: boundary fixtures per factor (just inside, just outside), financial-sector exclusions, missing data yields "unknown", not a pass.
- Wording: the Why text for a long-term suggestion carries the data date and source and the "reconsider if" line (snapshot, `bz-testing`).
