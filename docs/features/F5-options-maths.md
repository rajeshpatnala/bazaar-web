# F5: Options maths

Status: done. Plan: `.agents/plans/F5-options-maths.md`. Owner area: `crates/options`.
Summary: pure Black-76 pricing, Greeks, IV solver and expiry payoff engine; no I/O or clock.
Decisions: Black-76 on the forward (no dividend model); in-crate normal CDF (series + continued fraction, no dependency); `T` (minutes / 525600, a modelling convention) and `r` are caller inputs; `year_fraction` takes `NaiveDateTime`s that are IST by project convention (never UTC; no type change, owner decision) and validates `TimeParams` (typed `TimeError`), calendar-time convention counts overnight minutes; invalid inputs return `None`, never NaN.
Known limits: European, flat vol, no chain analytics.
Deferred: TA-Lib/QuantLib external cross-checks (owner or tester with the tools).
Acceptance 3 (implied vol: bracketed Newton + bisection, typed `IvError`, capped iterations, relative price tolerance) and 4 (expiry payoff from kinks, exact `Decimal` breakevens; a naked short put is bounded, net short calls flag `unbounded_loss`) are met in PR 2. Payoff uses plain `Decimal` (not `Price`/`Money`) to stay free of core money newtypes.
IV solver stops on price or bracket width, then checks `noise/vega <= vol_accuracy` and returns `IllConditioned` where vega is too small (deep ITM, very short T): never a wrong number. Payoff: `has_naked_short` is informational (per-kind short qty > long qty, no strike check); the risk crate and scanner own the defined-risk classification and margin. Leg cap 64; breakevens unrounded; zero-P&L plateaus list only their ends.
