//! Composition helpers: how the execution store is opened (F10 slice 8).

use std::path::Path;

use bazaar_core::ports::RepoError;
use bazaar_execution::audit::chain_hash;
use bazaar_store::SqliteStore;

/// Open the SQLite store for execution work with the audit chain verified: a tampered log is
/// `Err(RepoError::Corrupt)` and is never appended to. Never use `SqliteStore::open` for the
/// execution store (CR-128-2, SEC-128-2); `tests/compose.rs` fails if that happens.
///
/// # Errors
/// The store's `RepoError`; `Corrupt` when any audit row was altered, removed or reordered.
pub async fn open_store(path: &Path) -> Result<SqliteStore, RepoError> {
    SqliteStore::open_verified(path, chain_hash).await
}
