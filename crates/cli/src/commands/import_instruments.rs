//! `import-instruments`: load a Kite-format instruments CSV into the SQLite store.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use bazaar_core::clock::Clock;
use bazaar_core::ports::InstrumentRepo;
use bazaar_data::instruments::{ParseError, ParseLimits, parse_instruments};
use bazaar_store::SqliteStore;

use super::{CommandError, DB_FILE, ensure_data_dir, repo_label};

/// Outcome of one import, for printing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportSummary {
    pub accepted: u64,
    pub rejected: u64,
    pub written: u64,
}

/// Read `path` without ever holding more than `max_bytes + 1` bytes: the metadata length is
/// checked first, then the read itself is capped (the file may grow or lie about its size).
fn read_bounded(path: &Path, max_bytes: usize) -> Result<Vec<u8>, CommandError> {
    let limit = u64::try_from(max_bytes).map_err(|_| CommandError::TooLarge)?;
    let file = File::open(path).map_err(|_| CommandError::Unreadable)?;
    let meta = file.metadata().map_err(|_| CommandError::Unreadable)?;
    if !meta.is_file() {
        return Err(CommandError::Unreadable);
    }
    if meta.len() > limit {
        return Err(CommandError::TooLarge);
    }
    let mut buf = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut buf)
        .map_err(|_| CommandError::Unreadable)?;
    if buf.len() > max_bytes {
        return Err(CommandError::TooLarge);
    }
    Ok(buf)
}

/// Import a Kite-format instruments CSV into `<data_dir>/bazaar.sqlite`, dated today (IST).
///
/// The batch is bounded twice: the file read by `limits.max_bytes` and the row count by
/// `limits.max_rows` (checked again here before the single store transaction).
pub async fn import_instruments(
    data_dir: &Path,
    file: &Path,
    clock: &dyn Clock,
    limits: &ParseLimits,
) -> Result<ImportSummary, CommandError> {
    let bytes = read_bounded(file, limits.max_bytes)?;
    let parsed = parse_instruments(&bytes, limits).map_err(|e| match e {
        ParseError::TooLarge { .. } => CommandError::TooLarge,
        _ => CommandError::Parse,
    })?;
    if parsed.instruments.is_empty() {
        return Err(CommandError::NoValidRows);
    }
    // Defence in depth: the parser enforces the same cap; this guards the single store transaction.
    if parsed.instruments.len() > limits.max_rows {
        return Err(CommandError::Parse);
    }
    ensure_data_dir(data_dir)?;
    let store = SqliteStore::open(&data_dir.join(DB_FILE))
        .await
        .map_err(repo_label)?;
    let fetched_on = clock.now_ist().date_naive();
    let summary = store
        .upsert_instruments(&parsed.instruments, fetched_on)
        .await
        .map_err(repo_label)?;
    Ok(ImportSummary {
        accepted: parsed.report.accepted,
        rejected: parsed.report.rejected,
        written: summary.written,
    })
}
