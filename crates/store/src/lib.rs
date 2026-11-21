//! SQLite and Parquet/DuckDB repositories implementing the `core` ports.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod candles;
mod sqlite;

pub use sqlite::SqliteStore;
