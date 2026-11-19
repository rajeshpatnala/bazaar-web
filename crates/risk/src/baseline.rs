//! `LimitsBaseline`: where the remembered limits come from. A read error is a Reject: the caller
//! of [`rule_ctx`] must never fall back to the file value alone when the baseline is unreadable
//! (the baseline can only tighten, so the file value may be looser than the one in force).
//!
//! Persistence and loosening (next session, with typed confirmation) belong to F10; the in-memory
//! store here only tightens, instantly.

use crate::config::{Baseline, GateConfig, RuleCtx};
use chrono::DateTime;
use chrono_tz::Tz;
use rust_decimal::Decimal;
use std::fmt;

/// The baseline could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineError(pub String);

impl fmt::Display for BaselineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the remembered limits could not be read ({}); no order can be checked until they can",
            self.0
        )
    }
}

impl std::error::Error for BaselineError {}

/// Source of the remembered limits.
pub trait LimitsBaseline {
    /// The limits in force before this run; an error means Reject.
    fn read(&self) -> Result<Baseline, BaselineError>;
}

/// Build the rule context from the file and the baseline. `Err` = the gate rejects the intent.
pub fn rule_ctx(
    config: &GateConfig,
    store: &dyn LimitsBaseline,
    now: DateTime<Tz>,
) -> Result<RuleCtx, BaselineError> {
    let baseline = store.read()?;
    Ok(config.rule_ctx(&baseline, now))
}

/// Test double: returns a fixed baseline or a fixed error.
#[derive(Debug, Clone)]
pub struct FakeBaseline(Result<Baseline, BaselineError>);

impl FakeBaseline {
    /// A store that reads `baseline`.
    #[must_use]
    pub const fn ok(baseline: Baseline) -> Self {
        Self(Ok(baseline))
    }

    /// A store whose read fails with `why`.
    #[must_use]
    pub fn failing(why: &str) -> Self {
        Self(Err(BaselineError(why.to_string())))
    }
}

impl LimitsBaseline for FakeBaseline {
    fn read(&self) -> Result<Baseline, BaselineError> {
        self.0.clone()
    }
}

/// In-memory store: values only ever tighten. A restart forgets them until F10 persists them.
#[derive(Debug, Clone)]
pub struct MemoryBaseline(Baseline);

impl MemoryBaseline {
    /// An empty store.
    #[must_use]
    pub const fn new() -> Self {
        Self(Baseline::none())
    }

    /// Merge `candidate`: for each key the stricter of the stored and the candidate value is kept.
    pub fn tighten(&mut self, candidate: &Baseline) {
        let (a, b) = (&mut self.0, candidate);
        a.max_risk_per_trade_pct = lower(a.max_risk_per_trade_pct, b.max_risk_per_trade_pct);
        a.max_daily_loss_pct = lower(a.max_daily_loss_pct, b.max_daily_loss_pct);
        a.max_total_open_risk_pct = lower(a.max_total_open_risk_pct, b.max_total_open_risk_pct);
        a.max_weekly_loss_pct = lower(a.max_weekly_loss_pct, b.max_weekly_loss_pct);
        a.max_position_value_pct = lower(a.max_position_value_pct, b.max_position_value_pct);
        a.max_notional_per_order_inr =
            lower(a.max_notional_per_order_inr, b.max_notional_per_order_inr);
        a.min_charges_slippage_buffer =
            higher(a.min_charges_slippage_buffer, b.min_charges_slippage_buffer);
        a.cap_equity_at_starting_cash =
            match (a.cap_equity_at_starting_cash, b.cap_equity_at_starting_cash) {
                (Some(x), Some(y)) => Some(x || y),
                (x, y) => x.or(y),
            };
        a.max_open_positions = lower(a.max_open_positions, b.max_open_positions);
        a.max_orders_per_minute = lower(a.max_orders_per_minute, b.max_orders_per_minute);
        a.max_orders_per_day = lower(a.max_orders_per_day, b.max_orders_per_day);
        a.cooloff_after_losses = lower(a.cooloff_after_losses, b.cooloff_after_losses);
        a.max_new_entries_per_day = lower(a.max_new_entries_per_day, b.max_new_entries_per_day);
        a.entry_zone_tolerance_bps = lower(a.entry_zone_tolerance_bps, b.entry_zone_tolerance_bps);
        a.reentry_lockout_minutes = higher(a.reentry_lockout_minutes, b.reentry_lockout_minutes);
        a.max_sector_pct = lower(a.max_sector_pct, b.max_sector_pct);
        a.override_max_per_day = lower(a.override_max_per_day, b.override_max_per_day);
        a.override_max_per_week = lower(a.override_max_per_week, b.override_max_per_week);
    }
}

impl Default for MemoryBaseline {
    fn default() -> Self {
        Self::new()
    }
}

impl LimitsBaseline for MemoryBaseline {
    fn read(&self) -> Result<Baseline, BaselineError> {
        Ok(self.0.clone())
    }
}

trait Ord2: Copy + PartialOrd {}
impl Ord2 for Decimal {}
impl Ord2 for u32 {}

fn pick<T: Ord2>(a: Option<T>, b: Option<T>, keep_lower: bool) -> Option<T> {
    match (a, b) {
        (Some(x), Some(y)) => Some(if (x <= y) == keep_lower { x } else { y }),
        (x, y) => x.or(y),
    }
}

fn lower<T: Ord2>(a: Option<T>, b: Option<T>) -> Option<T> {
    pick(a, b, true)
}

fn higher<T: Ord2>(a: Option<T>, b: Option<T>) -> Option<T> {
    pick(a, b, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use chrono_tz::Asia::Kolkata;

    const TEXT: &str = include_str!("../../../fixtures/config/risk.toml");

    fn now() -> DateTime<Tz> {
        Kolkata.with_ymd_and_hms(2026, 1, 5, 10, 0, 0).unwrap()
    }

    #[test]
    fn read_error_is_an_error_not_the_file_value() {
        let cfg = GateConfig::from_toml_str(TEXT).unwrap();
        let err = rule_ctx(&cfg, &FakeBaseline::failing("disk"), now()).unwrap_err();
        assert!(err.to_string().contains("could not be read"));
        assert!(err.to_string().contains("disk"));
    }

    #[test]
    fn stricter_baseline_wins_through_the_store() {
        let cfg = GateConfig::from_toml_str(TEXT).unwrap();
        let mut b = Baseline::none();
        b.max_new_entries_per_day = Some(1);
        b.reentry_lockout_minutes = Some(90);
        let ctx = rule_ctx(&cfg, &FakeBaseline::ok(b), now()).unwrap();
        assert_eq!(ctx.limits.max_new_entries_per_day, 1);
        assert_eq!(ctx.limits.reentry_lockout_minutes, 90);
    }

    #[test]
    fn memory_store_only_tightens() {
        let mut m = MemoryBaseline::new();
        let mut b = Baseline::none();
        b.max_new_entries_per_day = Some(2);
        b.reentry_lockout_minutes = Some(60);
        b.max_daily_loss_pct = Some(Decimal::from(2));
        b.cap_equity_at_starting_cash = Some(false);
        m.tighten(&b);
        let mut looser = Baseline::none();
        looser.max_new_entries_per_day = Some(3);
        looser.reentry_lockout_minutes = Some(30);
        looser.max_daily_loss_pct = Some(Decimal::from(3));
        looser.cap_equity_at_starting_cash = Some(true);
        m.tighten(&looser);
        let got = m.read().unwrap();
        assert_eq!(got.max_new_entries_per_day, Some(2));
        assert_eq!(got.reentry_lockout_minutes, Some(60));
        assert_eq!(got.max_daily_loss_pct, Some(Decimal::from(2)));
        assert_eq!(got.cap_equity_at_starting_cash, Some(true));
        let mut tighter = Baseline::none();
        tighter.max_new_entries_per_day = Some(1);
        tighter.reentry_lockout_minutes = Some(120);
        m.tighten(&tighter);
        let got = m.read().unwrap();
        assert_eq!(got.max_new_entries_per_day, Some(1));
        assert_eq!(got.reentry_lockout_minutes, Some(120));
    }

    #[test]
    fn empty_store_changes_nothing() {
        let cfg = GateConfig::from_toml_str(TEXT).unwrap();
        let ctx = rule_ctx(&cfg, &MemoryBaseline::new(), now()).unwrap();
        assert_eq!(ctx.limits, cfg.effective(&Baseline::none()));
    }
}
