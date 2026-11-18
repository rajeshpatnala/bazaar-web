# 4. Options math (summary; formulas in the reference)

(From `bz-quant` SKILL.md; open when the task touches this area.)


- `price(kind, F, K, T, r, sigma) -> f64` (Black-76), `greeks(...) -> Greeks { delta, gamma, vega, theta, rho }`, `implied_vol(kind, price, F, K, T, r) -> Result<f64, IvError>`.
- Forward `F`: use the same-expiry future's price; if absent derive from put-call parity at the strike nearest spot. `T` in years from the valuation instant to the expiry instant, which is `calendar.close_time(expiry_date, segment, kind)` (`bz-india-markets`; never a literal clock time, because session ends change by circular), using minutes, floored at a small epsilon; `r` from config with `verified_on`. Options test fixtures carry the fixture calendar's close time.
- Use **mid** of bid/ask for IV; fall back to LTP only when the book is empty and mark the result `stale`. Compute IV from the OTM side of each strike.
- Solver returns typed errors (`BelowIntrinsic`, `AboveUpperBound`, `ExpiredOrZeroT`, `NoConvergence`); the chain shows "n/a", never a made-up number.
- Chain analytics: IV smile, IV rank and percentile (needs a stored daily ATM-IV or India VIX history; fewer than ~250 observations flags `insufficient_history`). **Structure selection uses IV rank bands** (`bz-market-strategy` section 6); IV percentile is displayed as context only, never as the selection input. Also PCR by OI and by volume, max pain, OI walls, change-in-OI, futures OI build-up, basis and rollover %.
- Payoff engine: `Strategy legs -> Payoff { curve, breakevens, max_profit, max_loss, unbounded flags, pop }` at expiry, exact breakevens from the piecewise-linear structure (not a sampled grid), premiums as `Decimal`. Optional net-of-charges variant calls the `CostModel` from `bz-backtest-risk`.
