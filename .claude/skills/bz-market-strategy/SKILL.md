---
name: bz-market-strategy
description: Trader-mentor playbook for Bazaar strategy work - capital-first philosophy, regime grid, evidence bar, IV-rank options selection, beginner Why? panels, reject list. Catalogue, long-term SIP mode and spec template are in references. Use when planning strategies, recommendations, options or long-term features.
---

# Bazaar Market Strategy (mentor-trader playbook)

Domain skill for `market-strategist`: the veteran Indian-market trader who advises the team before code is written. The strategist decides **which ideas deserve to exist, when they apply, how much evidence is enough, and how they are explained to a beginner**. It writes specs and wording, never source code. Neighbours: `bz-india-markets` (sessions, expiries, lots, events, charges; the only home of disclaimer text and no-advice wording rules in `references/compliance-and-disclaimer.md`), `bz-trading-discipline` (DecisionGate rules and `risk.toml` keys), `bz-quant` (how strategies, regime and scoring are built; the strategy catalogue in `references/strategy-catalog.md`; the only `Recommendation` field list), `bz-backtest-risk` (cost model, walk-forward, sizing, the gate).

The user: beginner, Hyderabad, Zerodha. Capital and risk limits are read from `config/risk.toml`; never bake numbers into specs as constants, quote them as "the configured limit" (the examples in this skill use the default shape of a small account and are illustrative).

Depth, opened on demand:

| Topic | File |
|---|---|
| Strategy catalogue by horizon (idea, entry, exit, stop, regime, weaknesses) | `bz-quant/references/strategy-catalog.md` (single copy) |
| Long-term SIP mode, fundamentals screen | `references/long-term-mode.md` |
| Strategy spec template | `references/strategy-spec-template.md` |
| Worked Why? panels | `references/why-panel-examples.md` |

---

## 1. Operating mode

| Concern | Choice | Note |
|---|---|---|
| Output location | `docs/strategy/<name>.md` (spec), plus wording notes in the PR or task thread | Only folder the strategist writes to |
| Source code | Never edited | Hand off to `dev-quant` (strategy, scanner), `dev-backtest` (evidence, sizing), `dev-ui` (Why panel rendering) |
| Evidence inputs | Backtest reports and walk-forward output committed, or produced on fixtures by agents with `just cli-fixture backtest --strategy ... --from ... --to ...` (reports on real `data/` come from the owner's human-only `just backtest`) | The strategist reads results, does not rerun by hand-tuning |
| Market facts | From `bz-india-markets` rules and config files | Lot size, expiry, ban list, holidays are never quoted from memory |
| Secrets, live data | Never touched; connection status only via `just db-check` | `.env` is never read by agents |
| Review chain | strategist spec -> dev-quant -> dev-backtest evidence -> `quant-reviewer` and `risk-guardian` | Risk-guardian can veto anything that weakens capital protection |

---

## 2. Philosophy (the non-negotiables)

1. **Capital first, opportunity second.** The product's first job is to keep a beginner's capital alive long enough to learn. Surviving 20 bad trades matters more than catching one big winner.
2. **"No trade" is a valid, frequent, first-class output.** "No high-quality setups today" is a success state with its own Why panel, not an empty screen.
3. **Never certainty.** Probabilities, ranges and conditions only. A signal is an idea with a price at which it is proven wrong.
4. **Every idea has an invalidation.** If you cannot state the exact price or condition that proves the idea wrong, there is no trade.
5. **Rupee risk before reward.** Everything shown to the user starts with "you could lose X".
6. **Edge must survive costs.** Brokerage, STT, exchange fees, stamp duty, GST and slippage eat small accounts alive; a strategy is judged after costs only (`bz-backtest-risk`).
7. **Process over outcome.** A losing trade that followed the rules is a good trade; a winning trade that broke them is logged as a mistake (`bz-trading-discipline`).
8. **Simple beats clever.** Few parameters, plain logic, a stated economic reason. A strategy nobody can explain in two sentences does not ship.
9. **Beginner first.** Fewer, better, larger-timeframe signals. Default posture: swing and positional equity on daily bars first; long-term mode separate (`references/long-term-mode.md`); options defined-risk and paper-first; intraday last and disabled by default until the user has a journal to show for it. Short selling in cash equity is intraday-only (delivery cannot be shorted); anything bearish and overnight goes through futures/options.

Why this posture: SEBI's own studies of individual equity F&O traders reported that roughly nine in ten lost money (FY22-FY24 study: https://www.sebi.gov.in/sebi_data/attachdocs/sep-2024/1727085659479.pdf ; FY25 press release PR No. 39/2025: https://www.sebi.gov.in/sebi_data/attachdocs/jul-2025/1751900802566.pdf ; a newer study listed on sebi.gov.in in August 2026 covers FY25-FY26: https://www.sebi.gov.in/sebi_data/attachdocs/aug-2026/1787233506209.pdf). Quote them only as "SEBI studies reported"; read the PDF before putting an exact figure into UI text. Blog and news summaries are not citations.

---

## 3. Regime first

Strategies are tools for specific weather. The `regime` crate (`bz-quant`) reports trend and volatility axes; every spec states which cells it works in.

| | Low volatility (India VIX low, ATR contracting) | High volatility (VIX elevated or rising, ATR expanding) |
|---|---|---|
| **Trending** (index above rising 50/200 DMA, ADX strong, breadth healthy) | Trend following, breakouts, buy-the-dip pullbacks; debit spreads or long options | Trend following with smaller size and wider stops; credit spreads in trend direction |
| **Range-bound** (flat averages, ADX weak) | Mean reversion, support/resistance fades; calendars | Premium selling with defined risk; mean reversion at extremes; often best to stand aside |
| **Breakdown / panic** (index under 200 DMA, breadth collapsing, VIX spiking) | No new longs; cash is a position | Defined-risk bearish or hedged only; otherwise "no trade" |

Rules: if regime is unknown or conflicting, output "no trade"; a strategy outside its regime cells is penalised or suppressed by the scanner, and the Why panel says so ("this setup works best in trending markets; today is a range").

---

## 4. Strategy catalogue

One catalogue exists: `bz-quant/references/strategy-catalog.md` (idea, entry, exit, stop, best regime, weaknesses per strategy, grouped by family and horizon). Do not copy rows into specs; link to the row and state only what is specific to this account.

Strategist notes that are not in the catalogue:
- Intraday (ORB, VWAP reversion, gap, pivots) ships disabled; MIS positions must square off before the broker cut-off (a broker fact), and costs per rupee of edge are highest here, so the cost-adjusted evidence bar bites hardest.
- Weekly or positional trend following with wide stops means a small quantity under the per-trade risk limit; often a handful of shares. Say so in the spec.
- Pair trading and other two-leg ideas need futures, are margin-heavy for a small account, and are analysis-only.
- Candlestick patterns and delivery-% are confirmation layers, never triggers by themselves.
- Options structures: section 6. Long-term: `references/long-term-mode.md`.

---

## 5. Evidence bar: when a strategy is trusted

A strategy is "trusted" only if every row passes **after costs and slippage**, measured on out-of-sample data. The numbers are house defaults; they live in config (`strategies.toml`/`risk.toml`) and enforcement lives in the DecisionGate (`bz-trading-discipline`, `bz-backtest-risk`). They are rules of thumb, not statistical proof: published guidance disagrees on exact minimums, so tightening is always allowed and loosening needs a `DECISIONS.md` entry.

| Test | Default bar | Why |
|---|---|---|
| Sample size | >= 30 out-of-sample trades; fewer is labelled **low evidence** (product Hard rule 6). Promote to "trusted" at >= 100 trades across multiple regimes | 30 trades still leave a win rate uncertain by roughly +-9 points; one lucky month can fake an edge |
| Trades per free parameter | About 30+ trades per tuned parameter in the in-sample window | More knobs than data means a curve fit |
| Expectancy | Positive after costs, in R and in rupees; gross edge at least about 3x round-trip cost | Survives cost misestimation |
| Profit factor | >= 1.3 after costs (below 1.15 is noise) | Gross profit / gross loss |
| Walk-forward | Out-of-sample/in-sample performance ratio >= 0.5 (a house default and practitioner convention, not a standard); most windows profitable (default >= 60%); no single window supplies the whole profit | Edge that only exists in one period is a story |
| Parameter stability | Move each parameter +-20%; profit factor stays > 1.1 and shape is a plateau, not a spike | A parameter cliff means the optimiser found noise |
| Drawdown | Max drawdown (and a trade-order Monte Carlo 95th percentile) tolerable for a beginner, default <= 10% of capital | Behavioural survivability, not just maths |
| Breadth | Positive across several instruments or sectors, not one lucky stock | Reduces single-name overfit |
| Trial honesty | Spec records how many variants were tried; more variants raise the bar (deflated-Sharpe logic, Bailey and Lopez de Prado) | Test seven variants and one will look great by chance |
| Data hygiene | Corporate-action adjusted, `lookahead_` tests pass, survivorship noted | `bz-backtest-risk`, `bz-testing` |
| Economic story | One paragraph on why the edge should persist and who pays for it | No story, no trust |

Evidence labels used everywhere: **Rejected** (fails a row) -> **Low evidence** (< 30 OOS trades, or passes only provisionally) -> **Provisional** (30-99 trades, all rows pass) -> **Trusted** (>= 100 trades, all rows pass). Anything below Trusted carries a visible badge and a reduced-size suggestion; Rejected strategies never reach the scanner.

---

## 6. Options selection (defined risk by default)

In short: First fix the **view** (up, down, sideways, big move either way) and the **IV rank** of the underlying (India VIX for index context; stock IV from the chain; `bz-quant` computes it, and the same lookback must be used everywhere, state it ...
Full detail, examples and patterns: `references/options-selection.md` - read it before changing this area.

## 7. Writing beginner "Why?" panels

In short: The Why panel is generated from `Recommendation` fields (field list: `bz-quant`) via templates (deterministic, snapshot-tested), not free-form prose, so every number shown is a real field. The strategist owns the wording; `dev-ui` renders ...
Full detail, examples and patterns: `references/writing-beginner-why-panels.md` - read it before changing this area.

## 8. What to reject (the strategist's veto list)

| Reject | Why | Say instead |
|---|---|---|
| **Overfit strategies**: many parameters, optimal value on a spike, great in-sample and poor out-of-sample, tuned after seeing the test set | Will not repeat; fails the stability and walk-forward rows | Simplify, widen the plateau, or drop |
| **Martingale / doubling after a loss / "recovery" sizing** | Turns a streak into ruin; violates fixed-fraction risk | Fixed fractional sizing; smaller after a losing streak (cool-off) |
| **Averaging down** on a losing position (trading mode) | Adds to a thesis the market just contradicted; blocked by the gate | Exit at the stop; re-enter only on a fresh signal |
| **Widening or removing stops** | Converts a defined loss into an undefined one | Stop moves only in the profit direction |
| **Unhedged naked option selling** (short straddle/strangle, naked calls or puts) for a small account | Unbounded or very large loss versus the risk budget; margin and gap risk exceed the account | Defined-risk spreads; if the lot does not fit, "no trade" |
| **Ratio spreads / undefined-risk legs** | One side is effectively naked | Library shows them as high-risk education only |
| **Revenge trading, over-trading, "make it back today"** | The behaviours that lose | Daily loss limit and cool-off in the gate |
| **Signals without a stop or without costs** | Not testable, not a trade | Add both or drop |
| **Illiquid, ban-listed, circuit-locked or results-day instruments** | Fills and exits are not guaranteed | Filter in scanner (`bz-india-markets`) |
| **Chasing tips, certainty claims, return promises** | Compliance and trust failure; the app is not an adviser | Probabilistic, evidence-first language |
| **Look-ahead-fed signals** (a bar's close used to trade its own open, restated data) | Fantasy results | Reject until the `lookahead_` test passes |
| **Pattern or indicator-only trades with no market context** | Low hit rate and no edge after costs | Add regime and volume confirmation |

---

## 9. Strategist workflow in planning

1. Receive the feature question (new strategy, recommendation field, options structure, long-term rule).
2. Check market facts via `bz-india-markets` and risk rules via `bz-trading-discipline`; reference them, do not restate them.
3. Decide: ship, ship-as-paper-only, defer, or reject, with reasons in plain words.
4. If shipping, write the spec (`references/strategy-spec-template.md`) and the Why wording (section 7); list open data needs (instrument master fields, fundamentals CSV columns).
5. After implementation, read the evidence report; assign the evidence label (section 5); if it fails a row, send back to dev-quant or retire the spec. Never negotiate the bar to rescue a strategy.
6. Flag anything that weakens a risk rule to `risk-guardian` immediately; it has veto.

---

## 10. Red flags, tests and checklist

**Red flags** (the full wording/compliance scan is owned by `bz-security`; these are strategy-specific): any spec with no stop; thresholds in code instead of config; "average down", "martingale", "recover losses" in specs; options specs lacking a max-loss rupee figure or an IV rank band; Why text that opens with reward instead of risk; evidence quoted without cost adjustment; percentile used where rank is the selection input.

```
rg -in 'martingale|average[ -]down|recover (your )?losses|iv[_ ]percentile' docs/strategy web/src crates
```

**What to test (owned by dev-ui / dev-quant, requested by the strategist; `bz-testing` for tooling):**

- insta snapshot tests for every Why template path: normal, low-evidence, no-trade, options, long-term. Fixture-driven Recommendation structs, no live data.
- A unit test that Why output contains the rupee risk before any reward figure, uses en-IN grouping, and passes the deny-list test from the compliance reference.
- Evidence-label tests: 29 trades -> low evidence, 30 passing -> provisional, 100 passing -> trusted, parameter-cliff fixture -> rejected.
- Options: a fixture where one lot exceeds the per-trade budget must yield "no trade" with the rupee figures; IV rank 29/31/51 fixtures choose the expected band.
- Spec/code drift: the Strategy doc comment contains every spec heading (idea, entry, exit, stop, regime, weaknesses).

**Done checklist for a strategy decision:**

- [ ] Spec in `docs/strategy/<name>.md` with idea, regime fit, entry, stop, exit, sizing note, parameters, weaknesses (>= 3), evidence plan
- [ ] Rupee risk per trade fits the configured limit for this account (or the output is "no trade")
- [ ] Evidence label assigned from backtest and walk-forward output, after costs
- [ ] Options specs: defined risk, max loss in rupees, IV rank band, event and expiry rules, margin from API
- [ ] Why wording drafted: risk first, plain English, probabilistic, uses the shared disclaimer constant
- [ ] None of the reject-list items present; `risk-guardian` and `quant-reviewer` informed
- [ ] No source files edited by the strategist; no secrets or `.env` touched
