# Options math (crate `options`)

Open this when pricing, solving IV, computing Greeks or chain analytics, or building the payoff engine. Contract facts (lot size, expiry instant, strike step) come from the instrument master (`bz-india-markets`, `bz-market-data`); NSE index and stock options are European style, but check the contract specification for the instrument before relying on it for a new product. The math is f64; premiums and payoffs that become rupees are `Decimal` at the boundary.

## 1. Which model

- **Black-76 (default).** Prices a European option on a forward/future `F`. Index and stock options in India trade against a same-expiry future, so the market already embeds carry and dividends in `F`. Using `F` avoids guessing the dividend yield and spot-based carry. Inputs: `F`, `K`, `T`, `r`, `sigma`.
- **Black-Scholes-Merton with yield q (fallback).** Use only when no same-expiry future exists (new contract, EOD import without futures). It is Black-76 with `F = S * exp((r - q) * T)`. Dividends are lumpy for stocks: pass the expected dividend explicitly and flag the result `approx_dividends`; for stocks around ex-dates, prefer the future.
- **Forward from parity when the future is stale/absent:** `F = K + exp(r*T) * (C_mid - P_mid)` at the strike with the smallest |C - P|; use two or three strikes and take the median.
- **Rate `r`:** a single config value (short-term government/MIBOR-type rate) with `verified_on`; never a literal in code. Black-76 sensitivity to `r` is small (only discounting), another reason to prefer it.
- **Time `T`:** years = minutes from the valuation instant to the expiry instant / (365 * 24 * 60). The expiry instant is `calendar.close_time(expiry_date, segment, kind)` (`bz-india-markets`; the expiry date itself comes from the instrument master, already holiday-shifted). Never a literal clock time: session ends change by circular, and the options test fixtures take the close time from the fixture calendar. Floor at a small epsilon (a few minutes) and return `ExpiredOrZeroT` below it. Intraday on expiry day, `T` is tiny and gamma/theta explode: that is real, show it with a warning.

## 2. Prices and Greeks

With `d1 = (ln(F/K) + sigma^2 * T / 2) / (sigma * sqrt(T))`, `d2 = d1 - sigma * sqrt(T)`, `D = exp(-r*T)`, `N` the standard normal CDF, `n` the PDF:

- Call = `D * (F*N(d1) - K*N(d2))`; Put = `D * (K*N(-d2) - F*N(-d1))`. Parity: `C - P = D * (F - K)`.
- Delta (vs the future): call `D*N(d1)`, put `-D*N(-d1)`. Spot delta differs by the factor `dF/dS` (about `exp((r-q)T)`); say which one the UI shows (default: delta vs the underlying future, label it).
- Gamma (vs F): `D * n(d1) / (F * sigma * sqrt(T))`, same for call and put.
- Vega: `D * F * n(d1) * sqrt(T)`, same for both. Display per 1 volatility point (divide by 100).
- Theta (calendar time, value change as time passes): `r*V - D * F * n(d1) * sigma / (2*sqrt(T))` for both call and put (V = option value). Display per calendar day (divide by 365). Derived by differentiating the price with respect to `T`; sources write theta and rho differently, so this one is verified by a finite-difference test, not by trusting any single page.
- Rho with `F` held fixed: `-T * V`. Minor for the short-dated options traded here; show only in an advanced view.
- Black-Scholes-Merton variants: `delta = exp(-qT)*N(d1)` (call), `gamma = exp(-qT)*n(d1)/(S*sigma*sqrt(T))`, `vega = S*exp(-qT)*n(d1)*sqrt(T)`; implement by converting to Black-76 and scaling, so there is one core.
- Position Greeks = sum of `signed_qty * lot_size * greek` per leg. Expose raw (per-unit, per-year) values from the library and display conversions (per day, per vol point) in a separate `DisplayGreeks` so tests of the math stay unit-clean.

## 3. Implied volatility solver

1. Reject early with typed errors: `T` below epsilon; price at or below discounted intrinsic (`D*max(F-K,0)` call, `D*max(K-F,0)` put) within tolerance, which carries no vol information; price above the upper bound (`D*F` call, `D*K` put).
2. Solve on the OTM side: convert an ITM quote to the OTM one with parity (`C = P + D*(F-K)`) because the OTM price has far better vega.
3. Seed: Brenner-Subrahmanyam `sigma0 = sqrt(2*pi/T) * (price / (D*F))`, accurate near the money; clamp to a sane range (about 0.01 to 3). Far from the money use the clamped seed or midpoint of the bracket.
4. Keep a bracket `[lo, hi]` (default 1e-4 to 5.0). Evaluate `f(sigma) = model(sigma) - price`; tighten the bracket from its sign (price is increasing in sigma).
5. Newton step `sigma - f/vega`. If vega is below a floor (relative to price), the step leaves the bracket, or the step is not finite, take a bisection step instead. Stop when `|f| < 1e-8 * max(1, price)` or the bracket is narrower than 1e-10; at most 100 iterations; else `NoConvergence`.
6. Quote hygiene for chains: use the bid/ask mid; require bid > 0 and a spread percentage under a config limit, otherwise return IV flagged `illiquid` (or none). Premiums are tick-quantised (a tick is a large fraction of a far-OTM premium), so far-wing IV is noisy: do not draw a smile through it without the flag. LTP of a stale or illiquid strike is not a price.
7. Jackel's "Let's Be Rational" is the high-precision alternative if the Newton/bisection hybrid ever proves too slow or imprecise in the wings; adopt it behind the same function signature only with an ADR (`bz-architecture`).

## 4. IV rank, percentile and context

- Series: daily ATM IV at a fixed time (for example the 15:15 IST snapshot), interpolated to constant 30-day maturity, stored by the `store` crate. For indices India VIX history is a usable proxy; for stocks keep your own history (not available from the free feed, so new instruments start with `insufficient_history`).
- `IV rank = 100 * (IV - min) / (max - min)` over the lookback (default 252 trading days, config); `None` if max equals min.
- `IV percentile = 100 * (days with IV below today's) / (days in lookback)`.
- They disagree after a spike (rank stays low, percentile higher). **Selection uses IV rank bands** (the 30/50 bands in `bz-market-strategy` section 6, config values); show IV percentile beside it as context, never as the selection input, so both skills quote one metric. Require a minimum observation count (config) before showing either.
- Also expose `IV - realised vol` (premium over realised) using close-to-close volatility of the last N days; this is context for selling versus buying premium (`bz-market-strategy`), not a signal alone.
- Expected move to expiry (1 standard deviation): `F * sigma * sqrt(T)`; the ATM straddle mid is the market's own estimate. Show breakevens against it.

## 5. Chain analytics

- **PCR:** `sum(put OI) / sum(call OI)` for the chosen expiry (OI PCR) and `sum(put volume)/sum(call volume)` (volume PCR); optionally restricted to strikes within +/- N steps of ATM (param). Denominator zero gives `None`. There is no universal "neutral" level: interpret against the instrument's own history (percentile) and its change over the session; put thresholds in config, never in code.
- **Max pain:** for each candidate expiry price `X` (the pain function is piecewise linear with kinks at strikes, so evaluating at strikes suffices) compute `sum over calls OI_K * max(X-K, 0) + sum over puts OI_K * max(K-X, 0)` (option-writer payout at expiry). Max pain is the `X` with the smallest total. Report ties (all minimisers) and mark the one nearest spot. Prefix sums make it O(strikes). Use the chosen expiry only; the evidence that price gravitates to it is weak, so show it as context, mostly relevant in expiry week, never as a signal.
- **OI walls:** the call strikes above spot and put strikes below spot with the highest OI, plus change-in-OI since the previous session or snapshot. OI alone cannot tell writers from buyers or spreads from outright positions, so label them "heavy open interest", not "resistance guaranteed".
- **Futures OI build-up** (compare today to the previous session, or this snapshot to the previous one, for the same contract):

| Price | OI | Label | Reading |
|---|---|---|---|
| up | up | long build-up | fresh longs |
| down | up | short build-up | fresh shorts |
| up | down | short covering | shorts closing, may fade without fresh buying |
| down | down | long unwinding | longs closing |

  Apply minimum move thresholds (price % and OI %, config) and return `Neutral` below them. Near expiry, OI in the expiring contract falls from rollover alone, which mimics unwinding/covering: during the roll window (config days) classify on near-plus-next-month OI combined, or flag `rollover_distortion`. The same grid applied to an option contract's own premium and OI describes buying versus writing, but premium also moves with IV and the underlying, so keep it as an annotation.
- **Rollover %** = next-month OI / (near + next-month OI) on or near expiry; **basis** = future - spot, annualised `(F/S - 1) * 365 / days_to_expiry`.

## 6. Payoff engine and probability of profit

```rust
pub struct Leg { pub kind: OptionKind /* Call | Put | Future */, pub strike: Decimal, pub expiry: NaiveDate,
                 pub side: Side, pub lots: u32, pub lot_size: u32, pub entry_price: Decimal }
pub struct Payoff { pub breakevens: Vec<Decimal>, pub max_profit: Bound, pub max_loss: Bound,
                    pub pop: Option<f64>, pub curve: Vec<(Decimal, Decimal)>, pub warnings: Vec<Warning> }
pub enum Bound { Finite(Decimal), Unbounded }
```

- At-expiry P&L of the whole structure is piecewise linear in the underlying price with kinks at the strikes. Evaluate at 0, every strike and a point beyond the last strike; slopes of the first and last segments decide `Unbounded` profit/loss (net short calls give unbounded loss; net long call exposure gives unbounded profit; net short puts are bounded by price falling to 0). Breakevens are the exact zero crossings of those linear segments (linear interpolation), not read off a sampled grid. `curve` for charting may be sampled.
- Net premium is `sum(signed entry_price * lots * lot_size)`; mixed-expiry structures (calendar, diagonal) have no single expiry payoff: price the long leg with Black-76 at the short leg's expiry using the leg's IV and say the curve is a model estimate.
- Warnings: any short call without a covering long call (naked), any short put without cash/defined cover, ratio structures (net short quantity at a strike), and expiry-day gamma. Margin always comes from the broker margin API via `bz-execution`/`bz-market-data`; if unavailable, show "margin unavailable", never an estimate.
- Charges: an optional net-of-costs payoff calls `CostModel` from `bz-backtest-risk` for entry and exit legs; never duplicate rates here.
- **PoP:** model the terminal price as lognormal with mean equal to the forward: `S_T = F * exp(-sigma^2*T/2 + sigma*sqrt(T)*Z)`. Then `P(S_T > b) = N(d2(b))` with `d2(b) = (ln(F/b) - sigma^2*T/2) / (sigma*sqrt(T))`. The profit region is a union of intervals bounded by the breakevens (use the payoff sign between them); PoP is the sum of interval probabilities. Use ATM IV of the expiry by default (optionally the smile-interpolated IV at each breakeven). Label it "model estimate under market-implied volatility", it ignores fat tails, jumps, early exits and the stop; the probability of touching a stop is a different quantity. Short-premium structures look better than they are (selling carries tail risk), so show max loss in rupees beside PoP.

## 7. Reference values and tests

- Black-76 call, `F=19, K=19, T=0.75, r=0.10, sigma=0.28` is about 1.7011 (Haug's textbook futures-option example; confirm against the book or QuantLib before committing). Textbook values are published to few decimals, so assert them only to the printed precision.
- Black-Scholes `S=42, K=40, r=0.10, sigma=0.20, T=0.5`: call about 4.76, put about 0.81 (Hull's textbook example), same printed-precision rule.
- Exact checks: a grid of (F, K, T, sigma) from QuantLib or `py_vollib_vectorized`, computed offline and committed as `fixtures/reference/options/black76.csv` with tool and version in a README. Tolerances come from `bz-testing`: 1e-8 absolute for closed-form price and Greeks, 1e-6 for the iterative IV solver.
- Properties (`proptest`): put-call parity; price monotone increasing in sigma; deltas in [0,1] / [-1,0] scaled by `D`; gamma and vega non-negative; Greeks equal central finite differences of `price` within tolerance; `implied_vol(price(sigma)) = sigma` across moneyness 0.7 to 1.3, T from 5 minutes to 2 years, sigma 0.05 to 2.0; solver errors (not panics) outside the arbitrage bounds.
- Max pain, PCR, build-up and payoff: hand-built 5 to 7 strike chains in the test (tiny enough to compute by hand), plus an iron condor, a bull call spread, a straddle and a naked short call for breakeven/bound checks. Snapshot one real recorded chain fixture for chain analytics regression.
- Regression: golden IV smile on a recorded chain with `insta`.
