# F8: Strategy trait, Signal and the first three strategies

Status: building (slice 0 of 6 done: contracts). Not investment advice.

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

## Known limits
Fixtures prove rules not profit; every strategy is "Low evidence" until F9 plus F3 data; no shorts, options or intraday.

## What was built
Slice 0: `DataView`, `Signal` + `ExitPlan` + `validate`, `ExitAction`/`OpenPosition`, `StrategyContext`/`PortfolioView`, `Strategy`/`StrategyMeta`, `ParamSet`/`parse_params`/`params_hash`, `StrategyFactory` + `build_enabled`, stubs for later modules, three disabled `[strategy.*]` tables.

## Deferred
First evidence run (F9, owner-run F3 fetch); catalog rows and `bz-quant` skill-text amendments (manager); strategies, registry and cross-strategy tests (slices 1-5).
