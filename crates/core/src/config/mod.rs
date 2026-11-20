//! Config loading and typed config files, shared by cli, server and tests.
//!
//! Every file starts with `schema_version`, `verified_on` and `source`. Every struct rejects
//! unknown keys; money and percentages are TOML strings (a TOML float is a parse error).
//! The loader reads files only: no environment, no network, no clock (callers pass `today`).

mod calendar;
mod charges;
mod error;
mod files;
mod risk;

use std::path::Path;

use chrono::{Datelike, NaiveDate};

pub use calendar::{
    AppliesTo, CalendarConfig, HolidayKind, HolidayRow, Phase, SessionRow, SpecialKind,
    SpecialSessionRow,
};
pub use charges::{
    Brokerage, ChargeExchange, ChargeSegment, ChargeSide, ChargesConfig, ChargesMeta, DpCharge,
    GstRate, HistoryPolicy, MAX_RATE_SETS, NoRateSet, OptionExercise, RateSet, TurnoverBasis,
};
pub use error::ConfigError;
pub use files::{
    ComplianceConfig, DataConfig, Event, EventKind, EventsConfig, MAX_EVENTS,
    MAX_SYMBOLS_PER_WATCHLIST, MAX_TEXT_CHARS, MAX_WATCHLISTS, ScannerConfig, ScannerWeights,
    StrategiesConfig, Watchlist, WatchlistsConfig,
};
pub use risk::{
    Behaviour, CooloffUntil, ExecutionSection, Exposure, HardLimits, Limits, ModeSection,
    OnDailyLoss, RiskConfig,
};

/// Money and percentages are TOML strings. `rust_decimal::serde::str` also accepts TOML
/// numbers, so this strict version reads a string only (a float never becomes money).
mod decimal_str {
    use std::str::FromStr;

    use rust_decimal::Decimal;
    use serde::Deserialize;
    use serde::de::{Deserializer, Error};

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Decimal, D::Error> {
        let s = String::deserialize(d)?;
        Decimal::from_str(&s).map_err(|_| D::Error::custom("expected a decimal string"))
    }
}

/// The only `schema_version` this loader understands.
pub const SCHEMA_VERSION: u32 = 1;

/// The three keys every config file starts with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub schema_version: u32,
    /// Date a human compared the file with its sources. `None` only for a placeholder.
    pub verified_on: Option<NaiveDate>,
    /// The file holds no data yet; it is never reported as fresh.
    pub placeholder: bool,
    /// URL, circular number or `owner`.
    pub source: String,
}

impl Header {
    fn check(&self, file: &'static str) -> Result<(), ConfigError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ConfigError::invalid(
                file,
                "schema_version",
                format!("unsupported version {}", self.schema_version),
            ));
        }
        if self.source.trim().is_empty() {
            return Err(ConfigError::invalid(file, "source", "must not be empty"));
        }
        match (self.placeholder, self.verified_on) {
            (true, Some(_)) => Err(ConfigError::invalid(
                file,
                "verified_on",
                "a placeholder file must not claim verification",
            )),
            (false, None) => Err(ConfigError::invalid(file, "verified_on", "is required")),
            _ => Ok(()),
        }
    }
}

/// One typed config file. Implemented by the eight structs.
pub trait ConfigFile: serde::de::DeserializeOwned {
    /// File name inside the config directory.
    const FILE: &'static str;
    /// The shared header (explicit fields per struct, because `flatten` and
    /// `deny_unknown_fields` do not mix).
    fn header(&self) -> Header;
    /// Range and consistency checks; returns the first failing key path.
    fn validate(&self) -> Result<(), ConfigError>;
}

/// Implements `header()` for a struct with the three header fields.
macro_rules! impl_header {
    () => {
        fn header(&self) -> $crate::config::Header {
            $crate::config::Header {
                schema_version: self.schema_version,
                verified_on: self.verified_on,
                placeholder: self.placeholder,
                source: self.source.clone(),
            }
        }
    };
}
pub(crate) use impl_header;

/// Parse and validate one file from text (the pure half of [`load`]).
pub fn parse_str<T: ConfigFile>(text: &str) -> Result<T, ConfigError> {
    let value: T = toml::from_str(text).map_err(|e| ConfigError::Parse {
        file: T::FILE,
        line: e.span().map(|s| {
            text.get(..s.start)
                .unwrap_or_default()
                .matches('\n')
                .count()
                + 1
        }),
        message: e.message().to_owned(),
    })?;
    value.header().check(T::FILE)?;
    value.validate()?;
    Ok(value)
}

/// Read, parse and validate `dir/<T::FILE>`.
pub fn load<T: ConfigFile>(dir: &Path) -> Result<T, ConfigError> {
    let text = read_capped(&dir.join(T::FILE), MAX_CONFIG_BYTES).map_err(|e| match e {
        ReadError::Io(source) => ConfigError::Io {
            file: T::FILE,
            source,
        },
        ReadError::TooLarge => ConfigError::invalid(T::FILE, "file", "larger than the size cap"),
    })?;
    parse_str(&text)
}

/// Largest config file the loader reads (1 MiB).
pub const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

enum ReadError {
    Io(std::io::Error),
    TooLarge,
}

/// Read a regular file as UTF-8, refusing more than `max` bytes (no unbounded read from a FIFO
/// or a device file).
fn read_capped(path: &Path, max: u64) -> Result<String, ReadError> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(ReadError::Io)?;
    if !file.metadata().map_err(ReadError::Io)?.is_file() {
        return Err(ReadError::Io(std::io::ErrorKind::InvalidInput.into()));
    }
    let mut text = String::new();
    file.take(max + 1)
        .read_to_string(&mut text)
        .map_err(ReadError::Io)?;
    if text.len() as u64 > max {
        return Err(ReadError::TooLarge);
    }
    Ok(text)
}

/// How old `verified_on` may get before a file is reported stale. The limits are policy chosen
/// by the caller (cli/server), not constants of this crate.
#[derive(Debug, Clone)]
pub struct StalenessPolicy {
    pub default_max_age_days: i64,
    /// Per-file overrides, e.g. a shorter limit for `charges.toml`.
    pub overrides: Vec<(&'static str, i64)>,
}

impl StalenessPolicy {
    fn limit(&self, file: &str) -> i64 {
        self.overrides
            .iter()
            .find(|(f, _)| *f == file)
            .map_or(self.default_max_age_days, |(_, d)| *d)
    }
}

/// Why a file is reported stale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaleReason {
    /// `verified_on` is older than the limit.
    TooOld { age_days: i64, max_age_days: i64 },
    /// The file is a placeholder: not yet populated, never fresh.
    Placeholder,
    /// `verified_on` is after `today`.
    FutureDated,
    /// `calendar.toml` does not cover the current year.
    YearNotCovered { year: i32 },
}

/// A staleness finding; shown in the UI and by `check-connections`, never a silent pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleWarning {
    pub file: &'static str,
    pub reason: StaleReason,
}

/// All eight config files.
#[derive(Debug, Clone)]
pub struct Config {
    pub risk: RiskConfig,
    pub charges: ChargesConfig,
    pub strategies: StrategiesConfig,
    pub watchlists: WatchlistsConfig,
    pub events: EventsConfig,
    pub calendar: CalendarConfig,
    pub data: DataConfig,
    pub compliance: ComplianceConfig,
}

impl Config {
    /// Load and validate every file in `dir`; the first error wins.
    pub fn load(dir: &Path) -> Result<Self, ConfigError> {
        Ok(Self {
            risk: load(dir)?,
            charges: load(dir)?,
            strategies: load(dir)?,
            watchlists: load(dir)?,
            events: load(dir)?,
            calendar: load(dir)?,
            data: load(dir)?,
            compliance: load(dir)?,
        })
    }

    /// Staleness of every file against `today` (from the `Clock`, never the system).
    #[must_use]
    pub fn stale_warnings(&self, today: NaiveDate, policy: &StalenessPolicy) -> Vec<StaleWarning> {
        let headers = [
            (RiskConfig::FILE, self.risk.header()),
            (ChargesConfig::FILE, self.charges.header()),
            (StrategiesConfig::FILE, self.strategies.header()),
            (WatchlistsConfig::FILE, self.watchlists.header()),
            (EventsConfig::FILE, self.events.header()),
            (CalendarConfig::FILE, self.calendar.header()),
            (DataConfig::FILE, self.data.header()),
            (ComplianceConfig::FILE, self.compliance.header()),
        ];
        let mut out = Vec::new();
        for (file, header) in headers {
            let max_age_days = policy.limit(file);
            let age_days = header.verified_on.map_or(0, |v| (today - v).num_days());
            let reason = if header.placeholder {
                Some(StaleReason::Placeholder)
            } else if age_days < 0 {
                Some(StaleReason::FutureDated)
            } else if age_days > max_age_days {
                Some(StaleReason::TooOld {
                    age_days,
                    max_age_days,
                })
            } else {
                None
            };
            if let Some(reason) = reason {
                out.push(StaleWarning { file, reason });
            }
        }
        if !self.calendar.placeholder && !self.calendar.covered_years.contains(&today.year()) {
            out.push(StaleWarning {
                file: CalendarConfig::FILE,
                reason: StaleReason::YearNotCovered { year: today.year() },
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHIPPED: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../config");
    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/config");

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn loads_shipped_config() {
        let cfg = Config::load(Path::new(SHIPPED)).unwrap();
        assert_eq!(
            cfg.risk.mode.execution_mode,
            crate::mode::ExecutionMode::Paper
        );
        assert_eq!(cfg.risk.header().source, "owner");
        assert!(!cfg.calendar.placeholder);
        assert_eq!(cfg.calendar.covered_years, vec![2026]);
    }

    #[test]
    fn loads_fixture_config() {
        let cfg = Config::load(Path::new(FIXTURES)).unwrap();
        assert_eq!(cfg.calendar.covered_years, vec![2026]);
        assert_eq!(cfg.watchlists.watchlist.len(), 1);
        assert_eq!(cfg.events.event.len(), 1);
    }

    #[test]
    fn missing_file_is_an_io_error_naming_the_file() {
        let err = load::<RiskConfig>(Path::new("/nonexistent-bazaar-dir")).unwrap_err();
        assert!(matches!(
            err,
            ConfigError::Io {
                file: "risk.toml",
                ..
            }
        ));
        assert!(err.to_string().starts_with("risk.toml: cannot read file"));
    }

    #[test]
    fn unknown_key_is_a_parse_error_with_line() {
        let text = "schema_version = 1\nverified_on = \"2026-01-01\"\nsource = \"x\"\nextra = 1\n";
        let err = parse_str::<DataConfig>(text).unwrap_err();
        assert!(matches!(
            err,
            ConfigError::Parse {
                file: "data.toml",
                line: Some(4),
                ..
            }
        ));
    }

    #[test]
    fn header_errors() {
        let bad_version = "schema_version = 2\nverified_on = \"2026-01-01\"\nsource = \"x\"\n";
        let err = parse_str::<DataConfig>(bad_version).unwrap_err();
        assert!(err.to_string().contains("schema_version"));
        let blank = "schema_version = 1\nverified_on = \"2026-01-01\"\nsource = \"  \"\n";
        let err = parse_str::<DataConfig>(blank).unwrap_err();
        assert!(err.to_string().contains("source"));
        let missing = "schema_version = 1\nsource = \"x\"\n";
        assert!(matches!(
            parse_str::<DataConfig>(missing),
            Err(ConfigError::Invalid { .. })
        ));
        let bad_date = "schema_version = 1\nverified_on = \"2026-13-45\"\nsource = \"x\"\n";
        assert!(parse_str::<DataConfig>(bad_date).is_err());
    }

    #[test]
    fn staleness_age_future_placeholder_and_uncovered_year() {
        // fixtures: risk, watchlists, events, calendar verified 2026-01-05 (covers 2026);
        // strategies, data, compliance are placeholders (charges is synthetic, verified too).
        let cfg = Config::load(Path::new(FIXTURES)).unwrap();
        let policy = StalenessPolicy {
            default_max_age_days: 30,
            overrides: vec![("risk.toml", 90)],
        };
        let placeholders = |w: &[StaleWarning]| {
            w.iter()
                .filter(|x| x.reason == StaleReason::Placeholder)
                .count()
        };
        let w = cfg.stale_warnings(d(2026, 1, 20), &policy);
        assert_eq!(w.len(), 3);
        assert_eq!(placeholders(&w), 3);
        let w = cfg.stale_warnings(d(2026, 3, 1), &policy); // 55 days: 3 over 30, risk ok
        assert_eq!(w.len(), 7);
        assert!(w.iter().all(|x| x.file != "risk.toml"));
        assert!(w.contains(&StaleWarning {
            file: "events.toml",
            reason: StaleReason::TooOld {
                age_days: 55,
                max_age_days: 30
            },
        }));
        let w = cfg.stale_warnings(d(2026, 1, 1), &policy);
        assert_eq!(w.len(), 8);
        assert_eq!(placeholders(&w), 3);
        assert!(w.contains(&StaleWarning {
            file: "risk.toml",
            reason: StaleReason::FutureDated
        }));
        let w = cfg.stale_warnings(d(2027, 1, 6), &policy);
        assert!(
            w.iter()
                .any(|x| x.reason == StaleReason::YearNotCovered { year: 2027 })
        );
    }

    #[test]
    fn shipped_placeholders_are_never_fresh() {
        let cfg = Config::load(Path::new(SHIPPED)).unwrap();
        let policy = StalenessPolicy {
            default_max_age_days: 100_000,
            overrides: vec![],
        };
        let w = cfg.stale_warnings(d(2026, 11, 20), &policy);
        let files: Vec<_> = w.iter().map(|x| x.file).collect();
        for f in ["data.toml", "compliance.toml", "events.toml"] {
            assert!(files.contains(&f), "{f}");
        }
        assert!(!files.contains(&"risk.toml") && !files.contains(&"watchlists.toml"));
        // calendar.toml, charges.toml and strategies.toml are real (verified 2026-11-20).
        assert!(!files.contains(&"calendar.toml") && !files.contains(&"charges.toml"));
        assert!(!files.contains(&"strategies.toml"));
        assert!(!cfg.charges.placeholder && cfg.charges.verified_on.is_some());
    }

    #[test]
    fn placeholder_header_rules() {
        let ph = "schema_version = 1\nplaceholder = true\nsource = \"x\"\n";
        assert!(parse_str::<DataConfig>(ph).is_ok());
        let both = format!("{ph}verified_on = \"2026-01-01\"\n");
        assert!(
            parse_str::<DataConfig>(&both)
                .unwrap_err()
                .to_string()
                .contains("must not claim verification")
        );
        let neither = "schema_version = 1\nsource = \"x\"\n";
        assert!(
            parse_str::<DataConfig>(neither)
                .unwrap_err()
                .to_string()
                .contains("verified_on: is required")
        );
    }

    #[test]
    fn oversized_and_non_regular_files_are_refused() {
        let dir = std::env::temp_dir().join(format!("bazaar-cfg-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("big.toml");
        std::fs::write(&file, "0123456789").unwrap();
        assert_eq!(read_capped(&file, 10).ok().as_deref(), Some("0123456789"));
        assert!(matches!(read_capped(&file, 9), Err(ReadError::TooLarge)));
        assert!(matches!(read_capped(&dir, 100), Err(ReadError::Io(_))));
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(MAX_CONFIG_BYTES, 1_048_576);
    }
}
