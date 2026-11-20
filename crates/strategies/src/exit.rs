//! The exit hook: what `Strategy::manage` may tell the engine about an open long position.

use chrono::DateTime;
use chrono_tz::Tz;
use rust_decimal::Decimal;

/// An open long position, as the engine reports it. Strategies never mutate it.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenPosition {
    pub instrument_token: u64,
    pub entry_price: Decimal,
    /// The stop currently in force.
    pub stop: Decimal,
    pub opened_at: DateTime<Tz>,
    /// Closed bars since entry.
    pub bars_held: u32,
}

/// What to do with an open position at this closed bar. An exit fills at the next open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitAction {
    Hold,
    /// Leave; the text is the plain-English reason.
    Exit(String),
    /// Raise the stop. Build it with [`ExitAction::tighten`], which refuses a lower stop.
    TightenStop(Decimal),
}

/// A `manage` answer that breaks the contract.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ExitError {
    #[error("a long stop may only move up: {new} is not above {current}")]
    StopNotRaised { current: Decimal, new: Decimal },
    /// The view or instrument could not be used (unadjusted candles, bad tick, empty view). The
    /// engine must treat it as a data fault, never as "hold".
    #[error("cannot manage the position: {0}")]
    Context(String),
}

impl ExitAction {
    /// `TightenStop(new_stop)` for a long: `new_stop` must be strictly above the current stop.
    ///
    /// # Errors
    /// `StopNotRaised` when `new_stop <= position.stop`.
    pub fn tighten(position: &OpenPosition, new_stop: Decimal) -> Result<Self, ExitError> {
        if new_stop <= position.stop {
            return Err(ExitError::StopNotRaised {
                current: position.stop,
                new: new_stop,
            });
        }
        Ok(Self::TightenStop(new_stop))
    }

    /// Check an action a strategy returned against the position (the engine calls this).
    ///
    /// # Errors
    /// `StopNotRaised` for a `TightenStop` that is not above the current stop.
    pub fn check(&self, position: &OpenPosition) -> Result<(), ExitError> {
        match self {
            Self::TightenStop(s) => Self::tighten(position, *s).map(|_| ()),
            Self::Hold | Self::Exit(_) => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::tests::ist;

    fn pos() -> OpenPosition {
        OpenPosition {
            instrument_token: 1,
            entry_price: Decimal::from(100),
            stop: Decimal::from(94),
            opened_at: ist(2),
            bars_held: 3,
        }
    }

    #[test]
    fn stop_may_only_move_up() {
        assert_eq!(
            ExitAction::tighten(&pos(), Decimal::from(95)),
            Ok(ExitAction::TightenStop(Decimal::from(95)))
        );
        for bad in [94, 90] {
            assert!(ExitAction::tighten(&pos(), Decimal::from(bad)).is_err());
            assert!(
                ExitAction::TightenStop(Decimal::from(bad))
                    .check(&pos())
                    .is_err()
            );
        }
    }

    #[test]
    fn hold_and_exit_always_pass_the_check() {
        assert_eq!(ExitAction::Hold.check(&pos()), Ok(()));
        assert_eq!(
            ExitAction::Exit("closed below the 10-day low".into()).check(&pos()),
            Ok(())
        );
    }
}
