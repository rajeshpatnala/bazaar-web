# F11 scanner defaults: strategist review

Date 2026-11-21. Reviewer: market-strategist. Input: `.agents/plans/F11-scanner.md`, `config/risk.toml`, `config/strategies.toml`.
Status of every number below: UNVERIFIED convention or proposal, not tuned on P&L, owner confirms later. None of it is a promise of returns; a high score means "fits the rules better", never "will win".

Owner limits used for the rupee examples (risk.toml): capital Rs 1,00,000; risk per trade 1% = Rs 1,000; daily loss 3% = Rs 3,000; max position value 25% = Rs 25,000; max 3 open positions; max 3 new entries per day.

## Verdict per item

| Item | Proposed | Verdict | Reason |
|---|---|---|---|
| Weights | conf 0.30, regime 0.25, liq 0.15, evid 0.30, sum 1 | ACCEPT with 3 notes | Sensible split; weights are a ranking aid only. See notes W1-W3. |
| Event penalty | subtract up to 15 | ACCEPT | Hard blocks (results day, policy day, ban) are in the gate (G06/G07), so the penalty only grades near-misses. Empty calendar: no penalty but a visible warning (see E1). |
| Liquidity floor | median 20-day traded value >= Rs 10 cr | ACCEPT | Conservative for a Rs 25,000 max order (about 0.025% of a day's value), but thin names bring wide spreads, circuit locks and ban risk, which a beginner cannot manage. Traded value = close x volume from bhavcopy, per day, median of the last 20 sessions. Looser later only with the owner's say-so. |
| min_trades_evidence | 30 | ACCEPT | Equals risk.toml `min_oos_trades` and the house evidence bar. Must be read from the same number, not a second copy that can drift: scanner value may be raised, never below the gate's. |
| low_evidence_cap | confidence <= 55 | ACCEPT, plus rank rule | Cap must apply BEFORE ranking (R1). 55 is below the midpoint of "good", so unproven names never look strong. |
| shrink_k | 30 | ACCEPT, plus prior rule | n/(n+30) gives 50% weight to own stats at 30 trades, 77% at 100. Fine. Shrink toward the strategy-wide stats only if those have >= 30 trades; otherwise toward a NEUTRAL prior (profit factor 1.0, expectancy 0), never toward an optimistic number (S1). |
| max_results | 10 | CHANGE to 5 | The owner may open at most 3 positions and 3 entries a day. Ten ideas is noise for a beginner. Blocked ideas are listed separately and do not use up the 5 slots (acceptance criterion 4 still holds: blocked is shown, never dropped). |
| Ban list unavailable | fail closed | ACCEPT, extend | Also fail closed when the list is older than `max_ban_list_age_hours` (24, G06) or has no date. Reason shown to the user in plain words. |
| Up x High (penalised) in ranking | donchian `up:high`, penalty 0.5 | ACCEPT, keep in ranking | Breakouts do happen in fast markets, but stops are wider and gaps bigger. Penalty halves the regime component; Why must say "conditions are rougher than ideal". It must never outrank an otherwise equal `up:normal` name (add to the golden fixture). |

## Notes

- W1 Confluence has little resolution: with 3 strategies in 3 families it takes 4 values (0, 1/3, 2/3, 1 of families agreeing). Do not over-read small score gaps; ties inside 3 points should be shown as "similar", ordering still deterministic (score desc, tradingsymbol asc).
- W2 Liquidity component: score on an ABSOLUTE log scale between the floor (0) and 10x the floor (1), not a percentile of today's universe. A percentile rewards whoever is least illiquid on a thin fixture and changes when names are added.
- W3 Evidence component while no F9 stats exist: a constant neutral-low value for everyone, so it does not reorder names, and every row carries `low_evidence`. The UI must say plainly "no backtest evidence yet", not hide it.
- R1 Rank by the capped confidence (or a score capped the same way), so a low-evidence name cannot sit above a Provisional or Trusted one just because of confluence. Labels follow `bz-market-strategy`: Low evidence (<30 OOS trades), Provisional (30-99, all bars pass), Trusted (>=100). Rejected strategies never reach the scanner.
- S1 No evidence at all = low_evidence, evidence component at its floor, never a good score. Evidence must be net of costs and out-of-sample; the `costs_applied` flag is shown in the Why.
- E1 Empty or placeholder events calendar: add warning "Results and policy dates not checked, look them up before acting." Score gets no bonus for "no events". Whether the gate blocks on an empty calendar is the risk-guardian's call (G07 is Hard); the scanner must not claim the name is event-free.
- C1 Confidence is a 0-100 score from the rules, labelled "score", never "chance" or "probability".
- F1 `shrink_k`, `low_evidence_cap`, weights: loader rejects values outside 0-1 / not summing to 1 (already planned); `low_evidence_cap` 0-100; `max_results` 1-20.
- Position size reality check: with a Rs 25,000 position cap, a stop 2.5 ATR away on a 2%-ATR stock is about 5% of price, so the risk is about Rs 1,250 at full cap, over the Rs 1,000 limit; sizing will then floor the quantity so risk stays <= Rs 1,000. Many recs will risk well below Rs 1,000. If one share exceeds the budget the answer is "no trade", not one share anyway.

## Beginner Why wording (templates; factors are structured, rendering fills the blanks)

Order: rupee risk, then why, then what would make it wrong, then the warnings.

1. Risk: "If this goes wrong and your stop is hit, you lose about Rs {risk_rupees} (your limit per trade is Rs {limit}). Quantity {qty} shares, entry Rs {lo} to Rs {hi}, stop Rs {stop}."
2. Reward: "Target Rs {t1}. On the worst fill you would make about Rs {rr} for every Re 1 you risk. A target is a hope, not a promise."
3. Why it was picked: "{n_families} of {n_total} method families agree: {names in plain words}." Never "strong buy".
4. Market fit: "The market is {trend} and this stock is {trend}, {vol} volatility. This method is meant for that kind of market." Weak cell: "This kind of market is allowed for this method but is rougher than ideal, so the score is cut."
5. Evidence: "Tested on {n} past trades after costs: {pf} profit factor. Fewer than 30 trades means this is low evidence and the score is held down." None: "No testing result is shown for this method. Treat this as an idea to study, not a signal to trust."
6. Liquidity: "About Rs {x} crore of this share trades on a typical day, so you can get in and out."
7. What would make it wrong: "The idea is wrong if the price closes below Rs {stop}, or if it has not worked after {max_hold} days."
8. Events and bans: "No ban today (list dated {date})." / "Ban list not available, so this name is hidden for safety." / "Results or policy dates were not checked."
9. Footer on every rec: "Educational tool, not investment advice. Scores are not probabilities and can be wrong."

Forbidden words: guaranteed, sure, certain, can't lose, risk-free, "will".

## Evidence needed before these defaults are called verified

Owner confirmation of weights, floor, k, cap, max_results; F9 stats flowing in through the EvidenceSource port; a golden fixture proving (a) a low-evidence name never outranks a Provisional one, (b) up:high ranks below up:normal when otherwise equal, (c) ban list missing/stale yields zero candidates with reasons.

## Failure modes

Weights look precise but are untested; confluence of correlated signals can over-count (family dedupe mitigates); fixture-only data means rankings say nothing about the real market; a high rank can still lose the full Rs 1,000.
