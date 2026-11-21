//! `check-connections`: SQLite, `DuckDB` and instrument-master freshness.

use std::path::Path;

use bazaar_core::calendar::MarketCalendar;
use bazaar_core::clock::Clock;
use bazaar_core::config::{self, CalendarConfig};
use bazaar_core::ports::InstrumentRepo;
use bazaar_core::segment::Segment;
use bazaar_store::{DuckStore, SqliteStore};

use super::{CANDLES_DIR, CommandError, DB_FILE, ensure_data_dir, repo_label};

/// Result of `check-connections`: the lines to print and whether everything is OK.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckReport {
    pub lines: Vec<String>,
    pub ok: bool,
}

/// Check the SQLite store, the `DuckDB` candle store (engine and Parquet reader load) and the freshness of the instrument master against the calendar's
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
    // The candle store: the engine starts and the Parquet reader loads (no data file is touched).
    let duck = match DuckStore::new(data_dir.join(CANDLES_DIR)) {
        Ok(d) => d.ping().await.map_err(repo_label),
        Err(e) => Err(repo_label(e)),
    };
    lines.push(match duck {
        Ok(()) => "OK duckdb".to_owned(),
        Err(e) => format!("FAILED duckdb {}", error_label(e)),
    });
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
