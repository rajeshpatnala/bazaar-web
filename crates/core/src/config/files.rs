//! Typed structs for the six files that carry little or no data in F1.
//!
//! `charges`, `strategies`, `data` and `compliance` are header-only; a later feature replaces
//! each struct with its real tables (and `deny_unknown_fields` makes a premature table fail).

use chrono::NaiveDate;
use serde::Deserialize;

use super::{ConfigError, ConfigFile, impl_header};

/// Defines a header-only config struct.
macro_rules! header_only {
    ($(#[$doc:meta])* $name:ident, $file:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            pub schema_version: u32,
            pub verified_on: NaiveDate,
            pub source: String,
        }

        impl ConfigFile for $name {
            const FILE: &'static str = $file;
            impl_header!();
            fn validate(&self) -> Result<(), ConfigError> {
                Ok(())
            }
        }
    };
}

header_only!(
    /// `charges.toml`: rate sets arrive with F2 (owner: `bz-backtest-risk`).
    ChargesConfig,
    "charges.toml"
);
header_only!(
    /// `strategies.toml`: enabled strategies and params (owner: `bz-quant`).
    StrategiesConfig,
    "strategies.toml"
);
header_only!(
    /// `data.toml`: rate limits and refresh windows (owner: `bz-market-data`).
    DataConfig,
    "data.toml"
);
header_only!(
    /// `compliance.toml`: live-phase checklist state (phase 9 only).
    ComplianceConfig,
    "compliance.toml"
);

/// One named, user-owned symbol list.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Watchlist {
    pub name: String,
    pub symbols: Vec<String>,
}

/// `watchlists.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WatchlistsConfig {
    pub schema_version: u32,
    pub verified_on: NaiveDate,
    pub source: String,
    #[serde(default)]
    pub watchlist: Vec<Watchlist>,
}

impl ConfigFile for WatchlistsConfig {
    const FILE: &'static str = "watchlists.toml";
    impl_header!();
    fn validate(&self) -> Result<(), ConfigError> {
        for (i, list) in self.watchlist.iter().enumerate() {
            if list.name.trim().is_empty() {
                return Err(ConfigError::invalid(
                    Self::FILE,
                    format!("watchlist[{i}].name"),
                    "must not be empty",
                ));
            }
            if self.watchlist[..i].iter().any(|o| o.name == list.name) {
                return Err(ConfigError::invalid(
                    Self::FILE,
                    format!("watchlist[{i}].name"),
                    "duplicate name",
                ));
            }
            if let Some(j) = list.symbols.iter().position(|s| s.trim().is_empty()) {
                return Err(ConfigError::invalid(
                    Self::FILE,
                    format!("watchlist[{i}].symbols[{j}]"),
                    "must not be empty",
                ));
            }
        }
        Ok(())
    }
}

/// Kind of a dated event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Results,
    RbiPolicy,
    Budget,
    Expiry,
    ExDate,
    IndexRebalance,
    SpecialSession,
}

/// One dated event.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub date: NaiveDate,
    pub kind: EventKind,
    /// Empty means market-wide.
    pub symbol: String,
    pub note: String,
    pub source: String,
}

/// `events.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventsConfig {
    pub schema_version: u32,
    pub verified_on: NaiveDate,
    pub source: String,
    #[serde(default)]
    pub event: Vec<Event>,
}

impl ConfigFile for EventsConfig {
    const FILE: &'static str = "events.toml";
    impl_header!();
    fn validate(&self) -> Result<(), ConfigError> {
        for (i, ev) in self.event.iter().enumerate() {
            if ev.source.trim().is_empty() {
                return Err(ConfigError::invalid(
                    Self::FILE,
                    format!("event[{i}].source"),
                    "must not be empty",
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse_str;

    const HEAD: &str = "schema_version = 1\nverified_on = \"2026-01-01\"\nsource = \"x\"\n";

    #[test]
    fn header_only_files_reject_tables() {
        let text = format!("{HEAD}[rates]\nstt = \"1\"\n");
        assert!(parse_str::<ChargesConfig>(&text).is_err());
        assert!(parse_str::<StrategiesConfig>(&text).is_err());
        assert!(parse_str::<DataConfig>(&text).is_err());
        assert!(parse_str::<ComplianceConfig>(&text).is_err());
        assert!(parse_str::<ChargesConfig>(HEAD).is_ok());
    }

    #[test]
    fn watchlist_validation() {
        let ok = format!("{HEAD}[[watchlist]]\nname = \"a\"\nsymbols = [\"X\"]\n");
        assert_eq!(
            parse_str::<WatchlistsConfig>(&ok).unwrap().watchlist.len(),
            1
        );
        assert_eq!(
            parse_str::<WatchlistsConfig>(HEAD).unwrap().watchlist.len(),
            0
        );

        let dup = format!("{ok}[[watchlist]]\nname = \"a\"\nsymbols = []\n");
        let err = parse_str::<WatchlistsConfig>(&dup).unwrap_err();
        assert!(err.to_string().contains("watchlist[1].name"));

        let blank_sym = format!("{HEAD}[[watchlist]]\nname = \"a\"\nsymbols = [\"X\", \" \"]\n");
        let err = parse_str::<WatchlistsConfig>(&blank_sym).unwrap_err();
        assert!(err.to_string().contains("watchlist[0].symbols[1]"));

        let blank_name = format!("{HEAD}[[watchlist]]\nname = \"\"\nsymbols = []\n");
        assert!(parse_str::<WatchlistsConfig>(&blank_name).is_err());
    }

    #[test]
    fn event_validation() {
        let row =
            "[[event]]\ndate = \"2026-03-20\"\nkind = \"budget\"\nsymbol = \"\"\nnote = \"n\"\n";
        let ok = format!("{HEAD}{row}source = \"s\"\n");
        assert_eq!(
            parse_str::<EventsConfig>(&ok).unwrap().event[0].kind,
            EventKind::Budget
        );
        let blank = format!("{HEAD}{row}source = \"\"\n");
        assert!(parse_str::<EventsConfig>(&blank).is_err());
        let bad_kind = ok.replace("budget", "party");
        assert!(parse_str::<EventsConfig>(&bad_kind).is_err());
    }
}
