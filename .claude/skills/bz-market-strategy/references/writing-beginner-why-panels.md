# 7. Writing beginner "Why?" panels

(From `bz-market-strategy` SKILL.md; open when the task touches this area.)


The Why panel is generated from `Recommendation` fields (field list: `bz-quant`) via templates (deterministic, snapshot-tested), not free-form prose, so every number shown is a real field. The strategist owns the wording; `dev-ui` renders it; `dev-quant` supplies the fields.

**Rules**

1. **Rupee risk is the first line**, in en-IN grouping: "If this goes wrong you lose about 980 rupees (0.98% of your capital)".
2. Plain English: 8th-grade vocabulary, sentences under ~20 words, <= ~120 words in the default view. Jargon appears only with a tooltip definition (RSI, ATR, profit factor, IV rank, theta).
3. Order is fixed: **Risk -> Setup -> Why now -> Proven wrong if -> Evidence -> Watch out -> Action**.
4. Probability language only ("tends to", "has historically", "about 4 in 10 trades worked"). The banned-phrase list, the disclaimer text and the full no-advice rules are in `bz-india-markets` `references/compliance-and-disclaimer.md`; follow them, do not restate them here.
5. **Evidence is stated plainly, including bad news:** trade count, win rate, profit factor, worst drawdown, and the **Low evidence** label under the configured minimum.
6. Always state the regime and whether it fits ("market is sideways today; this setup prefers trends -> smaller size or skip").
7. Name the exit before the entry: stop price, target(s), time limit.
8. The "Watch out" line lists the nearest event risk (results, expiry, RBI, ex-date, ban list) from the events calendar.
9. "No trade" panels explain **what is missing** and what would change the answer.

**Template (fields in braces come from the Recommendation struct)**

```
Risk first: you could lose about {risk_rupees} ({risk_pct_of_capital} of your capital) on this trade.
Size: {qty} shares/lots. Entry {entry_zone}. Stop {stop}. Target {target1} (reward is {rr} times the risk).

The setup: {one_sentence_plain_description}
Why now: {reason_1}; {reason_2}; {reason_3_if_any}.
Market today: {regime_label}. {regime_fit_sentence}
Proven wrong if: price closes {stop_condition}. Then we exit and accept the loss.

Track record ({strategy} on {symbol}, after costs): {n_trades} trades, {win_rate} won,
profit factor {pf}, worst drawdown {max_dd}.  {low_evidence_sentence_if_below_min}

Watch out: {nearest_event_or_none}. Hold time: about {horizon}.
{DISCLAIMER}        <- the shared constant, never retyped
```

**No-trade template**

```
No trade today. Nothing met our rules. Your capital stays safe at {cash_amount}.
What we looked at: {n_scanned} stocks; {n_blocked_by_regime} blocked by market conditions, {n_low_evidence} had too little history.
What would change this: {condition}. Waiting is a position too.
```

Worked examples (equity, options, long-term, no-trade) are in `references/why-panel-examples.md`; open it when drafting or reviewing Why wording.

---
