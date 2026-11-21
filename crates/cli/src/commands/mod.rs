//! Offline subcommands, one file each: `import-instruments` and `check-connections`.
//!
//! No network and no secret is touched. Failures are reported as short categories only (no
//! paths, SQL or file contents). Time comes from the `Clock` passed in.

mod check_connections;
mod import_instruments;

use std::path::Path;

use thiserror::Error;

pub use check_connections::{CheckReport, check_connections};
pub use import_instruments::{ImportSummary, import_instruments};

/// File name of the SQLite database inside the data directory.
pub const DB_FILE: &str = "bazaar.sqlite";

/// A failed command, with a safe category label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum CommandError {
    /// The data directory could not be created or secured.
    #[error("data directory unavailable")]
    DataDir,
    /// The input file could not be read.
    #[error("input file unreadable")]
    Unreadable,
    /// The input file is over the size limit.
    #[error("input file too large")]
    TooLarge,
    /// The CSV failed to parse (missing column, too many rows, no header).
    #[error("input could not be parsed")]
    Parse,
    /// The file held no valid row, so nothing was imported.
    #[error("no valid rows")]
    NoValidRows,
    /// The store failed; the label is the `RepoError` category.
    #[error("store error")]
    Store(&'static str),
}

/// Make sure `dir` exists. A directory this call creates (with its new parents) is owner-only
/// (0700 on unix); an existing, user-chosen directory is left exactly as it is.
pub(crate) fn ensure_data_dir(dir: &Path) -> Result<(), CommandError> {
    if dir.is_dir() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(|_| CommandError::DataDir)?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(dir).map_err(|_| CommandError::DataDir)?;
    Ok(())
}

pub(crate) fn repo_label(e: bazaar_core::ports::RepoError) -> CommandError {
    CommandError::Store(e.safe_reason())
}
