//! The `Strategy` trait. Long-only, daily-bar NSE cash equity, swing horizon.

use bazaar_core::dataview::DataView;
use bazaar_regime::RegimeFit;

use crate::context::StrategyContext;
use crate::exit::{ExitAction, ExitError, OpenPosition};
use crate::signal::Signal;

/// Static description of a strategy.
#[derive(Debug, Clone, PartialEq)]
pub struct StrategyMeta {
    pub id: &'static str,
    pub name: &'static str,
    pub version: u32,
    /// Fewer closed bars than this: no signal.
    pub min_history_bars: usize,
    /// Declared regime cells, from the strategy's config table (checked at load).
    pub regime_fit: RegimeFit,
    /// Score multiplier for a `Fit::Penalised` cell, from `strategies.toml`; the scanner applies it.
    pub penalty_weight: f64,
}

/// One strategy. Deterministic; no clock, no randomness.
///
/// Both methods see a `DataView` that ends at the closed decision bar. The closed-bar guarantee has
/// two halves: the view cannot reach later bars, and the caller builds the regimes in `ctx` from
/// bars up to that same bar, aligned by session date. An implementation starts with
/// `ctx.check_closed_bar(view)` and returns `None` (logging the reason at `debug`) on `Err`.
pub trait Strategy: Send {
    fn meta(&self) -> &StrategyMeta;

    /// Called once per closed bar. `None` when no setup (or any guard fails); never a panic.
    fn on_bar(&mut self, view: &DataView<'_>, ctx: &StrategyContext<'_>) -> Option<Signal>;

    /// Called once per closed bar for an open position of this strategy.
    ///
    /// # Errors
    /// `StopNotRaised` when the strategy asks to move a long stop down or sideways.
    fn manage(
        &mut self,
        view: &DataView<'_>,
        ctx: &StrategyContext<'_>,
        position: &OpenPosition,
    ) -> Result<ExitAction, ExitError>;
}
