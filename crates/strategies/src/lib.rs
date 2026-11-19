//! `Strategy` trait and one module per strategy.
//!
//! Long-only, daily-bar NSE cash equity, swing horizon. A strategy emits a `Signal`, which is not a
//! recommendation and not advice. Spec: `docs/strategy/F8-first-strategies.md`. Slice 0 holds the
//! contracts; the strategy modules are stubs until their slices.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

pub mod bollinger_reversion;
pub mod common;
pub mod context;
pub mod donchian_breakout;
pub mod exit;
pub mod factory;
pub mod params;
pub mod registry;
pub mod signal;
pub mod strategy;
#[cfg(test)]
pub mod testkit;
pub mod trend_pullback;

pub use context::{ContextError, PortfolioView, StrategyContext};
pub use exit::{ExitAction, ExitError, OpenPosition};
pub use factory::{BuildInput, StrategyFactory, build_enabled, meta_from};
pub use params::{
    ParamError, ParamSet, RegimeFitConfig, StrategyConfig, params_hash, parse_params,
};
pub use signal::{ExitPlan, Signal, SignalError, risk_per_share};
pub use strategy::{Strategy, StrategyMeta};
