//! `StrategyContext`: everything a strategy may know besides its own bars.
//!
//! Defined here, not on `risk::PortfolioSnapshot`: `strategies` may not depend on `risk`
//! (architecture test). The backtest and the scanner adapt their portfolio into a `PortfolioView`.
//! The option chain is left out: no option strategy exists in F8 and `options` has no chain type yet.

use bazaar_core::dataview::DataView;
use bazaar_core::instrument::Instrument;
use bazaar_regime::RegimeState;

use crate::exit::OpenPosition;

/// Instruments currently held and their open positions.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PortfolioView {
    pub positions: Vec<OpenPosition>,
}

impl PortfolioView {
    /// True when the instrument is already held (a strategy never adds to a position).
    #[must_use]
    pub fn holds(&self, instrument_token: u64) -> bool {
        self.positions
            .iter()
            .any(|p| p.instrument_token == instrument_token)
    }
}

/// The caller's side of the closed-bar guarantee failed.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ContextError {
    #[error("the view has no bars")]
    EmptyView,
    #[error("a regime is as of {0}, after the decision bar closed: look-ahead")]
    RegimeFromTheFuture(String),
}

/// Per-call inputs. Regimes must be computed by the caller from bars up to the decision bar,
/// with index and stock bars aligned by session date.
#[derive(Debug, Clone, Copy)]
pub struct StrategyContext<'a> {
    pub instrument: &'a Instrument,
    pub market_regime: &'a RegimeState,
    pub stock_regime: &'a RegimeState,
    pub portfolio: &'a PortfolioView,
}

impl StrategyContext<'_> {
    /// Closed-bar guard: neither regime may be as of a time after the view's last close. Strategies
    /// call this first and return `None` on `Err`; tests assert it. Its other half is the caller's:
    /// the regimes must be built only from bars the view can see.
    ///
    /// # Errors
    /// `EmptyView`, or `RegimeFromTheFuture` for the first regime past the decision bar.
    pub fn check_closed_bar(&self, view: &DataView<'_>) -> Result<(), ContextError> {
        let last = view.last().ok_or(ContextError::EmptyView)?.close_time();
        for r in [self.market_regime, self.stock_regime] {
            if r.as_of > last {
                return Err(ContextError::RegimeFromTheFuture(r.as_of.to_rfc3339()));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bazaar_core::candle::{Candle, Interval};
    use bazaar_core::instrument::{Exchange, InstrumentType};
    use bazaar_regime::Fact;
    use rust_decimal::Decimal;

    use crate::signal::tests::ist;

    fn inst() -> Instrument {
        Instrument {
            instrument_token: 7,
            exchange_token: 7,
            tradingsymbol: "ABC".into(),
            name: "ABC".into(),
            exchange: Exchange::parse("NSE"),
            instrument_type: InstrumentType::parse("EQ"),
            segment: "NSE".into(),
            expiry: None,
            strike: None,
            tick_size: Decimal::new(5, 2),
            lot_size: 1,
        }
    }

    fn view_ending_day_3() -> Vec<Candle> {
        (1..=3)
            .map(|d| {
                let p = Decimal::from(100);
                Candle::new(ist(d), Interval::Day, [p, p, p, p], 1, true).unwrap()
            })
            .collect()
    }

    #[test]
    fn regime_as_of_the_decision_bar_passes_and_a_later_one_fails() {
        let (i, p) = (inst(), PortfolioView::default());
        let bars = view_ending_day_3();
        let view = DataView::ending_at(&bars, 2).unwrap();
        let mk = |d| RegimeState::unknown(ist(d), Fact::new("t", "v", "m"));
        let (ok, late) = (mk(3), mk(4));
        let ctx = StrategyContext {
            instrument: &i,
            market_regime: &ok,
            stock_regime: &ok,
            portfolio: &p,
        };
        assert_eq!(ctx.check_closed_bar(&view), Ok(()));
        let bad = StrategyContext {
            stock_regime: &late,
            ..ctx
        };
        assert!(matches!(
            bad.check_closed_bar(&view),
            Err(ContextError::RegimeFromTheFuture(_))
        ));
        let bad = StrategyContext {
            market_regime: &late,
            ..ctx
        };
        assert!(bad.check_closed_bar(&view).is_err());
    }

    #[test]
    fn holds_matches_by_token() {
        let p = PortfolioView {
            positions: vec![OpenPosition {
                instrument_token: 7,
                entry_price: Decimal::from(1),
                stop: Decimal::ZERO,
                opened_at: ist(1),
                bars_held: 0,
            }],
        };
        assert!(p.holds(7) && !p.holds(8));
        assert!(!PortfolioView::default().holds(7));
    }
}
