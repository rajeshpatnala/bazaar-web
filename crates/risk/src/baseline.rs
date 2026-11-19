//! `LimitsBaseline`: where the remembered limits come from. A read error is a Reject: the caller
//! of [`rule_ctx`] must never fall back to the file value alone when the baseline is unreadable
//! (the baseline can only tighten, so the file value may be looser than the one in force).
//!
//! Persistence and loosening (next session, with typed confirmation) belong to F10; the in-memory
//! store here only tightens, instantly.

use crate::config::{Baseline, GateConfig, RuleCtx};
use chrono::DateTime;
use chrono_tz::Tz;
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

/// Test double: returns a fixed baseline or a fixed error. Test builds only.
#[cfg(test)]
#[derive(Debug, Clone)]
pub struct FakeBaseline(Result<Baseline, BaselineError>);

#[cfg(test)]
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

#[cfg(test)]
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

/// Keep the lower of two optional values (`None` = no value).
fn lower<T: Ord + Copy>(a: Option<T>, b: Option<T>) -> Option<T> {
    a.map_or(b, |x| Some(crate::config::lower(x, b)))
}

/// Keep the higher of two optional values (`None` = no value).
fn higher<T: Ord + Copy>(a: Option<T>, b: Option<T>) -> Option<T> {
    a.map_or(b, |x| Some(crate::config::higher(x, b)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use chrono_tz::Asia::Kolkata;
    use rust_decimal::Decimal;

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

    /// Every baseline key: a looser candidate never loosens, a stricter one wins, and a missing
    /// stored value takes the candidate. `lo`/`hi` are two distinct values of the key's type.
    macro_rules! tighten_table {
        ($field:ident, $lo:expr, $hi:expr, $stricter:expr) => {{
            let (lo, hi) = ($lo, $hi);
            let (strict, loose) = if $stricter == lo { (lo, hi) } else { (hi, lo) };
            let with = |v| {
                let mut b = Baseline::none();
                b.$field = Some(v);
                b
            };
            let mut m = MemoryBaseline::new();
            m.tighten(&with(loose));
            assert_eq!(m.read().unwrap().$field, Some(loose), stringify!($field));
            m.tighten(&with(strict));
            assert_eq!(m.read().unwrap().$field, Some(strict), stringify!($field));
            m.tighten(&with(loose));
            assert_eq!(m.read().unwrap().$field, Some(strict), stringify!($field));
            m.tighten(&Baseline::none());
            assert_eq!(m.read().unwrap().$field, Some(strict), stringify!($field));
        }};
    }

    #[test]
    fn every_baseline_key_tightens_in_its_own_direction() {
        let (d1, d2) = (Decimal::from(1), Decimal::from(2));
        // Max-type keys keep the lower value.
        tighten_table!(max_risk_per_trade_pct, d1, d2, d1);
        tighten_table!(max_daily_loss_pct, d1, d2, d1);
        tighten_table!(max_total_open_risk_pct, d1, d2, d1);
        tighten_table!(max_weekly_loss_pct, d1, d2, d1);
        tighten_table!(max_position_value_pct, d1, d2, d1);
        tighten_table!(max_notional_per_order_inr, d1, d2, d1);
        tighten_table!(max_sector_pct, d1, d2, d1);
        tighten_table!(max_open_positions, 1u32, 2u32, 1u32);
        tighten_table!(max_orders_per_minute, 1u32, 2u32, 1u32);
        tighten_table!(max_orders_per_day, 1u32, 2u32, 1u32);
        tighten_table!(cooloff_after_losses, 1u32, 2u32, 1u32);
        tighten_table!(max_new_entries_per_day, 1u32, 2u32, 1u32);
        tighten_table!(entry_zone_tolerance_bps, 1u32, 2u32, 1u32);
        tighten_table!(override_max_per_day, 1u32, 2u32, 1u32);
        tighten_table!(override_max_per_week, 1u32, 2u32, 1u32);
        // Floor-type keys keep the higher value.
        tighten_table!(min_charges_slippage_buffer, d1, d2, d2);
        tighten_table!(reentry_lockout_minutes, 1u32, 2u32, 2u32);
        // The cap flag: true (capped) is stricter than false.
        tighten_table!(cap_equity_at_starting_cash, false, true, true);
    }

    #[test]
    fn empty_store_changes_nothing() {
        let cfg = GateConfig::from_toml_str(TEXT).unwrap();
        let ctx = rule_ctx(&cfg, &MemoryBaseline::new(), now()).unwrap();
        assert_eq!(ctx.limits, cfg.effective(&Baseline::none()));
    }
}
