//! SQLite and Parquet/DuckDB repositories implementing the `core` ports.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod candles;
mod duck;
mod parquet;
mod sqlite;

pub use duck::DuckStore;
pub use parquet::ParquetStore;
pub use sqlite::SqliteStore;
