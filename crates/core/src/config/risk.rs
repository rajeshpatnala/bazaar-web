//! `risk.toml`: execution mode, owner limits and the hard ceilings the loader enforces.
//!
//! Every value lives in the file (owner profile, `source = "owner"`); none is a code constant.
//! The mode is read from the file only: no env var, CLI flag or HTTP route sets it.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::Deserialize;

use super::{ConfigError, ConfigFile, impl_header};
use crate::mode::ExecutionMode;

/// `[mode]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModeSection {
    pub execution_mode: ExecutionMode,
}

/// `[limits]`: percentages of capital, as decimal strings.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub max_risk_per_trade_pct: Decimal,
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub max_daily_loss_pct: Decimal,
}

/// `[hard_limits]`: ceilings for the `[limits]` keys.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HardLimits {
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub max_risk_per_trade_pct: Decimal,
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub max_daily_loss_pct: Decimal,
}

/// `[execution]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionSection {
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub paper_starting_cash_inr: Decimal,
}

/// `risk.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RiskConfig {
    pub schema_version: u32,
    #[serde(default)]
    pub placeholder: bool,
    #[serde(default)]
    pub verified_on: Option<NaiveDate>,
    pub source: String,
    pub mode: ModeSection,
    pub limits: Limits,
    pub hard_limits: HardLimits,
    pub execution: ExecutionSection,
}

/// A percentage must be above 0 and at most 100.
fn check_pct(key: &str, v: Decimal) -> Result<(), ConfigError> {
    if v <= Decimal::ZERO || v > Decimal::ONE_HUNDRED {
        return Err(ConfigError::invalid(
            RiskConfig::FILE,
            key,
            "must be above 0 and at most 100",
        ));
    }
    Ok(())
}

fn check_ceiling(key: &str, value: Decimal, ceiling: Decimal) -> Result<(), ConfigError> {
    if value > ceiling {
        return Err(ConfigError::invalid(
            RiskConfig::FILE,
            key,
            "is looser than its [hard_limits] ceiling",
        ));
    }
    Ok(())
}

impl ConfigFile for RiskConfig {
    const FILE: &'static str = "risk.toml";
    impl_header!();

    fn validate(&self) -> Result<(), ConfigError> {
        let (l, h) = (&self.limits, &self.hard_limits);
        check_pct(
            "hard_limits.max_risk_per_trade_pct",
            h.max_risk_per_trade_pct,
        )?;
        check_pct("hard_limits.max_daily_loss_pct", h.max_daily_loss_pct)?;
        check_pct("limits.max_risk_per_trade_pct", l.max_risk_per_trade_pct)?;
        check_pct("limits.max_daily_loss_pct", l.max_daily_loss_pct)?;
        check_ceiling(
            "limits.max_risk_per_trade_pct",
            l.max_risk_per_trade_pct,
            h.max_risk_per_trade_pct,
        )?;
        check_ceiling(
            "limits.max_daily_loss_pct",
            l.max_daily_loss_pct,
            h.max_daily_loss_pct,
        )?;
        if self.execution.paper_starting_cash_inr <= Decimal::ZERO {
            return Err(ConfigError::invalid(
                Self::FILE,
                "execution.paper_starting_cash_inr",
                "must be above 0",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse_str;

    const OK: &str = "schema_version = 1\nverified_on = \"2026-01-01\"\nsource = \"owner\"\n\
        [mode]\nexecution_mode = \"paper\"\n\
        [limits]\nmax_risk_per_trade_pct = \"1\"\nmax_daily_loss_pct = \"3\"\n\
        [hard_limits]\nmax_risk_per_trade_pct = \"1\"\nmax_daily_loss_pct = \"3\"\n\
        [execution]\npaper_starting_cash_inr = \"100000\"\n";

    fn with(from: &str, to: &str) -> String {
        assert!(OK.contains(from));
        OK.replacen(from, to, 1)
    }

    #[test]
    fn parses_decimals_exactly() {
        let cfg = parse_str::<RiskConfig>(&with("\"100000\"", "\"100000.50\"")).unwrap();
        assert_eq!(
            cfg.execution.paper_starting_cash_inr.to_string(),
            "100000.50"
        );
        assert_eq!(cfg.mode.execution_mode, ExecutionMode::Paper);
    }

    #[test]
    fn rejects_whitespace_around_decimal_strings() {
        assert!(parse_str::<RiskConfig>(&with("\"100000\"", "\" 100000\"")).is_err());
        assert!(parse_str::<RiskConfig>(&with("\"100000\"", "\"100000 \"")).is_err());
    }

    #[test]
    fn rejects_toml_float_and_integer_for_money() {
        assert!(parse_str::<RiskConfig>(&with("\"100000\"", "100000.5")).is_err());
        assert!(parse_str::<RiskConfig>(&with("\"100000\"", "100000")).is_err());
        assert!(
            parse_str::<RiskConfig>(&with(
                "max_daily_loss_pct = \"3\"",
                "max_daily_loss_pct = 3.0"
            ))
            .is_err()
        );
    }

    #[test]
    fn rejects_limit_above_hard_ceiling() {
        let text = with(
            "[limits]\nmax_risk_per_trade_pct = \"1\"",
            "[limits]\nmax_risk_per_trade_pct = \"1.01\"",
        );
        let err = parse_str::<RiskConfig>(&text).unwrap_err();
        assert!(err.to_string().contains("limits.max_risk_per_trade_pct"));
        assert!(err.to_string().contains("ceiling"));

        let text = with("max_daily_loss_pct = \"3\"", "max_daily_loss_pct = \"3.5\"");
        let err = parse_str::<RiskConfig>(&text).unwrap_err();
        assert!(err.to_string().contains("limits.max_daily_loss_pct"));
    }

    #[test]
    fn accepts_limit_below_or_equal_ceiling() {
        let text = with(
            "[limits]\nmax_risk_per_trade_pct = \"1\"",
            "[limits]\nmax_risk_per_trade_pct = \"0.25\"",
        );
        assert!(parse_str::<RiskConfig>(&text).is_ok());
    }

    #[test]
    fn rejects_zero_negative_and_over_100() {
        for bad in ["0", "-1", "101"] {
            let text = with(
                "[limits]\nmax_risk_per_trade_pct = \"1\"",
                &format!("[limits]\nmax_risk_per_trade_pct = \"{bad}\""),
            );
            assert!(parse_str::<RiskConfig>(&text).is_err(), "{bad}");
        }
        let text = with(
            "[hard_limits]\nmax_risk_per_trade_pct = \"1\"",
            "[hard_limits]\nmax_risk_per_trade_pct = \"0\"",
        );
        assert!(parse_str::<RiskConfig>(&text).is_err());
    }

    #[test]
    fn rejects_non_positive_cash_unknown_mode_and_unknown_key() {
        assert!(parse_str::<RiskConfig>(&with("\"100000\"", "\"0\"")).is_err());
        assert!(parse_str::<RiskConfig>(&with("\"paper\"", "\"live\"")).is_err());
        let extra = format!("{OK}[override]\nx = 1\n");
        assert!(parse_str::<RiskConfig>(&extra).is_err());
        let missing = OK.split("[execution]").next().unwrap();
        assert!(parse_str::<RiskConfig>(missing).is_err());
    }

    #[test]
    fn shipped_defaults_are_paper_one_and_three_percent() {
        let cfg: RiskConfig = crate::config::load(std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../config"
        )))
        .unwrap();
        assert_eq!(cfg.mode.execution_mode, ExecutionMode::Paper);
        assert_eq!(cfg.limits.max_risk_per_trade_pct, Decimal::ONE);
        assert_eq!(cfg.limits.max_daily_loss_pct, Decimal::from(3));
        assert_eq!(cfg.hard_limits.max_risk_per_trade_pct, Decimal::ONE);
        assert_eq!(cfg.hard_limits.max_daily_loss_pct, Decimal::from(3));
        assert_eq!(
            cfg.execution.paper_starting_cash_inr,
            Decimal::from(100_000)
        );
    }
}
