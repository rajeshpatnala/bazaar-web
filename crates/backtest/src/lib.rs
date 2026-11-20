//! Event-driven backtester, cost model and walk-forward.
//!
//! F9 slice 0: shared types and the `config/backtest.toml` loader. Later slices add the
//! `CostModel`, the engine, metrics and the walk-forward runner in their own modules.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod config;
pub mod types;

pub use config::{BacktestConfig, BacktestConfigError};
pub use types::{
    BacktestError, ChargeBreakdown, EquityPoint, EvidenceLabel, Fill, FillKind, HistoryPolicy, Leg,
    LegSide, Product, Provenance, SlippageClass, Trade,
};
