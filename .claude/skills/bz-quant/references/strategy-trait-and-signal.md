# 6. Strategy trait and signal

(From `bz-quant` SKILL.md; open when the task touches this area.)


```rust
use bazaar_core::{Candle, DataView};          // DataView design and ownership: bz-backtest-risk section 3.1

pub trait Strategy: Send {
    fn meta(&self) -> &StrategyMeta;          // id, name, horizon, segments, regime_fit, min_history_bars
    /// Called once per completed bar. `view` ends at the closed bar; there is no way to reach later bars.
    /// Use `view.last()` (an Option, no `expect`) for the decision bar and `view.lookback(n)` for history.
    fn on_bar(&mut self, view: &DataView, ctx: &StrategyContext) -> Option<Signal>;
}

/// Same type for the scanner and the backtest engine. "Now" is `view.last().close_time`; no clock field.
pub struct StrategyContext<'a> { pub instrument: &'a Instrument, pub regime: &'a RegimeState,
                                 pub chain: Option<&'a OptionChain>, pub portfolio: &'a PortfolioSnapshot }

pub struct Signal {
    pub strategy_id: &'static str,
    pub side: Side,                           // Buy | Sell (options: legs in `structure`)
    pub entry_low: Decimal, pub entry_high: Decimal,
    pub stop: Decimal, pub targets: Vec<Decimal>,
    pub horizon: Horizon,                     // Intraday | Swing | Positional | LongTerm
    pub structure: Option<OptionStructure>,
    pub facts: Vec<Fact>,                     // evidence for the Why panel: label + value + plain-English meaning
    pub valid_until: DateTime<Tz>,            // entry zone expiry, e.g. `calendar.close_time(..)` for intraday
}

pub trait StrategyFactory { fn id(&self) -> &'static str;
    fn build(&self, params: &toml::Table) -> Result<Box<dyn Strategy>, ParamError>; }
```

Params live in a `serde` struct with `Default`, `validate()` and ranges (also used by the optimiser in `bz-backtest-risk`). Registry maps id to factory. `config/strategies.toml` (this skill owns its schema) starts with the standard header (`schema_version`, `verified_on`, `source = "owner"`), then one `[strategy.<id>]` table per strategy with `enabled = false` by default and a `[strategy.<id>.params]` sub-table whose keys are exactly the params struct fields (unknown key is a load error), plus a `[scanner]` table for score weights, score floor, `min_trades_evidence` and the low-evidence confidence cap. A signal with `stop` on the wrong side of entry or `targets` that fail reward:risk minimum is dropped inside the strategy and logged at `debug`.
