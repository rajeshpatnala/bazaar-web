//! The event-driven loop: one instrument, one strategy, closed bars only.
//!
//! Bar `i` closes, the strategy decides on `DataView::ending_at(bars, i)` (the structural
//! no-look-ahead guard), and the order fills at bar `i + 1` through the fill model. Entries need an
//! [`EntryPolicy`] verdict (the real `DecisionGate` adapter is slice 4c; this crate ships no
//! approve-all policy), a size from `risk::size_with_cost` priced by the real cost model, and
//! regimes from a caller [`RegimeProvider`] that must be as of the closed bar. Every fill is
//! booked through `Ledger::apply`, so every leg carries its charges. Intraday positions square off
//! at the close of the bar they entered on (daily bars only; other intervals are refused); anything
//! still open at the end of the data is closed at the last close (`TimeExit`). Pyramiding and partial exits are not modelled.

use bazaar_core::calendar::MarketCalendar;
use bazaar_core::candle::{Candle, Interval};
use bazaar_core::dataview::DataView;
use bazaar_core::instrument::{Exchange, Instrument, InstrumentType};
use bazaar_risk::config::EffectiveLimits;
use bazaar_risk::sizing::Sized;
use bazaar_strategies::{PortfolioView, Signal, Strategy, StrategyContext};
use rust_decimal::Decimal;

use crate::config::BacktestConfig;
use crate::cost::CostModel;
use crate::fill::FillParams;
use crate::ledger::Ledger;
use crate::types::{BacktestError, EquityPoint, Product, SlippageClass, Trade};

mod sizing;
mod state;
mod stress;

pub(crate) use sizing::round_trip_cost;
use state::Engine;
pub(crate) use state::{invalid, snap};
pub use stress::{RunParts, StressedRun, run_with_stress};

/// A verdict on one sized entry. There is no default implementation outside tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryVerdict {
    Approve,
    Reject(String),
}

/// Decides whether a sized signal may enter (slice 4c adapts the `DecisionGate` to this).
pub trait EntryPolicy {
    /// Called at every decision bar before any entry is judged, with every trade closed so far
    /// (oldest first). A stateful policy (the gate adapter) derives its day and week figures from
    /// it; the default ignores it.
    fn observe(&mut self, _closed: &[Trade]) {}

    fn decide(&mut self, signal: &Signal, sized: &Sized, equity: Decimal) -> EntryVerdict;
}

/// Builds the strategy context for a decision bar. The implementation owns the regimes and must
/// return them as of `view.last()` only (the strategy's `check_closed_bar` rejects anything else);
/// `None` skips the bar.
pub trait RegimeProvider {
    fn context<'a>(
        &'a self,
        view: &DataView<'_>,
        instrument: &'a Instrument,
        portfolio: &'a PortfolioView,
        calendar: &'a MarketCalendar,
    ) -> Option<StrategyContext<'a>>;
}

/// Everything one run needs besides the strategy, regimes and policy.
#[derive(Debug)]
pub struct RunSpec<'a> {
    pub bars: &'a [Candle],
    pub instrument: &'a Instrument,
    pub calendar: &'a MarketCalendar,
    pub config: &'a BacktestConfig,
    pub cost: CostModel<'a>,
    pub limits: &'a EffectiveLimits,
    pub product: Product,
    pub capital: Decimal,
    /// `>= 1`: 1 normally, `fills.stress_slippage_multiplier` for the stress run (QUAN-84-8).
    pub slippage_multiplier: Decimal,
    /// The first `warmup_bars` bars are history only (indicator warm-up): no equity marks, and no
    /// entry fills in them (the first decision is at the last history bar, filling at the first
    /// window bar). 0 for a plain run; at most `bars.len()`.
    pub warmup_bars: usize,
}

/// What a run produced.
#[derive(Debug, Clone, PartialEq)]
pub struct RunOutput {
    pub trades: Vec<Trade>,
    pub equity: Vec<EquityPoint>,
    /// Signals not traded: policy reject, sizing refusal, or an entry zone the open gapped past.
    pub skipped: u32,
}

fn validate(spec: &RunSpec<'_>) -> Result<(), BacktestError> {
    if spec.slippage_multiplier < Decimal::ONE {
        return Err(invalid("slippage multiplier must be at least 1"));
    }
    if spec.warmup_bars > spec.bars.len() {
        return Err(invalid("warm-up is longer than the bars"));
    }
    let inst = spec.instrument;
    let cash_equity = inst.instrument_type == InstrumentType::Equity
        && matches!(inst.exchange, Exchange::Nse | Exchange::Bse);
    if !cash_equity {
        return Err(invalid("only NSE/BSE cash equity is modelled"));
    }
    if spec
        .bars
        .windows(2)
        .any(|w| w[0].close_time() >= w[1].close_time())
    {
        return Err(invalid("bars must be strictly increasing in time"));
    }
    if spec.product == Product::Intraday && spec.bars.iter().any(|b| b.interval() != Interval::Day)
    {
        return Err(invalid(
            "intraday runs need daily bars (square-off at the bar close is a daily-bar approximation)",
        ));
    }
    Ok(())
}

/// The fill parameters of a run; the gate adapter prices its cost estimate with the same ones.
pub(crate) fn fill_params<'a>(spec: &'a RunSpec<'_>) -> FillParams<'a> {
    FillParams {
        tick: spec.instrument.tick_size,
        slippage: spec.config.slippage.for_class(SlippageClass::Equity),
        fills: &spec.config.fills,
        slippage_multiplier: spec.slippage_multiplier,
    }
}

/// Run the loop. Single instrument: the whole `capital` is available to it, so pooling several
/// runs overstates capacity and must not be reported as a portfolio result. Intraday needs daily
/// bars (square-off at the bar close is a daily-bar approximation).
///
/// # Errors
/// `Invalid` for a bad spec (multiplier `< 1`, unsorted bars, a non-equity instrument, intraday
/// on non-daily bars) or an invalid signal, regime or `manage` answer; fill-model, cost-model
/// (`NoRateSet` included: never a silent skip) and ledger errors; `Overflow`.
pub fn run(
    spec: &RunSpec<'_>,
    strategy: &mut dyn Strategy,
    regimes: &dyn RegimeProvider,
    policy: &mut dyn EntryPolicy,
) -> Result<RunOutput, BacktestError> {
    validate(spec)?;
    let token =
        u32::try_from(spec.instrument.instrument_token).map_err(|_| invalid("token too large"))?;
    let mut e = Engine {
        spec,
        fp: fill_params(spec),
        ledger: Ledger::new(spec.cost, spec.capital),
        token,
        sid: strategy.meta().id,
        pending: None,
        open: None,
        exit_next: false,
        skipped: 0,
        tag: None,
        tagged: 0,
        warmup: spec.warmup_bars,
    };
    for (i, bar) in spec.bars.iter().enumerate() {
        let last = i + 1 == spec.bars.len();
        if i >= e.warmup {
            if e.pending.is_some() {
                e.enter(bar)?;
            } else {
                e.exit_at_open(bar)?;
            }
            e.check_stops(bar)?;
            e.square_off(bar)?;
            e.end_of_bar(bar, last)?;
        }
        if !last && i + 1 >= e.warmup {
            e.decide(i, strategy, regimes, policy)?;
        }
    }
    Ok(RunOutput {
        trades: e.ledger.trades().to_vec(),
        equity: e.ledger.equity_curve().to_vec(),
        skipped: e.skipped,
    })
}

#[cfg(test)]
mod tests;
