//! SQLite and Parquet/DuckDB repositories implementing the `core` ports.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod anchor;
#[cfg(test)]
mod anchor_tests;
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
mod port_impls;
mod sqlite;

pub use anchor::{AdoptReport, AnchorFault, AnchorStatus};
pub use approvals::{ApprovalDecision, ApprovalRow};
pub use duck::DuckStore;
pub use killswitch::KillSwitchRow;
pub use orders::{OrderEvent, OrderRow, OrderTransition};
pub use parquet::ParquetStore;
pub use sqlite::SqliteStore;
