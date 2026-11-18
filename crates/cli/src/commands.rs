//! Offline subcommands: `import-instruments` and `check-connections`.
//!
//! No network and no secret is touched. Failures are reported as short categories only (no
//! paths, SQL or file contents). Time comes from the `Clock` passed in.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use bazaar_core::calendar::MarketCalendar;
use bazaar_core::clock::Clock;
use bazaar_core::config::{self, CalendarConfig};
use bazaar_core::ports::InstrumentRepo;
use bazaar_core::segment::Segment;
use bazaar_data::instruments::{ParseError, ParseLimits, parse_instruments};
use bazaar_store::SqliteStore;
use thiserror::Error;

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

/// Outcome of one import, for printing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportSummary {
    pub accepted: u64,
    pub rejected: u64,
    pub written: u64,
}

/// Create `dir` (and parents) owner-only (0700 on unix) and tighten an existing one.
fn ensure_data_dir(dir: &Path) -> Result<(), CommandError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(|_| CommandError::DataDir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| CommandError::DataDir)?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(dir).map_err(|_| CommandError::DataDir)?;
    Ok(())
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

fn repo_label(e: bazaar_core::ports::RepoError) -> CommandError {
    CommandError::Store(e.safe_reason())
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

/// Result of `check-connections`: the lines to print and whether everything is OK.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckReport {
    pub lines: Vec<String>,
    pub ok: bool,
}

/// Check the SQLite store and the freshness of the instrument master against the calendar's
/// last trading day on or before today. A placeholder or uncovered calendar is never fresh.
pub async fn check_connections(
    config_dir: &Path,
    data_dir: &Path,
    clock: &dyn Clock,
) -> CheckReport {
    let mut lines = Vec::new();
    let store = match ensure_data_dir(data_dir) {
        Ok(()) => SqliteStore::open(&data_dir.join(DB_FILE))
            .await
            .map_err(repo_label),
        Err(e) => Err(e),
    };
    let store = match store {
        Ok(s) if s.ping().await.is_ok() => {
            lines.push("OK sqlite".to_owned());
            Some(s)
        }
        Ok(_) => {
            lines.push("FAILED sqlite unavailable".to_owned());
            None
        }
        Err(e) => {
            lines.push(format!("FAILED sqlite {}", error_label(e)));
            None
        }
    };
    let instruments = match store {
        Some(s) => instruments_state(&s, config_dir, clock).await,
        None => Err("no_store"),
    };
    lines.push(match instruments {
        Ok(()) => "OK instruments".to_owned(),
        Err(cat) => format!("FAILED instruments {cat}"),
    });
    let ok = lines.iter().all(|l| l.starts_with("OK"));
    CheckReport { lines, ok }
}

fn error_label(e: CommandError) -> &'static str {
    match e {
        CommandError::Store(c) => c,
        _ => "unavailable",
    }
}

async fn instruments_state(
    store: &SqliteStore,
    config_dir: &Path,
    clock: &dyn Clock,
) -> Result<(), &'static str> {
    let cfg: CalendarConfig = config::load(config_dir).map_err(|_| "calendar_unreadable")?;
    if cfg.placeholder {
        return Err("calendar_placeholder");
    }
    let calendar = MarketCalendar::from_config(&cfg);
    let today = clock.now_ist().date_naive();
    let tomorrow = today.succ_opt().ok_or("calendar_out_of_range")?;
    let last_trading = calendar
        .prev_trading_day(tomorrow, Segment::Equity)
        .map_err(|_| "calendar_out_of_range")?;
    match store
        .last_fetched_on()
        .await
        .map_err(bazaar_core::ports::RepoError::safe_reason)?
    {
        None => Err("empty"),
        Some(fetched) if fetched >= last_trading => Ok(()),
        Some(_) => Err("stale"),
    }
}
