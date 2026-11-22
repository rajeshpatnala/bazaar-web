//! `risk.toml`: execution mode, owner limits and the hard ceilings the loader enforces.
//!
//! Every value lives in the file (owner profile, `source = "owner"`); none is a code constant.
//! The mode is read from the file only: no env var, CLI flag or HTTP route sets it.
//! A missing key is a load error: nothing here has a serde default.

use chrono::{NaiveDate, NaiveTime, Timelike};
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
    /// G16 orders per minute (Hard ceiling).
    pub max_orders_per_minute: u32,
    /// G16 orders per day.
    pub max_orders_per_day: u32,
    /// G16 notional per order, rupees.
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub max_notional_per_order_inr: Decimal,
    /// Floor for the charges + slippage buffer the sizer accepts (rupees).
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub min_charges_slippage_buffer: Decimal,
    /// Sizing equity never rises above starting cash when true.
    pub cap_equity_at_starting_cash: bool,
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
    /// Ceiling for `limits.max_position_value_pct` (G14).
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub max_position_value_pct: Decimal,
    /// Ceiling for `exposure.max_sector_pct` (G05).
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub max_sector_pct: Decimal,
    /// Ceiling for `limits.max_open_positions` (G04a).
    pub max_open_positions: u32,
    /// Ceiling for `limits.max_orders_per_minute` (G16).
    pub max_orders_per_minute: u32,
    /// Ceiling for `limits.max_orders_per_day` (G16).
    pub max_orders_per_day: u32,
    /// Ceiling for `limits.max_notional_per_order_inr` (G16), rupees.
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub max_notional_per_order_inr: Decimal,
    /// Ceiling for `events.ban_list_max_age_hours` (G06).
    pub max_ban_list_age_hours: u32,
    /// Ceiling for `data.max_quote_age_secs` (G15).
    pub max_quote_age_secs: u32,
    /// Ceiling for `data.max_snapshot_age_secs`.
    pub max_snapshot_age_secs: u32,
    /// Latest allowed `events.expiry_day_no_new_after_ist` (G07), `HH:MM`.
    #[serde(deserialize_with = "hhmm")]
    pub latest_expiry_day_no_new_after_ist: NaiveTime,
    /// Smallest allowed `override.confirm_delay_secs`.
    pub min_confirm_delay_secs: u32,
    /// Largest allowed `override.size_factor`.
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub max_size_factor: Decimal,
    /// Smallest allowed `gate.evidence.min_oos_trades`.
    pub min_oos_trades: u32,
    /// Ceiling for `execution.approval_ttl_secs`.
    pub max_approval_ttl_secs: u32,
    /// Ceiling for `execution.max_quote_drift_bps`.
    pub max_quote_drift_bps: u32,
    /// Ceiling for `override.max_per_day`.
    pub max_override_per_day: u32,
    /// Ceiling for `override.max_per_week`.
    pub max_override_per_week: u32,
    /// Smallest allowed `exposure.correlation_window_days` (G05).
    pub min_correlation_window_days: u32,
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

/// `[options]`: G13.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    /// Loader refuses `true` (needs a config change, an ADR and the owner's opt-in).
    pub allow_naked_short: bool,
}

/// `[events]`: G06 and G07.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Events {
    pub block_ban_list: bool,
    pub ban_list_max_age_hours: u32,
    pub block_results_days: bool,
    pub block_policy_days: bool,
    /// IST wall time, `HH:MM`.
    #[serde(deserialize_with = "hhmm")]
    pub expiry_day_no_new_after_ist: NaiveTime,
    /// Relative path of the events file under the config dir.
    #[serde(rename = "events_file")]
    pub file: String,
}

/// A relative path whose components are all plain names (no `..`, root, drive prefix, NUL or backslash).
fn relative_normal_path(f: &str) -> bool {
    use std::path::{Component, Path};
    !f.is_empty()
        && !f.contains(['\0', '\\'])
        && Path::new(f)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

fn hhmm<'de, D: serde::Deserializer<'de>>(d: D) -> Result<NaiveTime, D::Error> {
    let s = String::deserialize(d)?;
    NaiveTime::parse_from_str(&s, "%H:%M").map_err(serde::de::Error::custom)
}

/// `[data]`: G15 and the snapshot age limit.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataSection {
    pub max_quote_age_secs: u32,
    /// Own key (RG-F6-15): a journal-built snapshot older than this is Reject.
    pub max_snapshot_age_secs: u32,
}

/// `[gate.evidence]`: G01.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub min_oos_trades: u32,
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub min_profit_factor: Decimal,
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub min_expectancy_r: Decimal,
    pub require_walk_forward_pass: bool,
}

/// `[gate]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateSection {
    pub evidence: Evidence,
}

/// `[override]`: section 3 of the discipline skill.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverrideSection {
    pub max_per_day: u32,
    pub max_per_week: u32,
    /// Fraction of the normal size an overridden trade gets, in (0,1].
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub size_factor: Decimal,
    pub confirm_delay_secs: u32,
}

/// `[execution]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionSection {
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub paper_starting_cash_inr: Decimal,
    /// Seconds an approval request stays open (F10). Unverified, owner to confirm.
    pub approval_ttl_secs: u32,
    /// Largest move of the fresh quote from the intent price, in bps, that an approval accepts.
    pub max_quote_drift_bps: u32,
    /// Own cap on orders per second (the minute and day caps live in `[limits]`).
    pub max_orders_per_second: u32,
    /// Phrase the user types to release the kill switch.
    pub kill_switch_release_phrase: String,
    /// While the kill switch is engaged an approved `reduce_only` exit may still be sent.
    pub kill_allow_reduce_only: bool,
    /// When true the daily-loss breaker also engages the kill switch.
    pub breaker_engages_kill_switch: bool,
    /// Paper slippage against the trader, in bps, per segment.
    pub paper_slippage_bps: PaperSlippage,
    /// Largest share of a bar's volume one paper order may take, in (0,1].
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub paper_max_participation: Decimal,
    /// Paper quotes older than this many seconds are stale: no fill.
    pub paper_stale_after_secs: u32,
    /// IST `HH:MM` at which paper MIS positions are squared off.
    #[serde(deserialize_with = "hhmm")]
    pub paper_mis_square_off_ist: NaiveTime,
    /// Seconds one quote read, gate run or send may take before the pipeline gives up
    /// (F10, BZ-205). Default 5, loader accepts 1..=30. Unverified, owner to confirm.
    #[serde(default = "default_io_timeout_secs")]
    pub io_timeout_secs: u32,
}

/// Default for `execution.io_timeout_secs`.
const fn default_io_timeout_secs() -> u32 {
    5
}

impl ExecutionSection {
    /// The I/O timeout as a `Duration` (already range-checked by the loader).
    #[must_use]
    pub const fn io_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.io_timeout_secs as u64)
    }
}

/// `[execution.paper_slippage_bps]`: one value per segment.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaperSlippage {
    pub equity: u32,
    pub equity_derivatives: u32,
    pub currency: u32,
    pub commodity: u32,
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
    pub options: Options,
    pub events: Events,
    pub data: DataSection,
    pub gate: GateSection,
    #[serde(rename = "override")]
    pub override_: OverrideSection,
    pub execution: ExecutionSection,
}

/// Loader sanity bounds (inclusive). They are structural limits that stop a degenerate value from
/// switching a guard off; the trading thresholds themselves live in the file.
/// Structural sanity bounds, not trading thresholds; policy ceilings live in `[hard_limits]`.
const OPEN_POSITIONS: (u32, u32) = (1, 20);
const CORRELATED_POSITIONS: (u32, u32) = (1, 10);
const CORRELATION_WINDOW_DAYS: (u32, u32) = (20, 250);
const COOLOFF_LOSSES: (u32, u32) = (1, 10);
const ENTRY_ZONE_BPS: (u32, u32) = (0, 50);
const REENTRY_MINUTES: (u32, u32) = (1, 480);
const ENTRIES_PER_DAY: (u32, u32) = (1, 20);
const QUOTE_AGE_SECS: (u32, u32) = (1, 60);
const SNAPSHOT_AGE_SECS: (u32, u32) = (1, 300);
const BAN_LIST_HOURS: (u32, u32) = (1, 48);
const ORDERS_PER_MINUTE: (u32, u32) = (1, 600);
const OVERRIDES_PER_DAY: (u32, u32) = (1, 10);
const OVERRIDES_PER_WEEK: (u32, u32) = (1, 50);
const CONFIRM_DELAY_SECS: (u32, u32) = (1, 300);
const MIN_OOS_TRADES: (u32, u32) = (1, 100_000);
const ORDERS_PER_DAY: (u32, u32) = (1, 1000);
/// `execution.io_timeout_secs`: a zero would disable the bound, a huge value would stall the writer.
const IO_TIMEOUT_SECS: (u32, u32) = (1, 30);
/// Cutoff for new expiry-day entries, seconds after midnight IST: within the 09:15..15:30 session.
const EXPIRY_CUTOFF_SECS: (u32, u32) = (9 * 3600 + 15 * 60, 15 * 3600 + 30 * 60);

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

fn forbid_false(key: &str, v: bool, why: &str) -> Result<(), ConfigError> {
    if !v {
        return Err(ConfigError::invalid(RiskConfig::FILE, key, why));
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
        check_pct(
            "hard_limits.max_position_value_pct",
            h.max_position_value_pct,
        )?;
        check_pct("hard_limits.max_sector_pct", h.max_sector_pct)?;
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
            (
                "limits.max_position_value_pct",
                l.max_position_value_pct,
                h.max_position_value_pct,
            ),
            (
                "exposure.max_sector_pct",
                self.exposure.max_sector_pct,
                h.max_sector_pct,
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

    /// The only place the correlation window is bounded (structural range, then RISK-52-7: not below its `[hard_limits]` floor).
    fn validate_correlation_floor(&self) -> Result<(), ConfigError> {
        let (h, e) = (&self.hard_limits, &self.exposure);
        check_range(
            "hard_limits.min_correlation_window_days",
            h.min_correlation_window_days,
            CORRELATION_WINDOW_DAYS,
        )?;
        check_range(
            "exposure.correlation_window_days",
            e.correlation_window_days,
            CORRELATION_WINDOW_DAYS,
        )?;
        check_floor_u32(
            "exposure.correlation_window_days",
            e.correlation_window_days,
            h.min_correlation_window_days,
        )?;
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

    /// G01, G06, G07, G13, G15, G16 and override keys.
    fn validate_gate_keys(&self) -> Result<(), ConfigError> {
        let ev = &self.events;
        let bad = |key: &str, why: &str| Err(ConfigError::invalid(Self::FILE, key, why));
        forbid_true(
            "options.allow_naked_short",
            self.options.allow_naked_short,
            "must be false (G13 needs an ADR and the owner's opt-in)",
        )?;
        for (key, on) in [
            ("events.block_ban_list", ev.block_ban_list),
            ("events.block_results_days", ev.block_results_days),
            ("events.block_policy_days", ev.block_policy_days),
        ] {
            if !on {
                return bad(key, "must be true (this guard cannot be switched off)");
            }
        }
        if !relative_normal_path(&ev.file) {
            return bad(
                "events.events_file",
                "must be a relative path inside config",
            );
        }
        check_range(
            "events.ban_list_max_age_hours",
            ev.ban_list_max_age_hours,
            BAN_LIST_HOURS,
        )?;
        check_range(
            "data.max_quote_age_secs",
            self.data.max_quote_age_secs,
            QUOTE_AGE_SECS,
        )?;
        check_range(
            "data.max_snapshot_age_secs",
            self.data.max_snapshot_age_secs,
            SNAPSHOT_AGE_SECS,
        )?;
        self.validate_evidence_and_override()?;
        self.validate_rate_ceilings()
    }

    /// G01 evidence keys, the charges floor and `[override]`.
    fn validate_evidence_and_override(&self) -> Result<(), ConfigError> {
        let (l, o, e) = (&self.limits, &self.override_, &self.gate.evidence);
        let bad = |key: &str, why: &str| Err(ConfigError::invalid(Self::FILE, key, why));
        check_range(
            "gate.evidence.min_oos_trades",
            e.min_oos_trades,
            MIN_OOS_TRADES,
        )?;
        if e.min_profit_factor < Decimal::ONE {
            return bad(
                "gate.evidence.min_profit_factor",
                "must be at least 1 (a factor below 1 loses money)",
            );
        }
        forbid_false(
            "gate.evidence.require_walk_forward_pass",
            e.require_walk_forward_pass,
            "must be true (the evidence bar needs a walk-forward pass)",
        )?;
        forbid_false(
            "limits.cap_equity_at_starting_cash",
            l.cap_equity_at_starting_cash,
            "must be true (owner decision: size never rises after wins)",
        )?;
        if e.min_expectancy_r <= Decimal::ZERO {
            return bad("gate.evidence.min_expectancy_r", "must be above 0");
        }
        if l.min_charges_slippage_buffer <= Decimal::ZERO {
            return bad("limits.min_charges_slippage_buffer", "must be above 0");
        }
        if o.size_factor <= Decimal::ZERO || o.size_factor > Decimal::ONE {
            return bad("override.size_factor", "must be above 0 and at most 1");
        }
        check_range(
            "override.confirm_delay_secs",
            o.confirm_delay_secs,
            CONFIRM_DELAY_SECS,
        )?;
        if o.max_per_week < o.max_per_day {
            return bad("override.max_per_week", "must not be below max_per_day");
        }
        Ok(())
    }

    /// Ceilings and floors for the No-row freshness keys and the override and evidence knobs.
    fn validate_no_row_ceilings(&self) -> Result<(), ConfigError> {
        let (h, o, ev, d) = (&self.hard_limits, &self.override_, &self.events, &self.data);
        for (key, v, r) in [
            (
                "hard_limits.max_ban_list_age_hours",
                h.max_ban_list_age_hours,
                BAN_LIST_HOURS,
            ),
            (
                "hard_limits.max_quote_age_secs",
                h.max_quote_age_secs,
                QUOTE_AGE_SECS,
            ),
            (
                "hard_limits.max_snapshot_age_secs",
                h.max_snapshot_age_secs,
                SNAPSHOT_AGE_SECS,
            ),
            (
                "hard_limits.min_confirm_delay_secs",
                h.min_confirm_delay_secs,
                CONFIRM_DELAY_SECS,
            ),
            (
                "hard_limits.min_oos_trades",
                h.min_oos_trades,
                MIN_OOS_TRADES,
            ),
        ] {
            check_range(key, v, r)?;
        }
        for (key, t) in [
            (
                "hard_limits.latest_expiry_day_no_new_after_ist",
                h.latest_expiry_day_no_new_after_ist,
            ),
            (
                "events.expiry_day_no_new_after_ist",
                ev.expiry_day_no_new_after_ist,
            ),
        ] {
            check_range(key, t.num_seconds_from_midnight(), EXPIRY_CUTOFF_SECS)?;
        }
        check_ceiling_u32(
            "events.ban_list_max_age_hours",
            ev.ban_list_max_age_hours,
            h.max_ban_list_age_hours,
        )?;
        check_ceiling_u32(
            "data.max_quote_age_secs",
            d.max_quote_age_secs,
            h.max_quote_age_secs,
        )?;
        check_ceiling_u32(
            "data.max_snapshot_age_secs",
            d.max_snapshot_age_secs,
            h.max_snapshot_age_secs,
        )?;
        check_floor_u32(
            "override.confirm_delay_secs",
            o.confirm_delay_secs,
            h.min_confirm_delay_secs,
        )?;
        check_floor_u32(
            "gate.evidence.min_oos_trades",
            self.gate.evidence.min_oos_trades,
            h.min_oos_trades,
        )?;
        if h.max_size_factor <= Decimal::ZERO || h.max_size_factor > Decimal::ONE {
            return Err(ConfigError::invalid(
                Self::FILE,
                "hard_limits.max_size_factor",
                "must be above 0 and at most 1",
            ));
        }
        check_ceiling("override.size_factor", o.size_factor, h.max_size_factor)?;
        if ev.expiry_day_no_new_after_ist > h.latest_expiry_day_no_new_after_ist {
            return Err(ConfigError::invalid(
                Self::FILE,
                "events.expiry_day_no_new_after_ist",
                "is later than its [hard_limits] ceiling",
            ));
        }
        Ok(())
    }

    /// G16 orders per day and notional per order, with their ceilings.
    fn validate_order_limits(&self) -> Result<(), ConfigError> {
        let (l, h) = (&self.limits, &self.hard_limits);
        check_range(
            "hard_limits.max_orders_per_day",
            h.max_orders_per_day,
            ORDERS_PER_DAY,
        )?;
        check_range(
            "limits.max_orders_per_day",
            l.max_orders_per_day,
            ORDERS_PER_DAY,
        )?;
        check_ceiling_u32(
            "limits.max_orders_per_day",
            l.max_orders_per_day,
            h.max_orders_per_day,
        )?;
        let (n, c) = (l.max_notional_per_order_inr, h.max_notional_per_order_inr);
        if n <= Decimal::ZERO || c <= Decimal::ZERO {
            return Err(ConfigError::invalid(
                Self::FILE,
                "limits.max_notional_per_order_inr",
                "must be above 0 (limit and ceiling)",
            ));
        }
        check_ceiling("limits.max_notional_per_order_inr", n, c)
    }

    /// G04a, G16 and override-count ceilings and their ranges.
    fn validate_rate_ceilings(&self) -> Result<(), ConfigError> {
        let (l, h, o) = (&self.limits, &self.hard_limits, &self.override_);
        check_range(
            "hard_limits.max_open_positions",
            h.max_open_positions,
            OPEN_POSITIONS,
        )?;
        check_ceiling_u32(
            "limits.max_open_positions",
            l.max_open_positions,
            h.max_open_positions,
        )?;
        self.validate_order_limits()?;
        self.validate_no_row_ceilings()?;
        for (key, v, c, r) in [
            (
                "hard_limits.max_orders_per_minute",
                h.max_orders_per_minute,
                None,
                ORDERS_PER_MINUTE,
            ),
            (
                "hard_limits.max_override_per_day",
                h.max_override_per_day,
                None,
                OVERRIDES_PER_DAY,
            ),
            (
                "hard_limits.max_override_per_week",
                h.max_override_per_week,
                None,
                OVERRIDES_PER_WEEK,
            ),
            (
                "limits.max_orders_per_minute",
                l.max_orders_per_minute,
                Some(h.max_orders_per_minute),
                ORDERS_PER_MINUTE,
            ),
            (
                "override.max_per_day",
                o.max_per_day,
                Some(h.max_override_per_day),
                OVERRIDES_PER_DAY,
            ),
            (
                "override.max_per_week",
                o.max_per_week,
                Some(h.max_override_per_week),
                OVERRIDES_PER_WEEK,
            ),
        ] {
            check_range(key, v, r)?;
            if let Some(c) = c {
                check_ceiling_u32(key, v, c)?;
            }
        }
        Ok(())
    }
}

impl ConfigFile for RiskConfig {
    const FILE: &'static str = "risk.toml";
    impl_header!();

    fn validate(&self) -> Result<(), ConfigError> {
        self.validate_percentages()?;
        self.validate_guards()?;
        self.validate_correlation_floor()?;
        self.validate_gate_keys()?;
        if self.execution.paper_starting_cash_inr <= Decimal::ZERO {
            return Err(ConfigError::invalid(
                Self::FILE,
                "execution.paper_starting_cash_inr",
                "must be above 0",
            ));
        }
        self.validate_execution_keys()
    }
}

impl RiskConfig {
    /// F10 `[execution]` keys: positive windows, caps within the `[hard_limits]` caps.
    fn validate_execution_keys(&self) -> Result<(), ConfigError> {
        let e = &self.execution;
        let bad = |key: &str, why: &str| Err(ConfigError::invalid(Self::FILE, key, why));
        for (key, v) in [
            ("execution.approval_ttl_secs", e.approval_ttl_secs),
            ("execution.max_quote_drift_bps", e.max_quote_drift_bps),
            ("execution.max_orders_per_second", e.max_orders_per_second),
            ("execution.paper_stale_after_secs", e.paper_stale_after_secs),
            (
                "execution.paper_slippage_bps.equity",
                e.paper_slippage_bps.equity,
            ),
            (
                "execution.paper_slippage_bps.equity_derivatives",
                e.paper_slippage_bps.equity_derivatives,
            ),
            (
                "execution.paper_slippage_bps.currency",
                e.paper_slippage_bps.currency,
            ),
            (
                "execution.paper_slippage_bps.commodity",
                e.paper_slippage_bps.commodity,
            ),
        ] {
            if v == 0 {
                return bad(key, "must be above 0");
            }
        }
        check_ceiling_u32(
            "execution.approval_ttl_secs",
            e.approval_ttl_secs,
            self.hard_limits.max_approval_ttl_secs,
        )?;
        check_ceiling_u32(
            "execution.max_quote_drift_bps",
            e.max_quote_drift_bps,
            self.hard_limits.max_quote_drift_bps,
        )?;
        // G15: paper must not fill on quotes the gate would refuse. The per-minute and per-day
        // caps in [limits] always apply in addition to the per-second cap.
        if e.paper_stale_after_secs > self.data.max_quote_age_secs {
            return bad(
                "execution.paper_stale_after_secs",
                "must not exceed data.max_quote_age_secs (G15)",
            );
        }
        if e.kill_switch_release_phrase.trim().len() < 8 {
            return bad(
                "execution.kill_switch_release_phrase",
                "must be at least 8 characters",
            );
        }
        check_range(
            "execution.io_timeout_secs",
            e.io_timeout_secs,
            IO_TIMEOUT_SECS,
        )?;
        let p = e.paper_max_participation;
        if p <= Decimal::ZERO || p > Decimal::ONE {
            return bad("execution.paper_max_participation", "must be in (0, 1]");
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
        max_orders_per_minute = 10\nmax_orders_per_day = 12\n\
        max_notional_per_order_inr = \"25000\"\nmin_charges_slippage_buffer = \"60\"\n\
        cap_equity_at_starting_cash = true\n\
        [hard_limits]\nmax_risk_per_trade_pct = \"1\"\nmax_daily_loss_pct = \"3\"\n\
        max_total_open_risk_pct = \"3\"\nmax_weekly_loss_pct = \"6\"\n\
        max_cooloff_after_losses = 2\nmax_new_entries_per_day = 3\n\
        min_reentry_lockout_minutes = 60\nmax_entry_zone_tolerance_bps = 0\n\
        max_position_value_pct = \"25\"\nmax_sector_pct = \"30\"\nmax_open_positions = 3\n\
        max_orders_per_minute = 10\nmax_orders_per_day = 12\n\
        max_notional_per_order_inr = \"25000\"\nmax_override_per_day = 1\nmax_override_per_week = 3\n\
        min_correlation_window_days = 20\n\
        max_ban_list_age_hours = 24\nmax_quote_age_secs = 5\nmax_snapshot_age_secs = 10\n\
        latest_expiry_day_no_new_after_ist = \"13:30\"\nmin_confirm_delay_secs = 10\n\
        max_size_factor = \"0.5\"\nmin_oos_trades = 30\n\
        max_approval_ttl_secs = 3600\nmax_quote_drift_bps = 500\n\
        [exposure]\nmax_sector_pct = \"30\"\nmax_correlated_positions = 2\n\
        correlation_threshold = \"0.7\"\ncorrelation_window_days = 60\n\
        [behaviour]\ncooloff_after_losses = 2\ncooloff_until = \"end_of_session\"\n\
        allow_stop_widening = false\nallow_averaging_down = false\n\
        entry_zone_tolerance_bps = 0\nreentry_lockout_minutes = 60\n\
        max_new_entries_per_day = 3\n\
        [execution]\npaper_starting_cash_inr = \"100000\"\n\
        approval_ttl_secs = 300\nmax_quote_drift_bps = 50\nmax_orders_per_second = 1\n\
        kill_switch_release_phrase = \"RELEASE KILL SWITCH\"\nkill_allow_reduce_only = true\nbreaker_engages_kill_switch = false\n\
        paper_max_participation = \"0.1\"\npaper_stale_after_secs = 5\n\
        paper_mis_square_off_ist = \"15:15\"\n\
        [execution.paper_slippage_bps]\nequity = 5\nequity_derivatives = 5\ncurrency = 2\ncommodity = 5\n\
        [options]\nallow_naked_short = false\n\
        [events]\nblock_ban_list = true\nban_list_max_age_hours = 24\n\
        block_results_days = true\nblock_policy_days = true\n\
        expiry_day_no_new_after_ist = \"13:30\"\nevents_file = \"events.toml\"\n\
        [data]\nmax_quote_age_secs = 5\nmax_snapshot_age_secs = 10\n\
        [gate.evidence]\nmin_oos_trades = 30\nmin_profit_factor = \"1.3\"\n\
        min_expectancy_r = \"0.1\"\nrequire_walk_forward_pass = true\n\
        [override]\nmax_per_day = 1\nmax_per_week = 3\nsize_factor = \"0.5\"\n\
        confirm_delay_secs = 10\n";

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

    /// `OK` with `key` in `section` set to `val` (`None` removes the line).
    fn edit(section: &str, key: &str, val: Option<&str>) -> String {
        edit_in(OK, section, key, val)
    }

    fn edit_in(base: &str, section: &str, key: &str, val: Option<&str>) -> String {
        let mut cur = String::new();
        let mut out = Vec::new();
        let mut hit = false;
        for line in base.lines() {
            if line.starts_with('[') {
                cur = line.trim_matches(|c| c == '[' || c == ']').to_string();
            }
            if cur == section && line.split(" = ").next() == Some(key) {
                hit = true;
                if let Some(v) = val {
                    out.push(format!("{key} = {v}"));
                }
                continue;
            }
            out.push(line.to_string());
        }
        assert!(hit, "{section}.{key} not in OK");
        out.join("\n") + "\n"
    }

    const NEW_KEYS: &[(&str, &str)] = &[
        ("limits", "on_daily_loss"),
        ("limits", "max_open_positions"),
        ("limits", "max_total_open_risk_pct"),
        ("limits", "max_weekly_loss_pct"),
        ("limits", "max_position_value_pct"),
        ("hard_limits", "max_total_open_risk_pct"),
        ("hard_limits", "max_weekly_loss_pct"),
        ("hard_limits", "max_cooloff_after_losses"),
        ("hard_limits", "max_new_entries_per_day"),
        ("hard_limits", "min_reentry_lockout_minutes"),
        ("hard_limits", "max_entry_zone_tolerance_bps"),
        ("exposure", "max_sector_pct"),
        ("exposure", "max_correlated_positions"),
        ("exposure", "correlation_threshold"),
        ("exposure", "correlation_window_days"),
        ("behaviour", "cooloff_after_losses"),
        ("behaviour", "cooloff_until"),
        ("behaviour", "allow_stop_widening"),
        ("behaviour", "allow_averaging_down"),
        ("behaviour", "entry_zone_tolerance_bps"),
        ("behaviour", "reentry_lockout_minutes"),
        ("behaviour", "max_new_entries_per_day"),
        ("limits", "max_orders_per_minute"),
        ("limits", "min_charges_slippage_buffer"),
        ("limits", "cap_equity_at_starting_cash"),
        ("limits", "max_orders_per_day"),
        ("limits", "max_notional_per_order_inr"),
        ("hard_limits", "max_orders_per_day"),
        ("hard_limits", "max_notional_per_order_inr"),
        ("hard_limits", "max_position_value_pct"),
        ("hard_limits", "max_sector_pct"),
        ("hard_limits", "max_open_positions"),
        ("hard_limits", "max_orders_per_minute"),
        ("hard_limits", "max_override_per_day"),
        ("hard_limits", "max_override_per_week"),
        ("options", "allow_naked_short"),
        ("events", "block_ban_list"),
        ("events", "ban_list_max_age_hours"),
        ("events", "block_results_days"),
        ("events", "block_policy_days"),
        ("events", "expiry_day_no_new_after_ist"),
        ("events", "events_file"),
        ("data", "max_quote_age_secs"),
        ("data", "max_snapshot_age_secs"),
        ("gate.evidence", "min_oos_trades"),
        ("gate.evidence", "min_profit_factor"),
        ("gate.evidence", "min_expectancy_r"),
        ("gate.evidence", "require_walk_forward_pass"),
        ("override", "max_per_day"),
        ("override", "max_per_week"),
        ("override", "size_factor"),
        ("override", "confirm_delay_secs"),
        ("hard_limits", "max_ban_list_age_hours"),
        ("hard_limits", "max_quote_age_secs"),
        ("hard_limits", "max_snapshot_age_secs"),
        ("hard_limits", "latest_expiry_day_no_new_after_ist"),
        ("hard_limits", "min_confirm_delay_secs"),
        ("hard_limits", "max_size_factor"),
        ("hard_limits", "min_oos_trades"),
        ("hard_limits", "min_correlation_window_days"),
    ];

    /// Each new key removed on its own must fail the load (no serde default to a loose value).
    #[test]
    fn every_new_key_is_required() {
        for (section, key) in NEW_KEYS {
            let text = edit(section, key, None);
            assert!(
                parse_str::<RiskConfig>(&text).is_err(),
                "missing {section}.{key} must fail"
            );
        }
        for sect in ["[exposure]", "[behaviour]"] {
            assert!(parse_str::<RiskConfig>(&OK.replacen(sect, "[zzz]", 1)).is_err());
        }
    }

    #[test]
    fn values_at_the_boundary_are_accepted() {
        let ok: &[(&str, &str, &str)] = &[
            ("hard_limits", "max_position_value_pct", "\"100\""),
            ("hard_limits", "max_sector_pct", "\"100\""),
            ("exposure", "correlation_threshold", "\"1\""),
            ("exposure", "max_correlated_positions", "1"),
            ("exposure", "max_correlated_positions", "10"),
            ("exposure", "correlation_window_days", "20"),
            ("exposure", "correlation_window_days", "250"),
            ("limits", "max_open_positions", "1"),
            ("hard_limits", "max_open_positions", "20"),
            ("hard_limits", "max_total_open_risk_pct", "\"100\""),
            ("hard_limits", "max_weekly_loss_pct", "\"100\""),
            ("hard_limits", "max_cooloff_after_losses", "10"),
            ("hard_limits", "max_new_entries_per_day", "20"),
            ("hard_limits", "max_entry_zone_tolerance_bps", "50"),
            ("behaviour", "cooloff_after_losses", "1"),
            ("behaviour", "cooloff_after_losses", "2"),
            ("behaviour", "max_new_entries_per_day", "1"),
            ("behaviour", "max_new_entries_per_day", "3"),
            ("behaviour", "reentry_lockout_minutes", "60"),
            ("behaviour", "entry_zone_tolerance_bps", "0"),
        ];
        for (sec, key, v) in ok {
            let text = edit(sec, key, Some(v));
            // Raising a hard ceiling to its bound can leave the value below it: still valid.
            assert!(parse_str::<RiskConfig>(&text).is_ok(), "{sec}.{key} = {v}");
        }
        // Limit equal to a raised ceiling, and the top of every range for the behaviour keys.
        let t = edit("limits", "max_weekly_loss_pct", Some("\"100\""));
        let t = t.replacen(
            "max_weekly_loss_pct = \"6\"\nmax_cooloff",
            "max_weekly_loss_pct = \"100\"\nmax_cooloff",
            1,
        );
        assert!(parse_str::<RiskConfig>(&t).is_ok());
        let t = edit("hard_limits", "max_cooloff_after_losses", Some("10"));
        let t = t.replacen(
            "[behaviour]\ncooloff_after_losses = 2",
            "[behaviour]\ncooloff_after_losses = 10",
            1,
        );
        assert!(parse_str::<RiskConfig>(&t).is_ok());
        let t = edit("hard_limits", "min_reentry_lockout_minutes", Some("480"));
        let t = t.replacen(
            "reentry_lockout_minutes = 60\nmax_new",
            "reentry_lockout_minutes = 480\nmax_new",
            1,
        );
        assert!(parse_str::<RiskConfig>(&t).is_ok());
    }

    #[test]
    fn hard_ceilings_are_range_checked() {
        let bad: &[(&str, &str, &str)] = &[
            (
                "max_total_open_risk_pct",
                "\"0\"",
                "hard_limits.max_total_open_risk_pct",
            ),
            (
                "max_total_open_risk_pct",
                "\"100.1\"",
                "hard_limits.max_total_open_risk_pct",
            ),
            (
                "max_weekly_loss_pct",
                "\"0\"",
                "hard_limits.max_weekly_loss_pct",
            ),
            (
                "max_weekly_loss_pct",
                "\"100.1\"",
                "hard_limits.max_weekly_loss_pct",
            ),
            (
                "max_cooloff_after_losses",
                "11",
                "hard_limits.max_cooloff_after_losses",
            ),
            (
                "max_new_entries_per_day",
                "21",
                "hard_limits.max_new_entries_per_day",
            ),
            (
                "min_reentry_lockout_minutes",
                "481",
                "hard_limits.min_reentry_lockout_minutes",
            ),
        ];
        for (key, v, want) in bad {
            let err = parse_str::<RiskConfig>(&edit("hard_limits", key, Some(v))).expect_err(key);
            assert!(err.to_string().contains(want), "{key} = {v}: {err}");
        }
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

    /// (section, key, bad value); the error must name the key.
    #[test]
    fn slice1_table_refuses_degenerate_and_loose_values() {
        let rows: &[(&str, &str, &str)] = &[
            ("limits", "max_orders_per_day", "0"),
            ("limits", "max_orders_per_day", "13"),
            ("limits", "max_notional_per_order_inr", "\"0\""),
            ("limits", "max_notional_per_order_inr", "\"25001\""),
            ("hard_limits", "max_orders_per_day", "0"),
            ("hard_limits", "max_orders_per_day", "1001"),
            ("hard_limits", "max_notional_per_order_inr", "\"0\""),
            ("hard_limits", "max_orders_per_minute", "601"),
            ("hard_limits", "max_override_per_day", "11"),
            ("hard_limits", "max_open_positions", "21"),
            ("hard_limits", "max_sector_pct", "\"0\""),
            ("hard_limits", "max_position_value_pct", "\"101\""),
            ("gate.evidence", "min_profit_factor", "\"0.99\""),
            ("gate.evidence", "require_walk_forward_pass", "false"),
            ("limits", "cap_equity_at_starting_cash", "false"),
            ("events", "events_file", "\"a\\\\b.toml\""),
            ("events", "events_file", "\"a/../b.toml\""),
            ("limits", "max_orders_per_minute", "0"),
            ("limits", "max_orders_per_minute", "601"),
            ("limits", "max_orders_per_minute", "11"),
            ("limits", "min_charges_slippage_buffer", "\"0\""),
            ("limits", "max_position_value_pct", "\"26\""),
            ("limits", "max_open_positions", "4"),
            ("exposure", "max_sector_pct", "\"31\""),
            ("hard_limits", "max_orders_per_minute", "0"),
            ("hard_limits", "max_open_positions", "0"),
            ("hard_limits", "max_override_per_day", "0"),
            ("hard_limits", "max_override_per_week", "51"),
            ("hard_limits", "max_position_value_pct", "\"0\""),
            ("hard_limits", "max_sector_pct", "\"101\""),
            ("options", "allow_naked_short", "true"),
            ("events", "block_ban_list", "false"),
            ("events", "block_results_days", "false"),
            ("events", "block_policy_days", "false"),
            ("events", "ban_list_max_age_hours", "0"),
            ("events", "ban_list_max_age_hours", "49"),
            ("events", "events_file", "\"\""),
            ("events", "events_file", "\"/etc/x.toml\""),
            ("events", "events_file", "\"../x.toml\""),
            ("data", "max_quote_age_secs", "0"),
            ("data", "max_quote_age_secs", "61"),
            ("data", "max_snapshot_age_secs", "0"),
            ("data", "max_snapshot_age_secs", "301"),
            ("gate.evidence", "min_oos_trades", "0"),
            ("gate.evidence", "min_oos_trades", "100001"),
            ("gate.evidence", "min_profit_factor", "\"0\""),
            ("gate.evidence", "min_expectancy_r", "\"-0.1\""),
            ("events", "ban_list_max_age_hours", "25"),
            ("data", "max_quote_age_secs", "6"),
            ("data", "max_snapshot_age_secs", "11"),
            ("gate.evidence", "min_oos_trades", "29"),
            ("override", "confirm_delay_secs", "9"),
            ("override", "size_factor", "\"0.51\""),
            ("hard_limits", "max_ban_list_age_hours", "0"),
            ("hard_limits", "max_ban_list_age_hours", "49"),
            ("hard_limits", "max_quote_age_secs", "0"),
            ("hard_limits", "max_quote_age_secs", "61"),
            ("hard_limits", "max_snapshot_age_secs", "301"),
            ("hard_limits", "min_confirm_delay_secs", "0"),
            ("hard_limits", "min_confirm_delay_secs", "301"),
            ("hard_limits", "max_size_factor", "\"0\""),
            ("hard_limits", "max_size_factor", "\"1.1\""),
            ("hard_limits", "min_oos_trades", "0"),
            ("hard_limits", "min_correlation_window_days", "19"),
            ("hard_limits", "min_correlation_window_days", "251"),
            ("exposure", "correlation_window_days", "19"),
            ("override", "max_per_day", "0"),
            ("override", "max_per_day", "2"),
            ("override", "max_per_week", "4"),
            ("override", "size_factor", "\"0\""),
            ("override", "size_factor", "\"1.01\""),
            ("override", "size_factor", "\"-0.5\""),
            ("override", "confirm_delay_secs", "0"),
            ("override", "confirm_delay_secs", "301"),
        ];
        for (sec, key, bad) in rows {
            let err = parse_str::<RiskConfig>(&edit(sec, key, Some(bad)))
                .expect_err(&format!("{sec}.{key} = {bad}"));
            assert!(err.to_string().contains(key), "{sec}.{key} = {bad}: {err}");
        }
    }

    /// RISK-52-7: a raised floor refuses a window the file itself would otherwise accept.
    #[test]
    fn correlation_window_below_raised_floor_is_refused_naming_the_key() {
        let t = edit("hard_limits", "min_correlation_window_days", Some("70"));
        let err = parse_str::<RiskConfig>(&t).unwrap_err().to_string();
        assert!(err.contains("exposure.correlation_window_days"), "{err}");
        let t = edit("hard_limits", "min_correlation_window_days", Some("60"));
        assert!(parse_str::<RiskConfig>(&t).is_ok());
    }

    #[test]
    fn slice1_week_override_cap_not_below_day_cap() {
        let t = edit("override", "max_per_day", Some("2"));
        let t = t.replace("max_override_per_day = 1", "max_override_per_day = 2");
        let t = t.replace("max_override_per_week = 3", "max_override_per_week = 1");
        let t = t.replace("max_per_week = 3", "max_per_week = 1");
        assert!(
            parse_str::<RiskConfig>(&t)
                .unwrap_err()
                .to_string()
                .contains("must not be below")
        );
    }

    #[test]
    fn slice1b_extremes_pass_when_ceiling_and_limit_both_sit_there() {
        let mut t = OK.to_string();
        for (sec, key, v) in [
            ("hard_limits", "max_size_factor", "\"1\""),
            ("override", "size_factor", "\"1\""),
            ("hard_limits", "max_quote_age_secs", "60"),
            ("data", "max_quote_age_secs", "60"),
            ("hard_limits", "max_snapshot_age_secs", "300"),
            ("data", "max_snapshot_age_secs", "300"),
            ("hard_limits", "min_oos_trades", "1"),
            ("gate.evidence", "min_oos_trades", "1"),
            ("exposure", "correlation_window_days", "20"),
            ("hard_limits", "min_confirm_delay_secs", "300"),
            ("override", "confirm_delay_secs", "300"),
            ("hard_limits", "max_ban_list_age_hours", "48"),
            ("events", "ban_list_max_age_hours", "48"),
        ] {
            t = edit_in(&t, sec, key, Some(v));
        }
        assert!(parse_str::<RiskConfig>(&t).is_ok());
    }

    #[test]
    fn slice1b_expiry_cutoff_must_stay_in_session() {
        for bad in ["\"09:14\"", "\"15:31\"", "\"23:59\"", "\"00:00\""] {
            let t = edit(
                "hard_limits",
                "latest_expiry_day_no_new_after_ist",
                Some(bad),
            );
            assert!(parse_str::<RiskConfig>(&t).is_err(), "hard {bad}");
            let t = edit("events", "expiry_day_no_new_after_ist", Some(bad));
            assert!(parse_str::<RiskConfig>(&t).is_err(), "limit {bad}");
        }
        let t = edit(
            "hard_limits",
            "latest_expiry_day_no_new_after_ist",
            Some("\"15:30\""),
        );
        assert!(parse_str::<RiskConfig>(&t).is_ok());
    }

    #[test]
    fn slice1b_expiry_cutoff_ceiling() {
        let ok = edit("events", "expiry_day_no_new_after_ist", Some("\"13:30\""));
        assert!(parse_str::<RiskConfig>(&ok).is_ok());
        let late = edit("events", "expiry_day_no_new_after_ist", Some("\"13:31\""));
        let err = parse_str::<RiskConfig>(&late).unwrap_err().to_string();
        assert!(err.contains("expiry_day_no_new_after_ist"), "{err}");
    }

    #[test]
    fn slice1_bad_time_and_wrong_types_are_refused() {
        for bad in ["\"1330\"", "\"25:00\"", "13"] {
            let t = edit("events", "expiry_day_no_new_after_ist", Some(bad));
            assert!(parse_str::<RiskConfig>(&t).is_err(), "{bad}");
        }
        let t = edit(
            "hard_limits",
            "latest_expiry_day_no_new_after_ist",
            Some("\"25:00\""),
        );
        assert!(parse_str::<RiskConfig>(&t).is_err());
        // Money-like keys must be strings, not TOML floats.
        let t = edit("override", "size_factor", Some("0.5"));
        assert!(parse_str::<RiskConfig>(&t).is_err());
    }

    #[test]
    fn slice1_keys_parse_and_boundaries_pass() {
        let cfg = parse_str::<RiskConfig>(OK).unwrap();
        assert_eq!(cfg.override_.size_factor.to_string(), "0.5");
        assert_eq!(cfg.data.max_snapshot_age_secs, 10);
        assert_eq!(
            cfg.events.expiry_day_no_new_after_ist.to_string(),
            "13:30:00"
        );
        assert!(cfg.limits.cap_equity_at_starting_cash);
        for (sec, key, ok) in [
            ("override", "size_factor", "\"0.5\""),
            ("override", "confirm_delay_secs", "10"),
            ("data", "max_quote_age_secs", "5"),
            ("data", "max_snapshot_age_secs", "10"),
            ("gate.evidence", "min_oos_trades", "30"),
            ("hard_limits", "max_quote_age_secs", "60"),
            ("hard_limits", "min_confirm_delay_secs", "1"),
            ("hard_limits", "max_size_factor", "\"1\""),
            ("limits", "max_orders_per_minute", "1"),
            ("limits", "max_position_value_pct", "\"25\""),
        ] {
            assert!(
                parse_str::<RiskConfig>(&edit(sec, key, Some(ok))).is_ok(),
                "{sec}.{key} = {ok}"
            );
        }
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
        assert_eq!(cfg.hard_limits.max_position_value_pct, Decimal::from(25));
        assert_eq!(cfg.hard_limits.max_sector_pct, Decimal::from(30));
        assert_eq!(cfg.hard_limits.max_open_positions, 3);
        assert_eq!(cfg.override_.max_per_day, 1);
        assert_eq!(cfg.verified_on, NaiveDate::from_ymd_opt(2026, 11, 18));
    }

    #[test]
    fn f10_execution_keys_parse_and_are_validated() {
        let cfg = parse_str::<RiskConfig>(OK).unwrap();
        let e = &cfg.execution;
        assert_eq!((e.approval_ttl_secs, e.max_quote_drift_bps), (300, 50));
        assert_eq!(e.paper_slippage_bps.currency, 2);
        assert_eq!(e.paper_mis_square_off_ist.to_string(), "15:15:00");
        for (from, to) in [
            ("approval_ttl_secs = 300", "approval_ttl_secs = 0"),
            ("max_orders_per_second = 1", "max_orders_per_second = 0"),
            ("approval_ttl_secs = 300", "approval_ttl_secs = 3601"),
            (
                "\nmax_quote_drift_bps = 50\n",
                "\nmax_quote_drift_bps = 501\n",
            ),
            (
                "\nmax_quote_drift_bps = 50\n",
                "\nmax_quote_drift_bps = 0\n",
            ),
            ("equity = 5", "equity = 0"),
            ("currency = 2", "currency = 0"),
            ("paper_stale_after_secs = 5", "paper_stale_after_secs = 6"),
            ("RELEASE KILL SWITCH", "short"),
            (
                "paper_max_participation = \"0.1\"",
                "paper_max_participation = \"1.5\"",
            ),
            ("paper_stale_after_secs = 5", "paper_stale_after_secs = 0"),
        ] {
            assert!(
                parse_str::<RiskConfig>(&OK.replace(from, to)).is_err(),
                "{from}"
            );
        }
    }

    #[test]
    fn io_timeout_defaults_to_5_and_accepts_1_to_30() {
        let cfg: RiskConfig = parse_str(OK).unwrap();
        assert_eq!(cfg.execution.io_timeout_secs, 5);
        assert_eq!(
            cfg.execution.io_timeout(),
            std::time::Duration::from_secs(5)
        );
        for v in ["1", "30"] {
            let text = with(
                "[execution]\n",
                &format!("[execution]\nio_timeout_secs = {v}\n"),
            );
            let cfg: RiskConfig = parse_str(&text).unwrap();
            assert_eq!(cfg.execution.io_timeout_secs, v.parse::<u32>().unwrap());
        }
    }

    #[test]
    fn io_timeout_zero_and_31_are_refused_naming_the_key() {
        for v in ["0", "31", "3600"] {
            let text = with(
                "[execution]\n",
                &format!("[execution]\nio_timeout_secs = {v}\n"),
            );
            let err = parse_str::<RiskConfig>(&text).unwrap_err().to_string();
            assert!(err.contains("execution.io_timeout_secs"), "{v}: {err}");
        }
    }
}
