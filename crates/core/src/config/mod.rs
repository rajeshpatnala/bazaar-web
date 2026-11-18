//! Config loading and typed config files, shared by cli, server and tests.
//!
//! Every file starts with `schema_version`, `verified_on` and `source`. Every struct rejects
//! unknown keys; money and percentages are TOML strings (a TOML float is a parse error).
//! The loader reads files only: no environment, no network, no clock (callers pass `today`).

mod calendar;
mod error;
mod files;
mod risk;

use std::path::Path;

use chrono::{Datelike, NaiveDate};

pub use calendar::{
    AppliesTo, CalendarConfig, HolidayKind, HolidayRow, Phase, SessionRow, SpecialKind,
    SpecialSessionRow,
};
pub use error::ConfigError;
pub use files::{
    ChargesConfig, ComplianceConfig, DataConfig, Event, EventKind, EventsConfig, StrategiesConfig,
    Watchlist, WatchlistsConfig,
};
pub use risk::{ExecutionSection, HardLimits, Limits, ModeSection, RiskConfig};

/// Money and percentages are TOML strings. `rust_decimal::serde::str` also accepts TOML
/// numbers, so this strict version reads a string only (a float never becomes money).
mod decimal_str {
    use std::str::FromStr;

    use rust_decimal::Decimal;
    use serde::Deserialize;
    use serde::de::{Deserializer, Error};

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Decimal, D::Error> {
        let s = String::deserialize(d)?;
        Decimal::from_str(s.trim()).map_err(|_| D::Error::custom("expected a decimal string"))
    }
}

/// The only `schema_version` this loader understands.
pub const SCHEMA_VERSION: u32 = 1;

/// The three keys every config file starts with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub schema_version: u32,
    /// Date a human compared the file with its sources.
    pub verified_on: NaiveDate,
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
        Ok(())
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
    let text = std::fs::read_to_string(dir.join(T::FILE)).map_err(|source| ConfigError::Io {
        file: T::FILE,
        source,
    })?;
    parse_str(&text)
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
            let age_days = (today - header.verified_on).num_days();
            let max_age_days = policy.limit(file);
            let reason = if age_days < 0 {
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
        if !self.calendar.covered_years.contains(&today.year()) {
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
        assert_eq!(cfg.calendar.covered_years, Vec::<i32>::new());
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
        let err = parse_str::<ChargesConfig>(text).unwrap_err();
        assert!(matches!(
            err,
            ConfigError::Parse {
                file: "charges.toml",
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
            Err(ConfigError::Parse { .. })
        ));
        let bad_date = "schema_version = 1\nverified_on = \"2026-13-45\"\nsource = \"x\"\n";
        assert!(parse_str::<DataConfig>(bad_date).is_err());
    }

    #[test]
    fn staleness_age_future_and_uncovered_year() {
        let cfg = Config::load(Path::new(FIXTURES)).unwrap(); // verified 2026-01-05, covers 2026
        let policy = StalenessPolicy {
            default_max_age_days: 30,
            overrides: vec![("charges.toml", 90)],
        };
        assert_eq!(cfg.stale_warnings(d(2026, 1, 20), &policy), vec![]);
        let w = cfg.stale_warnings(d(2026, 3, 1), &policy); // 55 days: 7 files over 30
        assert_eq!(w.len(), 7);
        assert!(w.iter().all(|x| x.file != "charges.toml"));
        assert!(w.contains(&StaleWarning {
            file: "risk.toml",
            reason: StaleReason::TooOld {
                age_days: 55,
                max_age_days: 30
            },
        }));
        let w = cfg.stale_warnings(d(2026, 1, 1), &policy);
        assert_eq!(w.len(), 8);
        assert!(w.iter().all(|x| x.reason == StaleReason::FutureDated));
        let w = cfg.stale_warnings(d(2027, 1, 6), &policy);
        assert!(
            w.iter()
                .any(|x| x.reason == StaleReason::YearNotCovered { year: 2027 })
        );
    }
}
