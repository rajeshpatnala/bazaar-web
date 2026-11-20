//! Event-driven backtester, cost model and walk-forward.
//!
//! F9 slice 0: shared types and the `config/backtest.toml` loader. Later slices add the
//! `CostModel`, the engine, metrics and the walk-forward runner in their own modules.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod config;
pub mod cost;
pub mod fill;
pub mod ledger;
pub mod types;

pub use config::{BacktestConfig, BacktestConfigError};
pub use cost::{CostModel, DpLedger};
pub use types::{
    BacktestError, ChargeBreakdown, EquityPoint, EvidenceLabel, Fill, FillKind, HistoryPolicy, Leg,
    LegInstrument, LegSide, OptionRight, Product, Provenance, SlippageClass, Trade,
};
