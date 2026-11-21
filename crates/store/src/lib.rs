//! SQLite and Parquet/DuckDB repositories implementing the `core` ports.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod approvals;
mod audit;
#[cfg(test)]
mod audit_tests;
mod candles;
mod duck;
mod killswitch;
mod ledger;
mod orders;
mod parquet;
mod sqlite;

pub use approvals::{ApprovalDecision, ApprovalRow};
pub use duck::DuckStore;
pub use killswitch::KillSwitchRow;
pub use orders::{OrderEvent, OrderRow, OrderTransition};
pub use parquet::ParquetStore;
pub use sqlite::SqliteStore;
