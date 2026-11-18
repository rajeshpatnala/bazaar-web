# 6. Options selection (defined risk by default)

(From `bz-market-strategy` SKILL.md; open when the task touches this area.)


First fix the **view** (up, down, sideways, big move either way) and the **IV rank** of the underlying (India VIX for index context; stock IV from the chain; `bz-quant` computes it, and the same lookback must be used everywhere, state it in the spec). **Selection uses IV rank bands; IV percentile is shown as context only**, never as the selection input (same statement in `bz-quant` `references/options-math.md`).

| IV rank | Options are... | Directional view | Preferred defined-risk structure | Avoid |
|---|---|---|---|---|
| Low (< ~30) | Cheap | Up / down | Debit spread (bull call, bear put), or a single long option with a premium-capped stop | Selling premium for tiny credit |
| Low | Cheap | Big move, direction unknown | Long straddle/strangle only with a catalyst; otherwise skip | Holding through theta decay hoping |
| Middle (~30-50) | Fair | Any | Discretionary; debit spreads in clear trends, credit spreads in clear ranges; otherwise "no trade" | Forcing a trade |
| High (> ~50) | Rich | Range-bound | Iron condor / iron butterfly (wings bought) | Naked short strangle |
| High | Rich | Mild up / down | Credit spread (bull put, bear call) at a level the chart supports | Long options (overpaying for vol) |
| Any | | Own the stock, mildly bullish | Covered call or cash-secured put, only if the lot fits the account | Over-sizing |
| Any | | Term-structure view | Calendar / diagonal | Around events without checking the front expiry |

The 30/50 bands are config values and a house convention to be tuned by backtest (many options-education sources use a 50 split; IV rank differs by platform), not a law.

Rules the strategist enforces in every options spec:

1. **Max loss in rupees is computed first** (spread width x lot - credit for credit spreads; premium x lot for long options) and must be <= the per-trade risk budget. If even one lot exceeds it, the output is "No trade: one lot risks X, your limit is Y". Do not shrink the structure until it becomes meaningless.
2. **Contract size is the key constraint for a small account.** SEBI tightened index-derivative contract sizes in late 2024 (circular of 2024-10-01 on measures for equity index derivatives; confirm the number and current lot sizes at sebi.gov.in and the instrument master). One index lot can control many times the whole account, so many index spreads and every covered call or cash-secured put exceed the per-lot risk budget. Expect options output to be mostly "paper-only education" or "no trade", and say so honestly rather than loosening the limit.
3. **Margin comes from the broker margin API**, never from a hand estimate; if it cannot be fetched, F&O paper orders are blocked.
4. **Events:** no short-premium structures through results, RBI policy or Budget unless the spec is an explicit event strategy that passed the evidence bar. Expiry-day afternoons: gamma risk is extreme, so entries on expiry day are suppressed by default for beginners. The expiry instant is `calendar.close_time(expiry_date, segment)`, not a remembered clock time (`bz-india-markets`).
5. **Exits are part of the spec:** profit target as % of max profit (default band 50-75% for credit structures), loss stop on premium or on the underlying, and a time exit well before expiry. Never "hope" past the stop; never widen it.
6. **Show the payoff:** breakevens, max profit, max loss, probability of profit (a model estimate with its assumptions), and days to expiry in the Why panel.
7. Ratio spreads, naked short options and any structure with undefined risk on one side: high-risk flag, excluded from recommendations (section 9).

---
