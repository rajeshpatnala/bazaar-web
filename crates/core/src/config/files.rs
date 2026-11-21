//! Typed structs for the six files that carry little or no data in F1.
//!
//! `strategies`, `data` and `compliance` are header-only; a later feature replaces
//! each struct with its real tables (and `deny_unknown_fields` makes a premature table fail).

use std::collections::BTreeMap;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::Deserialize;

use super::{ConfigError, ConfigFile, impl_header};

/// Bounds applied before any write path exists ("bound everything").
pub const MAX_WATCHLISTS: usize = 100;
pub const MAX_SYMBOLS_PER_WATCHLIST: usize = 1000;
pub const MAX_EVENTS: usize = 10_000;
/// Longest name, symbol, source or note, in characters.
pub const MAX_TEXT_CHARS: usize = 500;

fn too_long(s: &str) -> bool {
    s.chars().count() > MAX_TEXT_CHARS
}

/// Defines a header-only config struct.
macro_rules! header_only {
    ($(#[$doc:meta])* $name:ident, $file:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            pub schema_version: u32,
            #[serde(default)]
            pub placeholder: bool,
            #[serde(default)]
            pub verified_on: Option<NaiveDate>,
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

/// `strategies.toml`: header plus raw `[regime]` and `[strategy.<id>]` tables (owner: `bz-quant`).
///
/// Tables stay raw so core never depends on the crates that own their schema
/// (`bazaar-regime` parses `[regime]`; F8 parses `[strategy.<id>]`). Any other table is an error.
/// `validate` is a no-op here: F8 must bound and validate the `[strategy.<id>]` tables
/// (count and size; only the whole-file size cap applies today).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrategiesConfig {
    pub schema_version: u32,
    #[serde(default)]
    pub placeholder: bool,
    #[serde(default)]
    pub verified_on: Option<NaiveDate>,
    pub source: String,
    #[serde(default)]
    pub regime: Option<toml::Table>,
    #[serde(default)]
    pub strategy: BTreeMap<String, toml::Table>,
    /// Scanner scoring and filter settings (F11). Typed here: no strategy-world crate owns it.
    #[serde(default)]
    pub scanner: Option<ScannerConfig>,
}

impl StrategiesConfig {
    /// The `[scanner]` table. Fails closed: a missing table is an error, never a set of defaults.
    pub fn scanner_required(&self) -> Result<&ScannerConfig, ConfigError> {
        self.scanner.as_ref().ok_or_else(|| {
            ConfigError::invalid(
                Self::FILE,
                "scanner",
                "the [scanner] table is required to scan",
            )
        })
    }
}

impl ConfigFile for StrategiesConfig {
    const FILE: &'static str = "strategies.toml";
    impl_header!();
    fn validate(&self) -> Result<(), ConfigError> {
        match &self.scanner {
            Some(s) => s.validate(Self::FILE),
            None => Ok(()),
        }
    }
}

/// Score weights of the four additive components. Each is in 0..=1 and they sum to 1.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScannerWeights {
    pub confluence: f64,
    pub regime_fit: f64,
    pub liquidity: f64,
    pub evidence: f64,
}

/// `[scanner]` table of `strategies.toml`. Every value is an UNVERIFIED proposal.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScannerConfig {
    pub weights: ScannerWeights,
    /// Subtractive event penalty ceiling, in score points (0..=100).
    pub event_penalty_max_points: f64,
    /// Median 20-day traded value (rupees) below which a name is dropped. Written as a string.
    #[serde(with = "rust_decimal::serde::str")]
    pub min_median_traded_value: Decimal,
    /// Bars of daily history a name needs to be scanned.
    pub min_history_bars: u32,
    /// Fewer backtest trades than this means `low_evidence`.
    pub min_trades_evidence: u32,
    /// Pseudo-trades pulling thin evidence toward the strategy-wide stats: `n / (n + k)`.
    pub shrink_k: u32,
    /// Confidence ceiling (0..=100) for a `low_evidence` recommendation.
    pub low_evidence_cap: u8,
    /// Recommendations returned; blocked ones are listed apart and do not use these slots. 1..=20.
    pub max_results: u32,
    /// A ban list older than this (or undated) fails closed.
    pub max_ban_list_age_hours: u32,
    /// Liquidity scores 0 at the floor and 1 at this multiple of it, on a log scale (> 1).
    pub liquidity_full_score_multiple: f64,
}

impl ScannerConfig {
    fn validate(&self, file: &'static str) -> Result<(), ConfigError> {
        let w = &self.weights;
        let parts = [w.confluence, w.regime_fit, w.liquidity, w.evidence];
        if parts
            .iter()
            .any(|x| !x.is_finite() || !(0.0..=1.0).contains(x))
        {
            return Err(ConfigError::invalid(
                file,
                "scanner.weights",
                "each weight must be within 0 and 1",
            ));
        }
        if (parts.iter().sum::<f64>() - 1.0).abs() > 1e-9 {
            return Err(ConfigError::invalid(
                file,
                "scanner.weights",
                "weights must sum to 1",
            ));
        }
        let p = self.event_penalty_max_points;
        if !p.is_finite() || !(0.0..=100.0).contains(&p) {
            return Err(ConfigError::invalid(
                file,
                "scanner.event_penalty_max_points",
                "must be within 0 and 100",
            ));
        }
        if self.min_median_traded_value <= Decimal::ZERO {
            return Err(ConfigError::invalid(
                file,
                "scanner.min_median_traded_value",
                "must be above 0",
            ));
        }
        if self.low_evidence_cap >= 100 {
            return Err(ConfigError::invalid(
                file,
                "scanner.low_evidence_cap",
                "must be below 100 (100 switches the cap off)",
            ));
        }
        if !(1..=20).contains(&self.max_results) {
            return Err(ConfigError::invalid(
                file,
                "scanner.max_results",
                "must be within 1 and 20",
            ));
        }
        let m = self.liquidity_full_score_multiple;
        if !m.is_finite() || m <= 1.0 || m > 1000.0 {
            return Err(ConfigError::invalid(
                file,
                "scanner.liquidity_full_score_multiple",
                "must be above 1 and at most 1000",
            ));
        }
        for (key, v) in [
            ("min_history_bars", self.min_history_bars),
            ("min_trades_evidence", self.min_trades_evidence),
            ("shrink_k", self.shrink_k),
            ("max_ban_list_age_hours", self.max_ban_list_age_hours),
        ] {
            if v == 0 || v > 100_000 {
                return Err(ConfigError::invalid(
                    file,
                    format!("scanner.{key}"),
                    "must be within 1 and 100000",
                ));
            }
        }
        Ok(())
    }
}
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
    #[serde(default)]
    pub placeholder: bool,
    #[serde(default)]
    pub verified_on: Option<NaiveDate>,
    pub source: String,
    #[serde(default)]
    pub watchlist: Vec<Watchlist>,
}

impl ConfigFile for WatchlistsConfig {
    const FILE: &'static str = "watchlists.toml";
    impl_header!();
    fn validate(&self) -> Result<(), ConfigError> {
        if self.watchlist.len() > MAX_WATCHLISTS {
            return Err(ConfigError::invalid(
                Self::FILE,
                "watchlist",
                "too many lists",
            ));
        }
        for (i, list) in self.watchlist.iter().enumerate() {
            if list.symbols.len() > MAX_SYMBOLS_PER_WATCHLIST {
                return Err(ConfigError::invalid(
                    Self::FILE,
                    format!("watchlist[{i}].symbols"),
                    "too many symbols",
                ));
            }
            if too_long(&list.name) || list.symbols.iter().any(|s| too_long(s)) {
                return Err(ConfigError::invalid(
                    Self::FILE,
                    format!("watchlist[{i}]"),
                    "name or symbol is too long",
                ));
            }
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
    #[serde(default)]
    pub placeholder: bool,
    #[serde(default)]
    pub verified_on: Option<NaiveDate>,
    pub source: String,
    #[serde(default)]
    pub event: Vec<Event>,
}

impl ConfigFile for EventsConfig {
    const FILE: &'static str = "events.toml";
    impl_header!();
    fn validate(&self) -> Result<(), ConfigError> {
        if self.event.len() > MAX_EVENTS {
            return Err(ConfigError::invalid(Self::FILE, "event", "too many events"));
        }
        for (i, ev) in self.event.iter().enumerate() {
            if too_long(&ev.symbol) || too_long(&ev.note) || too_long(&ev.source) {
                return Err(ConfigError::invalid(
                    Self::FILE,
                    format!("event[{i}]"),
                    "symbol, note or source is too long",
                ));
            }
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
        assert!(parse_str::<DataConfig>(&text).is_err());
        assert!(parse_str::<ComplianceConfig>(&text).is_err());
    }

    #[test]
    fn strategies_keeps_raw_tables_and_rejects_others() {
        let ok = format!("{HEAD}[regime]\nconfirm_bars = 3\n[strategy.a]\nx = 1\n");
        let c = parse_str::<StrategiesConfig>(&ok).unwrap();
        assert!(c.regime.unwrap().contains_key("confirm_bars"));
        assert!(c.strategy.contains_key("a"));
        let none = parse_str::<StrategiesConfig>(HEAD).unwrap();
        assert!(none.regime.is_none() && none.strategy.is_empty());
        let bad = format!("{HEAD}[rates]\nstt = \"1\"\n");
        assert!(parse_str::<StrategiesConfig>(&bad).is_err());
    }

    const SCANNER: &str = "[scanner]\nevent_penalty_max_points = 15.0\nmin_median_traded_value = \"100000000\"\nmin_history_bars = 250\nmin_trades_evidence = 30\nshrink_k = 30\nlow_evidence_cap = 55\nmax_results = 5\nmax_ban_list_age_hours = 24\nliquidity_full_score_multiple = 10.0\n[scanner.weights]\nconfluence = 0.30\nregime_fit = 0.25\nliquidity = 0.15\nevidence = 0.30\n";

    #[test]
    fn scanner_table_parses_and_validates() {
        let c = parse_str::<StrategiesConfig>(&format!("{HEAD}{SCANNER}")).unwrap();
        let s = c.scanner.unwrap();
        assert_eq!(s.low_evidence_cap, 55);
        assert_eq!(s.min_median_traded_value, Decimal::from(100_000_000));
    }

    #[test]
    fn missing_scanner_table_fails_closed() {
        let none = parse_str::<StrategiesConfig>(HEAD).unwrap();
        assert!(none.scanner_required().is_err());
        let some = parse_str::<StrategiesConfig>(&format!("{HEAD}{SCANNER}")).unwrap();
        assert!(some.scanner_required().is_ok());
    }

    #[test]
    fn scanner_rejects_bad_values() {
        let bad = |from: &str, to: &str| {
            parse_str::<StrategiesConfig>(&format!("{HEAD}{}", SCANNER.replace(from, to)))
        };
        assert!(
            bad("confluence = 0.30", "confluence = 0.40").is_err(),
            "sum 1.1"
        );
        assert!(
            bad("confluence = 0.30", "confluence = 1.30").is_err(),
            "weight above 1"
        );
        assert!(
            bad("evidence = 0.30", "evidence = -0.05").is_err(),
            "negative weight"
        );
        assert!(bad("= 15.0", "= 150.0").is_err());
        assert!(bad("\"100000000\"", "\"0\"").is_err());
        assert!(bad("= 55", "= 101").is_err());
        assert!(bad("shrink_k = 30", "shrink_k = 0").is_err());
        assert!(bad("max_results = 5", "max_results = 21").is_err());
        assert!(bad("max_results = 5", "max_results = 5\nextra = 1").is_err());
        assert!(bad("= 24", "= 0").is_err());
        assert!(bad("= 10.0", "= 1.0").is_err());
        assert!(bad("= 10.0", "= 1001.0").is_err());
        assert!(bad("= 55", "= 100").is_err(), "cap 100 is a no-op");
        assert!(bad("min_history_bars = 250", "min_history_bars = 0").is_err());
        assert!(bad("min_trades_evidence = 30", "min_trades_evidence = 0").is_err());
        assert!(bad("= 15.0", "= -1.0").is_err());
        assert!(bad("= 15.0", "= nan").is_err());
        assert!(
            bad("confluence = 0.30", "confluence = 0.3000000001").is_ok(),
            "sum within tolerance"
        );
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
    fn bounds_on_counts_and_lengths() {
        let long = "x".repeat(MAX_TEXT_CHARS + 1);
        let text = format!("{HEAD}[[watchlist]]\nname = \"{long}\"\nsymbols = []\n");
        assert!(parse_str::<WatchlistsConfig>(&text).is_err());
        let text = format!("{HEAD}[[watchlist]]\nname = \"a\"\nsymbols = [\"{long}\"]\n");
        assert!(parse_str::<WatchlistsConfig>(&text).is_err());
        let syms = vec!["\"A\""; MAX_SYMBOLS_PER_WATCHLIST + 1].join(",");
        let text = format!("{HEAD}[[watchlist]]\nname = \"a\"\nsymbols = [{syms}]\n");
        assert!(parse_str::<WatchlistsConfig>(&text).is_err());
        let one = "[[watchlist]]\nname = \"n\"\nsymbols = []\n";
        let lists = one.repeat(MAX_WATCHLISTS + 1);
        assert!(parse_str::<WatchlistsConfig>(&format!("{HEAD}{lists}")).is_err());

        let ev = |note: &str| {
            format!(
                "[[event]]\ndate = \"2026-03-20\"\nkind = \"budget\"\nsymbol = \"\"\nnote = \"{note}\"\nsource = \"s\"\n"
            )
        };
        assert!(parse_str::<EventsConfig>(&format!("{HEAD}{}", ev("ok"))).is_ok());
        assert!(parse_str::<EventsConfig>(&format!("{HEAD}{}", ev(&long))).is_err());
        let many = ev("n").repeat(MAX_EVENTS + 1);
        assert!(parse_str::<EventsConfig>(&format!("{HEAD}{many}")).is_err());
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
