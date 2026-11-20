# F8: Strategy trait, Signal and the first three strategies

Status: built, in review (slices 0-5); evidence pending (F9). Not investment advice.

Summary: three long-only daily swing strategies behind one `Strategy` trait, shipped disabled, evidence pending.
Owner area: `crates/strategies`, `crates/core/src/dataview.rs`, `config/strategies.toml` `[strategy.*]`.
Spec: `docs/strategy/F8-first-strategies.md`.

## Decisions
- Long-only cash equity, owner 2026-11-20; UNVERIFIED defaults accepted, never tuned on P&L.
- `PortfolioView` lives in `strategies` (not `risk::PortfolioSnapshot`; architecture rule). `StrategyContext` has `market_regime` and `stock_regime` and no option chain (no chain type exists; add with the first option strategy).
- `Strategy::manage` returns `Result<ExitAction, ExitError>`; a long stop may only move up (`ExitAction::tighten`).
- Regime cells are written in `[strategy.<id>.regime_fit]` as `"trend:vol"`; `RegimeFit::check` runs at config load (`StrategyConfig::regime_fit`). `penalty_weight` (score multiplier for a `Penalised` cell, UNVERIFIED) is a key of each strategy table.
- The closed-bar guarantee is the caller's: regimes in the context are built from bars up to the decision bar; `StrategyContext::check_closed_bar` rejects a regime `as_of` after the last close.
- New dependency for `strategies`: `sha2` (already in the workspace) for `params_hash`.
- Donchian uses a fixed 2R reference target (const, spec section 3); trend_pullback keeps `target_r` as a param. Intentional, revisit if F9 evidence shows target choice matters.
- trend_pullback v1 holds a fixed stop; the spec wording is aligned.
- Stock-regime condition is stated in the Setup sentence; 'Market today' always shows the market label.

## Known limits
Fixtures prove rules not profit; every strategy is "Low evidence" until F9 plus F3 data; no shorts, options or intraday.

## What was built
Slice 0: `DataView`, `Signal` + `ExitPlan` + `validate`, `ExitAction`/`OpenPosition`, `StrategyContext`/`PortfolioView`, `Strategy`/`StrategyMeta`, `ParamSet`/`parse_params`/`params_hash`, `StrategyFactory` + `build_enabled`, stubs for later modules, three disabled `[strategy.*]` tables.

Slices 1-4: `common.rs` helpers (guards, entry zone, tick rounding, stop and rr checks, why-text template, `WHY_TEXT_LABEL`), `donchian_breakout`, `trend_pullback`, `bollinger_reversion`, each with fixtures in `fixtures/strategies/` and a hand-computed bar.
Slice 5: `registry::{factories, build_all}` (the three factories, enabled ones built from `config/strategies.toml`, unknown names, unknown param keys and out-of-range values are load errors) and cross-strategy tests in `crates/strategies/tests/` (`lookahead`, `regime_fit`, `why_text`, `config`, shared `common/`) that build every strategy through the shipped config. Every strategy ships `enabled = false`. `trend_pullback::manage` returns `ExitError::Context` for unadjusted or non-finite candles, and holds only while the 50-day average is warming up.

## Deferred
First evidence run (F9, owner-run F3 fetch); catalog rows and `bz-quant` skill-text amendments (manager).

Deferred from PR 77: QUAN-77-8 (stop fills are intraday with gap handling, `min(stop, open)`) to F9 and the tech lead; the `trend_pullback` `target_r` param style vs the `donchian_breakout` const is a tech-lead call (QUAN-77-5).

## Review round 1 (PR 77, donchian_breakout)
- `manage` needs no regime; it checks adjusted candles and the tick and returns `ExitError::Context` (never a silent Hold). `WHY_TEXT_LABEL` lives in `common.rs`. Added a `target_basis` fact. QUAN-77-9 disputed (no F&O flag in the instrument master; the const `SESSION_KIND` is documented).

## Review round 1 (PR 74)
- `StrategyContext` now carries `calendar: &MarketCalendar`; `next_session_close` gives `valid_until` (no date arithmetic in strategies). `check_closed_bar` rejects a regime from a later or an earlier session.
- Config tables gained `entry_zone_atr_frac` (0.25, a PROPOSAL: the spec names the key but gives no value; owner to confirm), `atr_period`, and the spec-fixed averages (`volume_sma_period`, `ema_pullback`, `ema_trend`). Strategy tests assert `Default == file`.
- `params_hash(id, table)` hashes the whole strategy table except `enabled` and returns `Result`. `market_trends` may not include `down`. `Signal` has `instrument_token` and checks target, recomputed `reference_rr`, time stop and provenance.
- Limit: `StrategyConfig::from_table` does not range-check `params` (types live in slices 2-4); each strategy's shipped-config test must cover its disabled table.
- Deferred (QUAN-74-10): F9 slice 0 adds `OpenPosition` fields (initial stop, quantity, strategy id) when it re-reads the types.

## Deferred (later rounds)
- Donchian stop fills: the F9 backtest must model intraday stop touches filled at min(stop, open), because a close-only model overstates the 10-day-low trail (QUAN-77-8).
- Worst-case `reward_risk` convention in `check_rr` (QUAN-79-1) is a separate fix PR (F8-rr) before F11; reward_risk is currently measured from the bottom of the entry zone, so optimistic.
- Worst-case reward_risk in check_rr (QUAN-79-1, QUAN-80-1) is a separate fix PR (F8-rr) before F11. No strategy may be enabled before it lands; the shipped config and registry doc say so.
- trend_pullback and bollinger: EMA/ATR/SMA are recomputed over the whole view on every bar (O(n) per bar); F9 should add streaming state inside the strategy (CR-76-3, QUAN-76-7).
- Optional QUAN-76-8: round the f64-converted level to 4 dp before tick snapping (direction is already conservative).
