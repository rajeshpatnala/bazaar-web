//! Position sizing, portfolio limits and the `DecisionGate`.
//!
//! Fail closed by type: no `Default` on the snapshot or intent, and every guard input is
//! `Tri::Known` or `Tri::Unknown` (unknown rejects entries).
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
        let default_line = manifest
            .lines()
            .find(|l| l.trim_start().starts_with("default"))
            .unwrap_or_default();
        assert!(!default_line.contains("live-trading"));
    }
}
