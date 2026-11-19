//! `risk.toml`: execution mode, owner limits and the hard ceilings the loader enforces.
//!
//! Every value lives in the file (owner profile, `source = "owner"`); none is a code constant.
//! The mode is read from the file only: no env var, CLI flag or HTTP route sets it.
//! A missing key is a load error: nothing here has a serde default.

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

/// G03: what happens at the daily loss limit. Only the entry lockout exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnDailyLoss {
    LockoutEntries,
}

/// G08: how long the cool-off lasts. Only "until the end of the session" exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CooloffUntil {
    EndOfSession,
}

/// `[limits]`: percentages of capital, as decimal strings.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub max_risk_per_trade_pct: Decimal,
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub max_daily_loss_pct: Decimal,
    pub on_daily_loss: OnDailyLoss,
    /// G04a (Soft).
    pub max_open_positions: u32,
    /// G04b (Hard ceiling).
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub max_total_open_risk_pct: Decimal,
    /// G19 (Hard ceiling).
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub max_weekly_loss_pct: Decimal,
    /// G14 (Soft).
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub max_position_value_pct: Decimal,
}

/// `[hard_limits]`: ceilings for the `[limits]` and `[behaviour]` keys.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HardLimits {
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub max_risk_per_trade_pct: Decimal,
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub max_daily_loss_pct: Decimal,
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub max_total_open_risk_pct: Decimal,
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub max_weekly_loss_pct: Decimal,
    /// Largest allowed `behaviour.cooloff_after_losses`.
    pub max_cooloff_after_losses: u32,
    /// Largest allowed `behaviour.max_new_entries_per_day`.
    pub max_new_entries_per_day: u32,
    /// Smallest allowed `behaviour.reentry_lockout_minutes`.
    pub min_reentry_lockout_minutes: u32,
    /// Largest allowed `behaviour.entry_zone_tolerance_bps`.
    pub max_entry_zone_tolerance_bps: u32,
}

/// `[exposure]`: G05.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exposure {
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub max_sector_pct: Decimal,
    pub max_correlated_positions: u32,
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub correlation_threshold: Decimal,
    pub correlation_window_days: u32,
}

/// `[behaviour]`: G08 to G11, G17, G18.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Behaviour {
    pub cooloff_after_losses: u32,
    pub cooloff_until: CooloffUntil,
    pub allow_stop_widening: bool,
    pub allow_averaging_down: bool,
    pub entry_zone_tolerance_bps: u32,
    pub reentry_lockout_minutes: u32,
    pub max_new_entries_per_day: u32,
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
    pub exposure: Exposure,
    pub behaviour: Behaviour,
    pub execution: ExecutionSection,
}

/// Loader sanity bounds (inclusive). They are structural limits that stop a degenerate value from
/// switching a guard off; the trading thresholds themselves live in the file.
const OPEN_POSITIONS: (u32, u32) = (1, 20);
const CORRELATED_POSITIONS: (u32, u32) = (1, 10);
const CORRELATION_WINDOW_DAYS: (u32, u32) = (20, 250);
const COOLOFF_LOSSES: (u32, u32) = (1, 10);
const ENTRY_ZONE_BPS: (u32, u32) = (0, 50);
const REENTRY_MINUTES: (u32, u32) = (1, 480);
const ENTRIES_PER_DAY: (u32, u32) = (1, 20);

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

fn check_range(key: &str, v: u32, (lo, hi): (u32, u32)) -> Result<(), ConfigError> {
    if v < lo || v > hi {
        return Err(ConfigError::invalid(
            RiskConfig::FILE,
            key,
            format!("must be between {lo} and {hi}"),
        ));
    }
    Ok(())
}

fn check_ceiling_u32(key: &str, value: u32, ceiling: u32) -> Result<(), ConfigError> {
    if value > ceiling {
        return Err(ConfigError::invalid(
            RiskConfig::FILE,
            key,
            "is looser than its [hard_limits] ceiling",
        ));
    }
    Ok(())
}

fn check_floor_u32(key: &str, value: u32, floor: u32) -> Result<(), ConfigError> {
    if value < floor {
        return Err(ConfigError::invalid(
            RiskConfig::FILE,
            key,
            "is looser than its [hard_limits] floor",
        ));
    }
    Ok(())
}

fn forbid_true(key: &str, v: bool, why: &str) -> Result<(), ConfigError> {
    if v {
        return Err(ConfigError::invalid(RiskConfig::FILE, key, why));
    }
    Ok(())
}

impl RiskConfig {
    /// Percentage keys: range, ceilings, and week >= day.
    fn validate_percentages(&self) -> Result<(), ConfigError> {
        let (l, h) = (&self.limits, &self.hard_limits);
        check_pct(
            "hard_limits.max_risk_per_trade_pct",
            h.max_risk_per_trade_pct,
        )?;
        check_pct("hard_limits.max_daily_loss_pct", h.max_daily_loss_pct)?;
        check_pct(
            "hard_limits.max_total_open_risk_pct",
            h.max_total_open_risk_pct,
        )?;
        check_pct("hard_limits.max_weekly_loss_pct", h.max_weekly_loss_pct)?;
        check_pct("limits.max_risk_per_trade_pct", l.max_risk_per_trade_pct)?;
        check_pct("limits.max_daily_loss_pct", l.max_daily_loss_pct)?;
        check_pct("limits.max_total_open_risk_pct", l.max_total_open_risk_pct)?;
        check_pct("limits.max_weekly_loss_pct", l.max_weekly_loss_pct)?;
        check_pct("limits.max_position_value_pct", l.max_position_value_pct)?;
        check_pct("exposure.max_sector_pct", self.exposure.max_sector_pct)?;
        for (key, v, c) in [
            (
                "limits.max_risk_per_trade_pct",
                l.max_risk_per_trade_pct,
                h.max_risk_per_trade_pct,
            ),
            (
                "limits.max_daily_loss_pct",
                l.max_daily_loss_pct,
                h.max_daily_loss_pct,
            ),
            (
                "limits.max_total_open_risk_pct",
                l.max_total_open_risk_pct,
                h.max_total_open_risk_pct,
            ),
            (
                "limits.max_weekly_loss_pct",
                l.max_weekly_loss_pct,
                h.max_weekly_loss_pct,
            ),
        ] {
            check_ceiling(key, v, c)?;
        }
        // A week may not be allowed to lose less than a day.
        if l.max_weekly_loss_pct < l.max_daily_loss_pct {
            return Err(ConfigError::invalid(
                Self::FILE,
                "limits.max_weekly_loss_pct",
                "must not be below limits.max_daily_loss_pct",
            ));
        }
        Ok(())
    }

    /// Counts, minutes, correlation, the forbidden flags and the behaviour ceilings.
    fn validate_guards(&self) -> Result<(), ConfigError> {
        let (h, e, b) = (&self.hard_limits, &self.exposure, &self.behaviour);
        check_range(
            "limits.max_open_positions",
            self.limits.max_open_positions,
            OPEN_POSITIONS,
        )?;
        if e.correlation_threshold <= Decimal::ZERO || e.correlation_threshold > Decimal::ONE {
            return Err(ConfigError::invalid(
                Self::FILE,
                "exposure.correlation_threshold",
                "must be above 0 and at most 1",
            ));
        }
        check_range(
            "exposure.max_correlated_positions",
            e.max_correlated_positions,
            CORRELATED_POSITIONS,
        )?;
        check_range(
            "exposure.correlation_window_days",
            e.correlation_window_days,
            CORRELATION_WINDOW_DAYS,
        )?;
        forbid_true(
            "behaviour.allow_stop_widening",
            b.allow_stop_widening,
            "must be false (G09 is never overridable)",
        )?;
        forbid_true(
            "behaviour.allow_averaging_down",
            b.allow_averaging_down,
            "must be false (G11 is never overridable)",
        )?;
        check_range(
            "behaviour.cooloff_after_losses",
            b.cooloff_after_losses,
            COOLOFF_LOSSES,
        )?;
        check_range(
            "behaviour.entry_zone_tolerance_bps",
            b.entry_zone_tolerance_bps,
            ENTRY_ZONE_BPS,
        )?;
        check_range(
            "behaviour.reentry_lockout_minutes",
            b.reentry_lockout_minutes,
            REENTRY_MINUTES,
        )?;
        check_range(
            "behaviour.max_new_entries_per_day",
            b.max_new_entries_per_day,
            ENTRIES_PER_DAY,
        )?;
        // The ceilings use the same structural bounds, so a ceiling cannot be set to nonsense.
        check_range(
            "hard_limits.max_cooloff_after_losses",
            h.max_cooloff_after_losses,
            COOLOFF_LOSSES,
        )?;
        check_range(
            "hard_limits.max_new_entries_per_day",
            h.max_new_entries_per_day,
            ENTRIES_PER_DAY,
        )?;
        check_range(
            "hard_limits.min_reentry_lockout_minutes",
            h.min_reentry_lockout_minutes,
            REENTRY_MINUTES,
        )?;
        check_range(
            "hard_limits.max_entry_zone_tolerance_bps",
            h.max_entry_zone_tolerance_bps,
            ENTRY_ZONE_BPS,
        )?;
        check_ceiling_u32(
            "behaviour.cooloff_after_losses",
            b.cooloff_after_losses,
            h.max_cooloff_after_losses,
        )?;
        check_ceiling_u32(
            "behaviour.max_new_entries_per_day",
            b.max_new_entries_per_day,
            h.max_new_entries_per_day,
        )?;
        check_ceiling_u32(
            "behaviour.entry_zone_tolerance_bps",
            b.entry_zone_tolerance_bps,
            h.max_entry_zone_tolerance_bps,
        )?;
        check_floor_u32(
            "behaviour.reentry_lockout_minutes",
            b.reentry_lockout_minutes,
            h.min_reentry_lockout_minutes,
        )
    }
}

impl ConfigFile for RiskConfig {
    const FILE: &'static str = "risk.toml";
    impl_header!();

    fn validate(&self) -> Result<(), ConfigError> {
        self.validate_percentages()?;
        self.validate_guards()?;
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
        on_daily_loss = \"lockout_entries\"\nmax_open_positions = 3\n\
        max_total_open_risk_pct = \"3\"\nmax_weekly_loss_pct = \"6\"\n\
        max_position_value_pct = \"25\"\n\
        [hard_limits]\nmax_risk_per_trade_pct = \"1\"\nmax_daily_loss_pct = \"3\"\n\
        max_total_open_risk_pct = \"3\"\nmax_weekly_loss_pct = \"6\"\n\
        max_cooloff_after_losses = 2\nmax_new_entries_per_day = 3\n\
        min_reentry_lockout_minutes = 60\nmax_entry_zone_tolerance_bps = 0\n\
        [exposure]\nmax_sector_pct = \"30\"\nmax_correlated_positions = 2\n\
        correlation_threshold = \"0.7\"\ncorrelation_window_days = 60\n\
        [behaviour]\ncooloff_after_losses = 2\ncooloff_until = \"end_of_session\"\n\
        allow_stop_widening = false\nallow_averaging_down = false\n\
        entry_zone_tolerance_bps = 0\nreentry_lockout_minutes = 60\n\
        max_new_entries_per_day = 3\n\
        [execution]\npaper_starting_cash_inr = \"100000\"\n";

    fn with(from: &str, to: &str) -> String {
        assert!(OK.contains(from), "{from}");
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
        assert!(
            parse_str::<RiskConfig>(&with(
                "correlation_threshold = \"0.7\"",
                "correlation_threshold = 0.7"
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
    fn f6_keys_parse_with_owner_values() {
        let cfg = parse_str::<RiskConfig>(OK).unwrap();
        assert_eq!(cfg.limits.max_open_positions, 3);
        assert_eq!(cfg.limits.max_position_value_pct, Decimal::from(25));
        assert_eq!(cfg.limits.on_daily_loss, OnDailyLoss::LockoutEntries);
        assert_eq!(cfg.behaviour.cooloff_after_losses, 2);
        assert_eq!(cfg.behaviour.cooloff_until, CooloffUntil::EndOfSession);
        assert_eq!(cfg.exposure.correlation_threshold.to_string(), "0.7");
    }

    /// Every new key, as the exact line to delete. Dropping any one must fail the load
    /// (no serde default to a loose value).
    #[test]
    fn every_new_key_is_required() {
        let lines = [
            "on_daily_loss = \"lockout_entries\"\n",
            "max_open_positions = 3\n",
            "max_total_open_risk_pct = \"3\"\nmax_weekly_loss_pct = \"6\"\nmax_position_value_pct = \"25\"\n",
            "max_position_value_pct = \"25\"\n",
            "max_cooloff_after_losses = 2\n",
            "min_reentry_lockout_minutes = 60\n",
            "max_entry_zone_tolerance_bps = 0\n",
            "max_sector_pct = \"30\"\n",
            "max_correlated_positions = 2\n",
            "correlation_threshold = \"0.7\"\n",
            "correlation_window_days = 60\n",
            "cooloff_after_losses = 2\ncooloff_until",
            "cooloff_until = \"end_of_session\"\n",
            "allow_stop_widening = false\n",
            "allow_averaging_down = false\n",
            "entry_zone_tolerance_bps = 0\nreentry",
            "reentry_lockout_minutes = 60\nmax_new",
        ];
        for l in lines {
            let to = if l.ends_with("cooloff_until") {
                "cooloff_until"
            } else if l.ends_with("reentry") {
                "reentry"
            } else if l.ends_with("max_new") {
                "max_new"
            } else {
                ""
            };
            assert!(
                parse_str::<RiskConfig>(&with(l, to)).is_err(),
                "missing {l:?} must fail"
            );
        }
        // Whole sections, and the [hard_limits] keys that are not unique lines.
        for sect in ["[exposure]", "[behaviour]"] {
            assert!(parse_str::<RiskConfig>(&OK.replacen(sect, "[zzz]", 1)).is_err());
        }
        let no_hard_week = OK.replacen(
            "max_total_open_risk_pct = \"3\"\nmax_weekly_loss_pct = \"6\"\nmax_cooloff",
            "max_cooloff",
            1,
        );
        assert!(parse_str::<RiskConfig>(&no_hard_week).is_err());
        let no_hard_entries = OK.replacen(
            "max_cooloff_after_losses = 2\nmax_new_entries_per_day = 3\n",
            "max_cooloff_after_losses = 2\n",
            1,
        );
        assert!(parse_str::<RiskConfig>(&no_hard_entries).is_err());
    }

    fn assert_all_refused(bad: &[(&str, &str, &str)]) {
        for (from, to, key) in bad {
            let err = parse_str::<RiskConfig>(&with(from, to)).expect_err(to);
            assert!(err.to_string().contains(key), "{to}: {err}");
        }
    }

    /// One row per rule: (unique fragment of `OK`, replacement, text the error must contain).
    #[test]
    fn range_table_refuses_degenerate_limits_and_exposure() {
        assert_all_refused(&[
            (
                "max_open_positions = 3",
                "max_open_positions = 0",
                "max_open_positions",
            ),
            (
                "max_open_positions = 3",
                "max_open_positions = 21",
                "max_open_positions",
            ),
            (
                "max_position_value_pct = \"25\"",
                "max_position_value_pct = \"0\"",
                "max_position_value_pct",
            ),
            (
                "max_position_value_pct = \"25\"",
                "max_position_value_pct = \"100.5\"",
                "max_position_value_pct",
            ),
            (
                "[limits]\nmax_risk_per_trade_pct = \"1\"\nmax_daily_loss_pct = \"3\"\non_daily_loss = \"lockout_entries\"\nmax_open_positions = 3\nmax_total_open_risk_pct = \"3\"",
                "[limits]\nmax_risk_per_trade_pct = \"1\"\nmax_daily_loss_pct = \"3\"\non_daily_loss = \"lockout_entries\"\nmax_open_positions = 3\nmax_total_open_risk_pct = \"0\"",
                "limits.max_total_open_risk_pct",
            ),
            (
                "[limits]\nmax_risk_per_trade_pct = \"1\"\nmax_daily_loss_pct = \"3\"\non_daily_loss = \"lockout_entries\"\nmax_open_positions = 3\nmax_total_open_risk_pct = \"3\"",
                "[limits]\nmax_risk_per_trade_pct = \"1\"\nmax_daily_loss_pct = \"3\"\non_daily_loss = \"lockout_entries\"\nmax_open_positions = 3\nmax_total_open_risk_pct = \"3.5\"",
                "ceiling",
            ),
            (
                "max_weekly_loss_pct = \"6\"\nmax_position",
                "max_weekly_loss_pct = \"-1\"\nmax_position",
                "limits.max_weekly_loss_pct",
            ),
            (
                "max_weekly_loss_pct = \"6\"\nmax_position",
                "max_weekly_loss_pct = \"7\"\nmax_position",
                "ceiling",
            ),
            (
                "max_weekly_loss_pct = \"6\"\nmax_position",
                "max_weekly_loss_pct = \"2\"\nmax_position",
                "must not be below",
            ),
            (
                "max_sector_pct = \"30\"",
                "max_sector_pct = \"0\"",
                "max_sector_pct",
            ),
            (
                "max_sector_pct = \"30\"",
                "max_sector_pct = \"101\"",
                "max_sector_pct",
            ),
            (
                "max_correlated_positions = 2",
                "max_correlated_positions = 0",
                "max_correlated_positions",
            ),
            (
                "max_correlated_positions = 2",
                "max_correlated_positions = 11",
                "max_correlated_positions",
            ),
            (
                "correlation_threshold = \"0.7\"",
                "correlation_threshold = \"0\"",
                "correlation_threshold",
            ),
            (
                "correlation_threshold = \"0.7\"",
                "correlation_threshold = \"1.01\"",
                "correlation_threshold",
            ),
            (
                "correlation_window_days = 60",
                "correlation_window_days = 19",
                "correlation_window_days",
            ),
            (
                "correlation_window_days = 60",
                "correlation_window_days = 251",
                "correlation_window_days",
            ),
            (
                "allow_stop_widening = false",
                "allow_stop_widening = true",
                "allow_stop_widening",
            ),
            (
                "allow_averaging_down = false",
                "allow_averaging_down = true",
                "allow_averaging_down",
            ),
        ]);
    }

    #[test]
    fn range_table_refuses_degenerate_behaviour_and_ceilings() {
        assert_all_refused(&[
            (
                "[behaviour]\ncooloff_after_losses = 2",
                "[behaviour]\ncooloff_after_losses = 0",
                "cooloff_after_losses",
            ),
            (
                "[behaviour]\ncooloff_after_losses = 2",
                "[behaviour]\ncooloff_after_losses = 3",
                "ceiling",
            ),
            (
                "[behaviour]\ncooloff_after_losses = 2",
                "[behaviour]\ncooloff_after_losses = 11",
                "cooloff_after_losses",
            ),
            (
                "entry_zone_tolerance_bps = 0\nreentry",
                "entry_zone_tolerance_bps = 1\nreentry",
                "ceiling",
            ),
            (
                "entry_zone_tolerance_bps = 0\nreentry",
                "entry_zone_tolerance_bps = 51\nreentry",
                "entry_zone_tolerance_bps",
            ),
            (
                "reentry_lockout_minutes = 60\nmax_new",
                "reentry_lockout_minutes = 0\nmax_new",
                "reentry_lockout_minutes",
            ),
            (
                "reentry_lockout_minutes = 60\nmax_new",
                "reentry_lockout_minutes = 59\nmax_new",
                "floor",
            ),
            (
                "reentry_lockout_minutes = 60\nmax_new",
                "reentry_lockout_minutes = 481\nmax_new",
                "reentry_lockout_minutes",
            ),
            (
                "max_new_entries_per_day = 3\n[execution]",
                "max_new_entries_per_day = 0\n[execution]",
                "max_new_entries_per_day",
            ),
            (
                "max_new_entries_per_day = 3\n[execution]",
                "max_new_entries_per_day = 4\n[execution]",
                "ceiling",
            ),
            (
                "max_new_entries_per_day = 3\n[execution]",
                "max_new_entries_per_day = 21\n[execution]",
                "max_new_entries_per_day",
            ),
            // Ceilings are range-checked too, so a ceiling cannot be pushed to nonsense.
            (
                "max_cooloff_after_losses = 2",
                "max_cooloff_after_losses = 0",
                "hard_limits.max_cooloff_after_losses",
            ),
            (
                "min_reentry_lockout_minutes = 60",
                "min_reentry_lockout_minutes = 0",
                "hard_limits.min_reentry_lockout_minutes",
            ),
            (
                "max_entry_zone_tolerance_bps = 0",
                "max_entry_zone_tolerance_bps = 51",
                "hard_limits.max_entry_zone_tolerance_bps",
            ),
            (
                "max_cooloff_after_losses = 2\nmax_new_entries_per_day = 3",
                "max_cooloff_after_losses = 2\nmax_new_entries_per_day = 0",
                "hard_limits.max_new_entries_per_day",
            ),
        ]);
    }

    #[test]
    fn unknown_enum_values_are_refused() {
        assert!(parse_str::<RiskConfig>(&with("\"lockout_entries\"", "\"warn_only\"")).is_err());
        assert!(parse_str::<RiskConfig>(&with("\"end_of_session\"", "\"never\"")).is_err());
    }

    #[test]
    fn shipped_file_has_owner_decided_values_and_unchanged_verified_on() {
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
        // Owner decisions 2026-11-19.
        assert_eq!(cfg.behaviour.cooloff_after_losses, 2);
        assert_eq!(cfg.limits.max_open_positions, 3);
        assert_eq!(cfg.limits.max_position_value_pct, Decimal::from(25));
        assert_eq!(cfg.limits.max_weekly_loss_pct, Decimal::from(6));
        assert_eq!(cfg.behaviour.reentry_lockout_minutes, 60);
        assert_eq!(cfg.behaviour.max_new_entries_per_day, 3);
        assert_eq!(cfg.verified_on, NaiveDate::from_ymd_opt(2026, 11, 18));
    }
}
