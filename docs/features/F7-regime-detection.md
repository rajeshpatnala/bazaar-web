# F7: Regime detection

Status: building (step 1 of 3: config and types). Plan: `.agents/plans/F7-regime-detection.md`. Spec: `docs/strategy/F7-regime-definitions.md`. Owner area: `crates/regime`, `config/strategies.toml [regime]`.
Summary: label the market and a stock as Up/Range/Down/Mixed/Unknown with a calm/normal/wild swing word, from daily bars. Pure `classify(candles, params, prev)`; two runs (index = market regime, own series = stock regime).
Decisions: every default is UNVERIFIED, accepted by the owner 2026-11-20, revisited after F9 shows state frequencies, never tuned on P&L; config lives in `config/strategies.toml` (`[regime]`, later `[strategy.<id>]`), not `risk.toml`; core keeps the tables raw (`toml::Table`) and `RegimeParams::from_config` validates; load bounds are sanity limits, wider than the spec's stability-sweep ranges.
Known limits: lagging (EMA/ADX plus `confirm_bars`); no VIX or breadth until F3; daily bars only; Mixed may be common (accepted: fewer signals). Spec says warm-up is "about 270 bars"; the formula `ema_slow + vol_lookback_bars + confirm_bars` gives 453 at defaults, and the code follows the formula.
Deferred: state-frequency report by regime cell (F9); `bz-quant` section 5 skill text (two states per decision, market + stock) is for the manager to update.
Step status: 1 config + types (this PR); 2 `classify`; 3 `RegimeFit`, labels, property tests.
