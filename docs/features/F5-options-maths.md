# F5: Options maths

Status: building. Plan: `.agents/plans/F5-options-maths.md`. Owner area: `crates/options`.
Summary: pure Black-76 pricing, Greeks, IV solver and expiry payoff engine; no I/O or clock.
Decisions: Black-76 on the forward (no dividend model); in-crate normal CDF (series + continued fraction, no dependency); `T` (minutes / 525600, a modelling convention) and `r` are caller inputs; `year_fraction` takes `NaiveDateTime`s that are IST by project convention (never UTC; no type change, owner decision) and validates `TimeParams` (typed `TimeError`), calendar-time convention counts overnight minutes; invalid inputs return `None`, never NaN.
Known limits: European, flat vol, no chain analytics.
Deferred: TA-Lib/QuantLib external cross-checks (owner or tester with the tools). Steps 3-4 (IV, payoff) follow in PR 2.
Not yet met (PR 2): acceptance 3 (implied vol) and 4 (payoff).
