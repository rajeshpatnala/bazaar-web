//! Position sizing, portfolio limits and the `DecisionGate`.
//!
//! Fail closed by type: no `Default` on the snapshot or intent, and every guard input is
//! `Tri::Known` or `Tri::Unknown` (unknown rejects entries).
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects
    )
)]

pub mod audit;
pub mod baseline;
pub mod classify;
pub mod config;
pub mod gate;
pub mod money_fmt;
pub mod override_;
pub mod rules;
pub mod sizing;
pub mod snapshot;
pub mod types;
pub mod verdict;

#[cfg(test)]
mod tests {
    #[test]
    fn live_trading_is_not_a_default_feature() {
        let manifest = include_str!("../Cargo.toml");
        let features = manifest
            .split("[features]")
            .nth(1)
            .expect("[features] table present");
        let table = features.split("\n[").next().unwrap_or(features);
        let default_line = table
            .lines()
            .find(|l| l.trim_start().starts_with("default"))
            .expect("default line present");
        assert!(!default_line.contains("live-trading"));
    }
}
