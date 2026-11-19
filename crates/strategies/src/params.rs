//! Typed loading of one `[strategy.<id>]` table of `config/strategies.toml`.
//!
//! Every key is UNVERIFIED (convention or proposal), accepted by the owner on 2026-11-20 and never
//! tuned on strategy P&L. An unknown key or an out-of-range value is a load error.

use std::fmt::Write;

use bazaar_regime::{RegimeFit, RegimeFitError, Trend, VolState};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

/// Why a strategy table could not be used.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ParamError {
    #[error("strategy {id}: {reason}")]
    Parse { id: String, reason: String },
    #[error("strategy {id}: {key}: {reason}")]
    Invalid {
        id: String,
        key: &'static str,
        reason: &'static str,
    },
    #[error("strategy {id}: regime_fit: {source}")]
    RegimeFit { id: String, source: RegimeFitError },
    #[error("strategies.toml has [strategy.{0}] but no factory builds it")]
    UnknownStrategy(String),
}

/// A params struct: `Default` (the spec defaults), serde with `deny_unknown_fields`, and ranges.
pub trait ParamSet: DeserializeOwned + Default {
    /// Check every range.
    ///
    /// # Errors
    /// `ParamError::Invalid` naming the key.
    fn validate(&self, id: &str) -> Result<(), ParamError>;
}

/// Regime cells as written in the file: `"up:normal"`; trends `up|down|range`, vols `low|normal|high`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegimeFitConfig {
    pub market_trends: Vec<String>,
    pub stock_cells: Vec<String>,
    /// Allowed but weaker cells (must be in `stock_cells`).
    #[serde(default)]
    pub weak_cells: Vec<String>,
}

/// The `[strategy.<id>]` table.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrategyConfig {
    pub enabled: bool,
    /// Score multiplier the scanner applies to a `Penalised` fit, in (0, 1). UNVERIFIED.
    pub penalty_weight: f64,
    pub regime_fit: RegimeFitConfig,
    pub params: toml::Table,
}

fn trend(id: &str, s: &str) -> Result<Trend, ParamError> {
    match s {
        "up" => Ok(Trend::Up),
        "down" => Ok(Trend::Down),
        "range" => Ok(Trend::Range),
        _ => Err(invalid(id, "regime_fit", "trend must be up, down or range")),
    }
}

fn cell(id: &str, s: &str) -> Result<(Trend, VolState), ParamError> {
    let (t, v) = s
        .split_once(':')
        .ok_or_else(|| invalid(id, "regime_fit", "cell must look like up:normal"))?;
    let vol = match v {
        "low" => VolState::Low,
        "normal" => VolState::Normal,
        "high" => VolState::High,
        _ => return Err(invalid(id, "regime_fit", "vol must be low, normal or high")),
    };
    Ok((trend(id, t)?, vol))
}

fn invalid(id: &str, key: &'static str, reason: &'static str) -> ParamError {
    ParamError::Invalid {
        id: id.to_owned(),
        key,
        reason,
    }
}

impl StrategyConfig {
    /// Parse and validate one strategy table.
    ///
    /// # Errors
    /// Unknown key, wrong type, `penalty_weight` outside (0, 1), or a bad regime declaration.
    pub fn from_table(id: &str, table: &toml::Table) -> Result<Self, ParamError> {
        let c: Self = table
            .clone()
            .try_into()
            .map_err(|e: toml::de::Error| ParamError::Parse {
                id: id.to_owned(),
                reason: e.message().to_owned(),
            })?;
        if !(c.penalty_weight > 0.0 && c.penalty_weight < 1.0) {
            return Err(invalid(id, "penalty_weight", "must be above 0 and below 1"));
        }
        c.regime_fit(id)?;
        Ok(c)
    }

    /// The declared regime cells, checked with `RegimeFit::check` (a bad declaration only suppresses,
    /// so it is rejected here, at load).
    ///
    /// # Errors
    /// A parse problem, or the `RegimeFitError` from `check`.
    pub fn regime_fit(&self, id: &str) -> Result<RegimeFit, ParamError> {
        let cells = |v: &[String]| v.iter().map(|s| cell(id, s)).collect::<Result<Vec<_>, _>>();
        let fit = RegimeFit {
            market_trends: self
                .regime_fit
                .market_trends
                .iter()
                .map(|s| trend(id, s))
                .collect::<Result<_, _>>()?,
            stock_cells: cells(&self.regime_fit.stock_cells)?,
            weak_cells: cells(&self.regime_fit.weak_cells)?,
        };
        fit.check().map_err(|source| ParamError::RegimeFit {
            id: id.to_owned(),
            source,
        })?;
        Ok(fit)
    }
}

/// Parse `[strategy.<id>.params]` into `P` (unknown key = error) and validate its ranges.
///
/// # Errors
/// `Parse` for an unknown key or wrong type, `Invalid` for an out-of-range value.
pub fn parse_params<P: ParamSet>(id: &str, table: &toml::Table) -> Result<P, ParamError> {
    let p: P = table
        .clone()
        .try_into()
        .map_err(|e: toml::de::Error| ParamError::Parse {
            id: id.to_owned(),
            reason: e.message().to_owned(),
        })?;
    p.validate(id)?;
    Ok(p)
}

/// Provenance hash of the params table: SHA-256 of its canonical TOML (keys sorted), hex.
#[must_use]
pub fn params_hash(table: &toml::Table) -> String {
    let text = toml::to_string(table).unwrap_or_default();
    Sha256::digest(text.as_bytes())
        .iter()
        .fold(String::new(), |mut acc, b| {
            let _ = write!(acc, "{b:02x}");
            acc
        })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(deny_unknown_fields)]
    pub(crate) struct Demo {
        n: usize,
    }
    impl Default for Demo {
        fn default() -> Self {
            Self { n: 20 }
        }
    }
    impl ParamSet for Demo {
        fn validate(&self, id: &str) -> Result<(), ParamError> {
            if (15..=30).contains(&self.n) {
                Ok(())
            } else {
                Err(invalid(id, "n", "must be 15..=30"))
            }
        }
    }

    fn tbl(s: &str) -> toml::Table {
        s.parse().unwrap()
    }

    const OK: &str = r#"
enabled = false
penalty_weight = 0.5
params = { n = 20 }
[regime_fit]
market_trends = ["up", "range"]
stock_cells = ["up:low", "up:normal", "up:high"]
weak_cells = ["up:high"]
"#;

    #[test]
    fn good_table_loads_and_builds_a_checked_regime_fit() {
        let c = StrategyConfig::from_table("d", &tbl(OK)).unwrap();
        assert!(!c.enabled);
        let fit = c.regime_fit("d").unwrap();
        assert_eq!(fit.weak_cells, vec![(Trend::Up, VolState::High)]);
        assert_eq!(fit.market_trends, vec![Trend::Up, Trend::Range]);
    }

    #[test]
    fn bad_tables_are_load_errors() {
        let e = |s: String| StrategyConfig::from_table("d", &tbl(&s)).unwrap_err();
        assert!(matches!(
            e(format!("{OK}\nextra = 1")),
            ParamError::Parse { .. }
        ));
        assert!(matches!(
            e(OK.replace("0.5", "1.5")),
            ParamError::Invalid {
                key: "penalty_weight",
                ..
            }
        ));
        assert!(matches!(
            e(OK.replace(
                "weak_cells = [\"up:high\"]",
                "weak_cells = [\"range:high\"]"
            )),
            ParamError::RegimeFit {
                source: RegimeFitError::WeakNotDeclared,
                ..
            }
        ));
        assert!(matches!(
            e(OK.replace("\"range\"", "\"mixed\"")),
            ParamError::Invalid {
                key: "regime_fit",
                ..
            }
        ));
        assert!(matches!(
            e(OK.replace("up:low", "uplow")),
            ParamError::Invalid {
                key: "regime_fit",
                ..
            }
        ));
    }

    #[test]
    fn params_reject_unknown_keys_and_out_of_range_values() {
        assert_eq!(
            parse_params::<Demo>("d", &tbl("n = 25")).unwrap(),
            Demo { n: 25 }
        );
        assert!(matches!(
            parse_params::<Demo>("d", &tbl("n = 25\nz = 1")),
            Err(ParamError::Parse { .. })
        ));
        assert!(matches!(
            parse_params::<Demo>("d", &tbl("n = 5")),
            Err(ParamError::Invalid { key: "n", .. })
        ));
    }

    #[test]
    fn params_hash_is_stable_and_changes_with_values() {
        assert_eq!(
            params_hash(&tbl("a = 1\nb = 2")),
            params_hash(&tbl("b = 2\na = 1"))
        );
        assert_ne!(params_hash(&tbl("a = 1")), params_hash(&tbl("a = 2")));
        assert_eq!(params_hash(&tbl("a = 1")).len(), 64);
    }

    #[test]
    fn shipped_config_has_three_disabled_valid_tables() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config");
        let cfg: bazaar_core::config::StrategiesConfig = bazaar_core::config::load(&dir).unwrap();
        let ids = ["donchian_breakout", "trend_pullback", "bollinger_reversion"];
        assert_eq!(cfg.strategy.keys().count(), ids.len());
        for id in ids {
            let c = StrategyConfig::from_table(id, &cfg.strategy[id]).unwrap();
            assert!(!c.enabled, "{id} must ship disabled");
        }
    }
}
