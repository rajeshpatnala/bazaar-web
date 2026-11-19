//! `GateConfig` (validated `risk.toml` plus its hash), `EffectiveLimits` (file value combined with
//! the baseline, stricter wins) and `RuleCtx` (what rules see: effective limits only).

use bazaar_core::config::{ConfigError, CooloffUntil, OnDailyLoss, RiskConfig, parse_str};
use bazaar_core::mode::ExecutionMode;
use chrono::{DateTime, NaiveTime};
use chrono_tz::Tz;
use rust_decimal::Decimal;
use sha2::{Digest, Sha256};
use std::fmt::{self, Write as _};

/// SHA-256 of the raw `risk.toml` text, lower-case hex. Same text, same hash.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConfigHash(String);

impl ConfigHash {
    /// Hash of the file text exactly as read.
    #[must_use]
    pub fn of_text(text: &str) -> Self {
        let digest = Sha256::digest(text.as_bytes());
        let mut hex = String::with_capacity(64);
        for byte in &digest {
            let _ = write!(hex, "{byte:02x}");
        }
        Self(hex)
    }

    /// The hex string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ConfigHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Validated config. No `Default`; only built from file text through the core loader.
#[derive(Debug, Clone)]
pub struct GateConfig {
    risk: RiskConfig,
    hash: ConfigHash,
}

impl GateConfig {
    /// Parse and validate `risk.toml` text (missing key, ceiling breach, degenerate value = error).
    pub fn from_toml_str(text: &str) -> Result<Self, ConfigError> {
        let risk = parse_str::<RiskConfig>(text)?;
        Ok(Self {
            risk,
            hash: ConfigHash::of_text(text),
        })
    }

    /// Hash of the text this config was parsed from.
    #[must_use]
    pub const fn hash(&self) -> &ConfigHash {
        &self.hash
    }

    /// The execution mode, read from the file only.
    #[must_use]
    pub const fn mode(&self) -> ExecutionMode {
        self.risk.mode.execution_mode
    }

    /// Combine the file with the baseline: for each tightenable key the stricter value wins.
    #[must_use]
    pub fn effective(&self, baseline: &Baseline) -> EffectiveLimits {
        let r = &self.risk;
        let (lim, beh) = (&r.limits, &r.behaviour);
        let (exp, ovr, dat) = (&r.exposure, &r.override_, &r.data);
        EffectiveLimits {
            max_risk_per_trade_pct: lower_dec(
                lim.max_risk_per_trade_pct,
                baseline.max_risk_per_trade_pct,
            ),
            max_daily_loss_pct: lower_dec(lim.max_daily_loss_pct, baseline.max_daily_loss_pct),
            max_total_open_risk_pct: lower_dec(
                lim.max_total_open_risk_pct,
                baseline.max_total_open_risk_pct,
            ),
            max_weekly_loss_pct: lower_dec(lim.max_weekly_loss_pct, baseline.max_weekly_loss_pct),
            max_position_value_pct: lower_dec(
                lim.max_position_value_pct,
                baseline.max_position_value_pct,
            ),
            max_notional_per_order_inr: lower_dec(
                lim.max_notional_per_order_inr,
                baseline.max_notional_per_order_inr,
            ),
            min_charges_slippage_buffer: higher_dec(
                lim.min_charges_slippage_buffer,
                baseline.min_charges_slippage_buffer,
            ),
            cap_equity_at_starting_cash: lim.cap_equity_at_starting_cash
                || baseline.cap_equity_at_starting_cash == Some(true),
            max_open_positions: lower_u32(lim.max_open_positions, baseline.max_open_positions),
            max_orders_per_minute: lower_u32(
                lim.max_orders_per_minute,
                baseline.max_orders_per_minute,
            ),
            max_orders_per_day: lower_u32(lim.max_orders_per_day, baseline.max_orders_per_day),
            cooloff_after_losses: lower_u32(
                beh.cooloff_after_losses,
                baseline.cooloff_after_losses,
            ),
            max_new_entries_per_day: lower_u32(
                beh.max_new_entries_per_day,
                baseline.max_new_entries_per_day,
            ),
            entry_zone_tolerance_bps: lower_u32(
                beh.entry_zone_tolerance_bps,
                baseline.entry_zone_tolerance_bps,
            ),
            reentry_lockout_minutes: higher_u32(
                beh.reentry_lockout_minutes,
                baseline.reentry_lockout_minutes,
            ),
            max_sector_pct: lower_dec(exp.max_sector_pct, baseline.max_sector_pct),
            override_max_per_day: lower_u32(ovr.max_per_day, baseline.override_max_per_day),
            override_max_per_week: lower_u32(ovr.max_per_week, baseline.override_max_per_week),
            max_quote_age_secs: dat.max_quote_age_secs,
            max_snapshot_age_secs: dat.max_snapshot_age_secs,
            override_size_factor: ovr.size_factor,
            override_confirm_delay_secs: ovr.confirm_delay_secs,
            correlation_threshold: exp.correlation_threshold,
            max_correlated_positions: exp.max_correlated_positions,
            correlation_window_days: exp.correlation_window_days,
            min_oos_trades: r.gate.evidence.min_oos_trades,
            min_profit_factor: r.gate.evidence.min_profit_factor,
            min_expectancy_r: r.gate.evidence.min_expectancy_r,
            require_walk_forward_pass: r.gate.evidence.require_walk_forward_pass,
            block_ban_list: r.events.block_ban_list,
            ban_list_max_age_hours: r.events.ban_list_max_age_hours,
            block_results_days: r.events.block_results_days,
            block_policy_days: r.events.block_policy_days,
            expiry_day_no_new_after_ist: r.events.expiry_day_no_new_after_ist,
            events_file: r.events.file.clone(),
            allow_naked_short: r.options.allow_naked_short,
            allow_stop_widening: beh.allow_stop_widening,
            allow_averaging_down: beh.allow_averaging_down,
            cooloff_until: beh.cooloff_until,
            on_daily_loss: lim.on_daily_loss,
            paper_starting_cash_inr: r.execution.paper_starting_cash_inr,
        }
    }

    /// The rule context for one gate run.
    #[must_use]
    pub(crate) fn rule_ctx(&self, baseline: &Baseline, now: DateTime<Tz>) -> RuleCtx {
        RuleCtx {
            limits: self.effective(baseline),
            mode: self.mode(),
            config_hash: self.hash.clone(),
            now,
        }
    }
}

fn lower_dec(file: Decimal, base: Option<Decimal>) -> Decimal {
    base.map_or(file, |b| file.min(b))
}

fn higher_dec(file: Decimal, base: Option<Decimal>) -> Decimal {
    base.map_or(file, |b| file.max(b))
}

fn lower_u32(file: u32, base: Option<u32>) -> u32 {
    base.map_or(file, |b| file.min(b))
}

fn higher_u32(file: u32, base: Option<u32>) -> u32 {
    base.map_or(file, |b| file.max(b))
}

/// Baseline limits remembered by the `LimitsBaseline` store (slice 6; persisted by F10): a value
/// here can only tighten the file. `None` = no baseline for that key. No `Default`: use `none()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Baseline {
    pub max_risk_per_trade_pct: Option<Decimal>,
    pub max_daily_loss_pct: Option<Decimal>,
    pub max_total_open_risk_pct: Option<Decimal>,
    pub max_weekly_loss_pct: Option<Decimal>,
    pub max_position_value_pct: Option<Decimal>,
    pub max_notional_per_order_inr: Option<Decimal>,
    pub min_charges_slippage_buffer: Option<Decimal>,
    pub cap_equity_at_starting_cash: Option<bool>,
    pub max_open_positions: Option<u32>,
    pub max_orders_per_minute: Option<u32>,
    pub max_orders_per_day: Option<u32>,
    pub cooloff_after_losses: Option<u32>,
    pub max_new_entries_per_day: Option<u32>,
    pub entry_zone_tolerance_bps: Option<u32>,
    pub reentry_lockout_minutes: Option<u32>,
    pub max_sector_pct: Option<Decimal>,
    pub override_max_per_day: Option<u32>,
    pub override_max_per_week: Option<u32>,
}

impl Baseline {
    /// A baseline that constrains nothing.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            max_risk_per_trade_pct: None,
            max_daily_loss_pct: None,
            max_total_open_risk_pct: None,
            max_weekly_loss_pct: None,
            max_position_value_pct: None,
            max_notional_per_order_inr: None,
            min_charges_slippage_buffer: None,
            cap_equity_at_starting_cash: None,
            max_open_positions: None,
            max_orders_per_minute: None,
            max_orders_per_day: None,
            cooloff_after_losses: None,
            max_new_entries_per_day: None,
            entry_zone_tolerance_bps: None,
            reentry_lockout_minutes: None,
            max_sector_pct: None,
            override_max_per_day: None,
            override_max_per_week: None,
        }
    }
}

#[allow(clippy::struct_excessive_bools)] // mirrors the flag keys of risk.toml one for one
/// The limits rules may read: file value combined with the baseline, stricter wins.
///
/// Known limit: the freshness, override-size, confirm-delay, correlation, evidence and event keys
/// have no baseline entry, so they can be loosened within their `[hard_limits]` ceilings or sane
/// bounds mid-session until F10 persists a baseline for them. The ceilings equal the defaults today.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveLimits {
    pub max_risk_per_trade_pct: Decimal,
    pub max_daily_loss_pct: Decimal,
    pub max_total_open_risk_pct: Decimal,
    pub max_weekly_loss_pct: Decimal,
    pub max_position_value_pct: Decimal,
    pub max_notional_per_order_inr: Decimal,
    pub min_charges_slippage_buffer: Decimal,
    pub cap_equity_at_starting_cash: bool,
    pub max_open_positions: u32,
    pub max_orders_per_minute: u32,
    pub max_orders_per_day: u32,
    pub cooloff_after_losses: u32,
    pub max_new_entries_per_day: u32,
    pub entry_zone_tolerance_bps: u32,
    pub reentry_lockout_minutes: u32,
    pub max_sector_pct: Decimal,
    pub override_max_per_day: u32,
    pub override_max_per_week: u32,
    pub max_quote_age_secs: u32,
    pub max_snapshot_age_secs: u32,
    pub override_size_factor: Decimal,
    pub override_confirm_delay_secs: u32,
    // Pass-through keys (no baseline entry): rules read them here, never from the raw file.
    pub correlation_threshold: Decimal,
    pub max_correlated_positions: u32,
    pub correlation_window_days: u32,
    pub min_oos_trades: u32,
    pub min_profit_factor: Decimal,
    pub min_expectancy_r: Decimal,
    pub require_walk_forward_pass: bool,
    pub block_ban_list: bool,
    pub ban_list_max_age_hours: u32,
    pub block_results_days: bool,
    pub block_policy_days: bool,
    pub expiry_day_no_new_after_ist: NaiveTime,
    pub events_file: String,
    pub allow_naked_short: bool,
    pub allow_stop_widening: bool,
    pub allow_averaging_down: bool,
    pub cooloff_until: CooloffUntil,
    pub on_daily_loss: OnDailyLoss,
    pub paper_starting_cash_inr: Decimal,
}

/// What every rule receives: effective limits, mode, config hash and the injected time. It holds
/// no raw config, so a rule cannot read a looser file value than the baseline allows.
#[derive(Debug, Clone)]
pub struct RuleCtx {
    pub limits: EffectiveLimits,
    pub mode: ExecutionMode,
    pub config_hash: ConfigHash,
    pub now: DateTime<Tz>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use chrono_tz::Asia::Kolkata;

    const TEXT: &str = include_str!("../../../fixtures/config/risk.toml");

    fn cfg() -> GateConfig {
        GateConfig::from_toml_str(TEXT).unwrap()
    }

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    #[test]
    fn hash_is_sha256_hex_of_the_text() {
        // sha256("abc")
        assert_eq!(
            ConfigHash::of_text("abc").as_str(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(cfg().hash(), &ConfigHash::of_text(TEXT));
        assert_ne!(ConfigHash::of_text("a"), ConfigHash::of_text("a "));
        assert_eq!(ConfigHash::of_text("abc").to_string().len(), 64);
    }

    #[test]
    fn same_text_same_config_and_hash() {
        assert_eq!(cfg().hash(), cfg().hash());
        assert_eq!(
            cfg().effective(&Baseline::none()),
            cfg().effective(&Baseline::none())
        );
    }

    #[test]
    fn invalid_text_is_a_load_error() {
        assert!(GateConfig::from_toml_str("").is_err());
        let loosened = TEXT.replacen(
            "allow_stop_widening = false",
            "allow_stop_widening = true",
            1,
        );
        assert_ne!(loosened, TEXT);
        assert!(GateConfig::from_toml_str(&loosened).is_err());
    }

    #[test]
    fn no_baseline_gives_file_values() {
        let c = cfg();
        let e = c.effective(&Baseline::none());
        assert_eq!(e.max_risk_per_trade_pct, d("0.5"));
        assert_eq!(e.max_daily_loss_pct, d("2"));
        assert_eq!(c.mode(), bazaar_core::mode::ExecutionMode::Paper);
    }

    #[test]
    fn baseline_can_tighten_but_never_loosen() {
        let c = cfg();
        let file = c.effective(&Baseline::none());

        let mut tight = Baseline::none();
        tight.max_daily_loss_pct = Some(d("1"));
        tight.max_open_positions = Some(1);
        tight.reentry_lockout_minutes = Some(file.reentry_lockout_minutes + 30);
        tight.min_charges_slippage_buffer = Some(file.min_charges_slippage_buffer + d("10"));
        tight.cap_equity_at_starting_cash = Some(true);
        let e = c.effective(&tight);
        assert_eq!(e.max_daily_loss_pct, d("1"));
        assert_eq!(e.max_open_positions, 1);
        assert_eq!(e.reentry_lockout_minutes, file.reentry_lockout_minutes + 30);
        assert_eq!(
            e.min_charges_slippage_buffer,
            file.min_charges_slippage_buffer + d("10")
        );
        assert!(e.cap_equity_at_starting_cash);

        let big = d("1000000");
        let loose = Baseline {
            max_risk_per_trade_pct: Some(big),
            max_daily_loss_pct: Some(big),
            max_total_open_risk_pct: Some(big),
            max_weekly_loss_pct: Some(big),
            max_position_value_pct: Some(big),
            max_notional_per_order_inr: Some(big),
            min_charges_slippage_buffer: Some(Decimal::ZERO),
            cap_equity_at_starting_cash: Some(false),
            max_open_positions: Some(u32::MAX),
            max_orders_per_minute: Some(u32::MAX),
            max_orders_per_day: Some(u32::MAX),
            cooloff_after_losses: Some(u32::MAX),
            max_new_entries_per_day: Some(u32::MAX),
            entry_zone_tolerance_bps: Some(u32::MAX),
            reentry_lockout_minutes: Some(0),
            max_sector_pct: Some(big),
            override_max_per_day: Some(u32::MAX),
            override_max_per_week: Some(u32::MAX),
        };
        let e = c.effective(&loose);
        assert_eq!(
            e, file,
            "a looser baseline must not change the effective limits"
        );
    }

    #[test]
    fn every_baseline_key_tightens_the_matching_limit() {
        let c = cfg();
        let file = c.effective(&Baseline::none());
        let mut b = Baseline::none();
        b.max_risk_per_trade_pct = Some(d("0.1"));
        b.max_total_open_risk_pct = Some(d("0.1"));
        b.max_weekly_loss_pct = Some(d("0.1"));
        b.max_position_value_pct = Some(d("0.1"));
        b.max_notional_per_order_inr = Some(d("1"));
        b.max_orders_per_minute = Some(0);
        b.max_orders_per_day = Some(0);
        b.cooloff_after_losses = Some(0);
        b.max_new_entries_per_day = Some(0);
        b.entry_zone_tolerance_bps = Some(0);
        b.max_sector_pct = Some(d("0.1"));
        b.override_max_per_day = Some(0);
        b.override_max_per_week = Some(0);
        let e = c.effective(&b);
        assert_eq!(e.max_risk_per_trade_pct, d("0.1"));
        assert_eq!(e.max_total_open_risk_pct, d("0.1"));
        assert_eq!(e.max_weekly_loss_pct, d("0.1"));
        assert_eq!(e.max_position_value_pct, d("0.1"));
        assert_eq!(e.max_notional_per_order_inr, d("1"));
        assert_eq!(
            (
                e.max_orders_per_minute,
                e.max_orders_per_day,
                e.cooloff_after_losses
            ),
            (0, 0, 0)
        );
        assert_eq!(
            (
                e.max_new_entries_per_day,
                e.entry_zone_tolerance_bps,
                e.override_max_per_day,
                e.override_max_per_week
            ),
            (0, 0, 0, 0)
        );
        assert_eq!(e.max_sector_pct, d("0.1"));
        // Non-tightenable keys pass through from the file.
        assert_eq!(e.max_quote_age_secs, file.max_quote_age_secs);
        assert_eq!(e.override_size_factor, file.override_size_factor);
    }

    #[test]
    fn effective_limits_pass_through_the_other_keys() {
        let e = cfg().effective(&Baseline::none());
        assert!(!e.allow_stop_widening && !e.allow_averaging_down && !e.allow_naked_short);
        assert!(e.block_ban_list && e.block_results_days && e.block_policy_days);
        assert!(e.correlation_threshold > Decimal::ZERO && e.correlation_threshold <= Decimal::ONE);
        assert!(e.max_correlated_positions > 0 && e.correlation_window_days > 0);
        assert!(e.min_oos_trades > 0 && e.min_profit_factor > Decimal::ZERO);
        assert!(e.ban_list_max_age_hours > 0 && !e.events_file.is_empty());
        assert!(e.paper_starting_cash_inr > Decimal::ZERO);
        assert_eq!(e.cooloff_until, CooloffUntil::EndOfSession);
        assert_eq!(e.on_daily_loss, OnDailyLoss::LockoutEntries);
        let _ = (
            e.min_expectancy_r,
            e.require_walk_forward_pass,
            e.expiry_day_no_new_after_ist,
        );
    }

    #[test]
    fn rule_ctx_and_limits_hold_no_risk_config() {
        let source = include_str!("config.rs");
        let production = source.split("#[cfg(test)]").next().unwrap_or_default();
        for name in ["pub struct EffectiveLimits", "pub struct RuleCtx"] {
            let body = production
                .split(name)
                .nth(1)
                .and_then(|r| r.split("\n}").next())
                .unwrap();
            assert!(
                !body.contains("RiskConfig"),
                "{name} must not hold RiskConfig"
            );
            assert!(
                !body.contains("GateConfig"),
                "{name} must not hold GateConfig"
            );
        }
    }

    #[test]
    fn rule_ctx_carries_effective_limits_hash_mode_and_time() {
        let now = Kolkata.with_ymd_and_hms(2026, 11, 19, 10, 0, 0).unwrap();
        let c = cfg();
        let ctx = c.rule_ctx(&Baseline::none(), now);
        assert_eq!(&ctx.config_hash, c.hash());
        assert_eq!(ctx.now, now);
        assert_eq!(ctx.mode, c.mode());
        assert_eq!(ctx.limits, c.effective(&Baseline::none()));
    }
}
