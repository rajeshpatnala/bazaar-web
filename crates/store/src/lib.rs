//! SQLite and Parquet/DuckDB repositories implementing the `core` ports.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod approvals;
mod candles;
mod duck;
mod killswitch;
mod orders;
mod parquet;
mod sqlite;

pub use approvals::{ApprovalDecision, ApprovalRow};
pub use duck::DuckStore;
pub use killswitch::KillSwitchRow;
pub use orders::{OrderEvent, OrderRow};
pub use parquet::ParquetStore;
pub use sqlite::SqliteStore;
