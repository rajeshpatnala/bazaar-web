# 6. Recommendation card

(From `bz-web-ui` SKILL.md; open when the task touches this area.)


The DTO is `components["schemas"]["RecommendationDto"]`, generated from the server. The field list lives only in `bz-quant`; this skill lists what the card shows and must not restate or extend the DTO. Money arrives as decimal strings. The card displays:

- instrument, action (a `NoTrade` result renders the empty state), horizon, regime badge with summary;
- **risk in rupees first** ("You risk ₹1,000 if the stop is hit"), then entry zone, stop, targets with reward:risk, size (quantity, or lots for F&O);
- confidence (a score, not a probability), agreeing strategies, backtest strip (trades, win rate, profit factor, average R, max drawdown, period, costs applied);
- `LowEvidenceBadge` next to confidence whenever the server flags low evidence; text states the trade count and a tooltip explains why few trades means weak proof;
- warnings (naked short, ratio spread, event nearby, stale data) in a prominent block above the entry details, never collapsed;
- gate state: a blocked recommendation shows each failing rule and offers no path to create an order intent;
- data-as-of for the delayed badge; Why? sections under a disclosure.

Why? panel: `<details>` or a button with `aria-expanded`, rendering the server-supplied sections in order (`bz-market-strategy` owns the template and wording). Render text only; never markdown-to-HTML of server strings.

```tsx
export function LowEvidenceBadge({ trades }: { trades: number }) {
  return (
    <span role="status" className="badge badge-warn" title="Fewer than about 30 past trades: treat the statistics as weak.">
      Low evidence: {trades} past trades
    </span>
  );
}
```

The empty list renders "No high-quality setups today" with the regime summary, never a blank table. The regime badge and the data-delayed badge are shared components used by the card, the chart header and the shell.
