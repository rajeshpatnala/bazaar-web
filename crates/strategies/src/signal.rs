//! `Signal`: what a strategy emits at a closed bar. A signal is not a recommendation and not advice:
//! the scanner and the `DecisionGate` decide what happens next, and the gate re-derives the risk.

use bazaar_regime::Fact;
use chrono::DateTime;
use chrono_tz::Tz;
use rust_decimal::Decimal;

/// How the idea ends if nothing else does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExitPlan {
    /// Leave after this many bars at the latest.
    pub time_stop_bars: u32,
    /// Plain-English exit rule for the Why panel.
    pub rule_text: String,
}

/// Long-only swing entry on a daily bar. Prices are rounded to the tick by the strategy.
#[derive(Debug, Clone, PartialEq)]
pub struct Signal {
    pub strategy_id: &'static str,
    /// The instrument this signal is for (display symbol comes from the instrument master).
    pub instrument_token: u64,
    /// Strategy version, for provenance.
    pub strategy_version: u32,
    /// Hash of the `[strategy.<id>.params]` table the strategy was built from.
    pub params_hash: String,
    /// `close_time` (IST) of the decision bar. The fill is the next open, never this close.
    pub signal_bar_time: DateTime<Tz>,
    pub entry_low: Decimal,
    pub entry_high: Decimal,
    pub stop: Decimal,
    /// `targets[0]` is a reference for reward:risk, not necessarily a limit order.
    pub targets: Vec<Decimal>,
    /// `entry_high - stop`. Informational: the gate re-derives it and never trusts this.
    pub risk_per_share: Decimal,
    /// `(targets[0] - entry_low) / risk_per_share`, computed, never a placeholder.
    pub reference_rr: Decimal,
    pub exit_plan: ExitPlan,
    pub facts: Vec<Fact>,
    /// The entry zone expires at the close of the next session.
    pub valid_until: DateTime<Tz>,
}

/// A signal that breaks the contract. Strategies drop such a signal (and log at `debug`).
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SignalError {
    #[error("entry zone is inverted (entry_low above entry_high)")]
    EntryZone,
    #[error("stop must be below the entry zone for a long")]
    StopNotBelowEntry,
    #[error("risk_per_share must equal entry_high - stop")]
    RiskMismatch,
    #[error("reference_rr must be positive")]
    NonPositiveRr,
    #[error("valid_until must be after the signal bar")]
    Expired,
    #[error("a signal needs a first target")]
    NoTarget,
    #[error("the first target must be above entry_high")]
    TargetNotAboveEntry,
    #[error("reference_rr does not match (targets[0] - entry_low) / risk_per_share")]
    RrMismatch,
    #[error("time_stop_bars must be positive")]
    NoTimeStop,
    #[error("params_hash must not be empty")]
    NoProvenance,
}

/// Allowed gap between the stated and the recomputed `reference_rr` (rounding to two places).
const RR_TOLERANCE: Decimal = Decimal::from_parts(1, 0, 0, false, 2);

/// Risk per share for a long: worst-case entry (top of the zone) minus the stop.
#[must_use]
pub fn risk_per_share(entry_high: Decimal, stop: Decimal) -> Decimal {
    entry_high - stop
}

impl Signal {
    /// Check the contract invariants.
    ///
    /// # Errors
    /// The first broken invariant.
    pub fn validate(&self) -> Result<(), SignalError> {
        if self.entry_low > self.entry_high {
            return Err(SignalError::EntryZone);
        }
        if self.stop >= self.entry_low {
            return Err(SignalError::StopNotBelowEntry);
        }
        if self.risk_per_share != risk_per_share(self.entry_high, self.stop) {
            return Err(SignalError::RiskMismatch);
        }
        if self.reference_rr <= Decimal::ZERO {
            return Err(SignalError::NonPositiveRr);
        }
        if self.valid_until <= self.signal_bar_time {
            return Err(SignalError::Expired);
        }
        let first = *self.targets.first().ok_or(SignalError::NoTarget)?;
        if first <= self.entry_high {
            return Err(SignalError::TargetNotAboveEntry);
        }
        // `risk_per_share` is positive here: stop < entry_low <= entry_high.
        let rr = (first - self.entry_low) / self.risk_per_share;
        if (rr - self.reference_rr).abs() > RR_TOLERANCE {
            return Err(SignalError::RrMismatch);
        }
        if self.exit_plan.time_stop_bars == 0 {
            return Err(SignalError::NoTimeStop);
        }
        if self.params_hash.is_empty() {
            return Err(SignalError::NoProvenance);
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use chrono::TimeZone;

    type Break = (fn(&mut Signal), SignalError);

    pub(crate) fn ist(d: u32) -> DateTime<Tz> {
        chrono_tz::Asia::Kolkata
            .with_ymd_and_hms(2026, 3, d, 15, 30, 0)
            .unwrap()
    }

    pub(crate) fn good() -> Signal {
        let (entry_low, entry_high, stop) =
            (Decimal::from(100), Decimal::from(102), Decimal::from(94));
        Signal {
            strategy_id: "t",
            instrument_token: 7,
            strategy_version: 1,
            params_hash: "h".into(),
            signal_bar_time: ist(2),
            entry_low,
            entry_high,
            stop,
            targets: vec![Decimal::from(116)],
            risk_per_share: risk_per_share(entry_high, stop),
            reference_rr: Decimal::from(2),
            exit_plan: ExitPlan {
                time_stop_bars: 10,
                rule_text: "x".into(),
            },
            facts: vec![],
            valid_until: ist(3),
        }
    }

    #[test]
    fn good_signal_is_valid_and_risk_is_top_of_zone_minus_stop() {
        let s = good();
        assert_eq!(s.validate(), Ok(()));
        assert_eq!(s.risk_per_share, Decimal::from(8));
    }

    #[test]
    fn each_broken_invariant_is_named() {
        let mut s = good();
        s.entry_low = Decimal::from(103);
        assert_eq!(s.validate(), Err(SignalError::EntryZone));
        let mut s = good();
        s.stop = Decimal::from(100);
        assert_eq!(s.validate(), Err(SignalError::StopNotBelowEntry));
        let mut s = good();
        s.risk_per_share = Decimal::from(1);
        assert_eq!(s.validate(), Err(SignalError::RiskMismatch));
        let mut s = good();
        s.reference_rr = Decimal::ZERO;
        assert_eq!(s.validate(), Err(SignalError::NonPositiveRr));
        let mut s = good();
        s.valid_until = s.signal_bar_time;
        assert_eq!(s.validate(), Err(SignalError::Expired));
        let broken: [Break; 5] = [
            (|s| s.targets.clear(), SignalError::NoTarget),
            (
                |s| s.targets = vec![Decimal::from(102)],
                SignalError::TargetNotAboveEntry,
            ),
            (
                |s| s.reference_rr = Decimal::from(3),
                SignalError::RrMismatch,
            ),
            (|s| s.exit_plan.time_stop_bars = 0, SignalError::NoTimeStop),
            (|s| s.params_hash.clear(), SignalError::NoProvenance),
        ];
        for (f, want) in broken {
            let mut s = good();
            f(&mut s);
            assert_eq!(s.validate(), Err(want));
        }
        let mut s = good();
        s.reference_rr = Decimal::new(201, 2);
        assert_eq!(s.validate(), Ok(()), "within two-place rounding");
    }
}
