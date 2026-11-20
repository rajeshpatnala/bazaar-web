//! The event-driven loop: one instrument, one strategy, closed bars only.
//!
//! Bar `i` closes, the strategy decides on `DataView::ending_at(bars, i)` (the structural
//! no-look-ahead guard), and the order fills at bar `i + 1` through the fill model. Entries need an
//! [`EntryPolicy`] verdict (the real `DecisionGate` adapter is slice 4c; this crate ships no
//! approve-all policy), a size from `risk::size_with_cost` priced by the real cost model, and
//! regimes from a caller [`RegimeProvider`] that must be as of the closed bar. Every fill is
//! booked through `Ledger::apply`, so every leg carries its charges. Intraday positions square off
//! at the close of the bar they entered on; anything still open at the end of the data is closed at
//! the last close (`TimeExit`). Pyramiding and partial exits are not modelled.

use std::collections::BTreeMap;

use bazaar_core::calendar::MarketCalendar;
use bazaar_core::candle::Candle;
use bazaar_core::dataview::DataView;
use bazaar_core::instrument::Instrument;
use bazaar_core::money::{Price, Qty};
use bazaar_core::segment::Segment;
use bazaar_risk::config::EffectiveLimits;
use bazaar_risk::sizing::{Sized, SizingInput, size_with_cost};
use bazaar_risk::types::{Side, Tri};
use bazaar_strategies::{
    ExitAction, OpenPosition, PortfolioView, Signal, Strategy, StrategyContext,
};
use rust_decimal::Decimal;

use crate::config::BacktestConfig;
use crate::cost::CostModel;
use crate::fill::{Direction, FillParams, SimFill, check_exit, fill_entry, fill_time_exit};
use crate::fill::{slip_amount, trade_date};
use crate::ledger::Ledger;
use crate::types::{
    BacktestError, EquityPoint, Fill, Leg, LegInstrument, LegSide, Product, SlippageClass, Trade,
};

/// A verdict on one sized entry. There is no default implementation outside tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryVerdict {
    Approve,
    Reject(String),
}

/// Decides whether a sized signal may enter (slice 4c adapts the `DecisionGate` to this).
pub trait EntryPolicy {
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
}

/// What a run produced.
#[derive(Debug, Clone, PartialEq)]
pub struct RunOutput {
    pub trades: Vec<Trade>,
    pub equity: Vec<EquityPoint>,
    /// Signals not traded: policy reject, sizing refusal, or an entry zone the open gapped past.
    pub skipped: u32,
}

struct Pending {
    signal: Signal,
    qty: Qty,
}

struct Open {
    entry: Price,
    stop: Decimal,
    target: Option<Decimal>,
    time_stop_bars: u32,
    bars_held: u32,
    opened_at: chrono::DateTime<chrono_tz::Tz>,
}

fn invalid(why: impl Into<String>) -> BacktestError {
    BacktestError::Invalid(why.into())
}

fn ov(what: &'static str) -> BacktestError {
    BacktestError::Overflow(what)
}

/// Close-only copy of a bar: filling a "time exit" on it prices at the close (square-off and
/// end-of-data exits happen at the close, not the open).
fn close_bar(bar: &Candle) -> Result<Candle, BacktestError> {
    let c = bar.close();
    Candle::new(bar.close_time(), bar.interval(), [c; 4], 0, bar.adjusted())
        .map_err(|e| invalid(e.to_string()))
}

fn book(
    ledger: &mut Ledger<'_>,
    id: &str,
    sim: SimFill,
    template: &Leg,
    bar: &Candle,
    stop: Option<Price>,
) -> Result<Option<Trade>, BacktestError> {
    let mut leg = template.clone();
    leg.trade_date = trade_date(bar.close_time());
    ledger.apply(id, sim.into_fill(leg, bar.close_time()), stop)
}

/// Run the loop.
///
/// # Errors
/// `Invalid` for a bad spec (multiplier `< 1`, unsorted or empty-tick data, an invalid signal or
/// `manage` answer), fill-model, cost-model and ledger errors, `Overflow`.
#[allow(clippy::too_many_lines)]
pub fn run(
    spec: &RunSpec<'_>,
    strategy: &mut dyn Strategy,
    regimes: &dyn RegimeProvider,
    policy: &mut dyn EntryPolicy,
) -> Result<RunOutput, BacktestError> {
    if spec.slippage_multiplier < Decimal::ONE {
        return Err(invalid("slippage multiplier must be at least 1"));
    }
    if spec
        .bars
        .windows(2)
        .any(|w| w[0].close_time() >= w[1].close_time())
    {
        return Err(invalid("bars must be strictly increasing in time"));
    }
    let inst = spec.instrument;
    let token = u32::try_from(inst.instrument_token).map_err(|_| invalid("token too large"))?;
    let slip = spec.config.slippage.for_class(SlippageClass::Equity);
    let fp = FillParams {
        tick: inst.tick_size,
        slippage: slip,
        fills: &spec.config.fills,
        slippage_multiplier: spec.slippage_multiplier,
    };
    let sid = strategy.meta().id;
    let mut ledger = Ledger::new(spec.cost, spec.capital);
    let template = |qty: Qty, price: Price, bar: &Candle| Leg {
        instrument_token: token,
        symbol: inst.tradingsymbol.clone(),
        segment: Segment::Equity,
        instrument: LegInstrument::Cash,
        product: spec.product,
        side: LegSide::Buy,
        qty,
        price,
        trade_date: trade_date(bar.close_time()),
    };
    let snap =
        |v: Decimal| Price::snap_to_tick(v, inst.tick_size).map_err(|e| invalid(e.to_string()));
    let (mut pending, mut open, mut exit_next, mut skipped): (
        Option<Pending>,
        Option<Open>,
        bool,
        u32,
    ) = (None, None, false, 0);
    for (i, bar) in spec.bars.iter().enumerate() {
        if let Some(p) = pending.take() {
            match fill_entry(bar, Direction::Long, Some(p.signal.entry_high), &fp)? {
                Some(sim) => {
                    let leg = template(p.qty, sim.price, bar);
                    book(&mut ledger, sid, sim, &leg, bar, Some(snap(p.signal.stop)?))?;
                    open = Some(Open {
                        entry: sim.price,
                        stop: p.signal.stop,
                        target: p.signal.targets.first().copied(),
                        time_stop_bars: p.signal.exit_plan.time_stop_bars,
                        bars_held: 0,
                        opened_at: bar.close_time(),
                    });
                }
                None => skipped += 1,
            }
        } else if exit_next {
            exit_next = false;
            if let Some(o) = open.take() {
                let sim = fill_time_exit(bar, Direction::Long, &fp)?;
                let held = ledger.open_position(token).map(|p| p.entry.leg.clone());
                let leg = held.unwrap_or_else(|| template(Qty(0), o.entry, bar));
                book(&mut ledger, sid, sim, &leg, bar, None)?;
            }
        }
        if let Some(o) = &open {
            // `entry` only validates the stop's side: a trailed stop may sit above the fill.
            let side_ref = snap(o.entry.value().max(o.stop + inst.tick_size))?;
            if let Some(sim) = check_exit(bar, Direction::Long, side_ref, o.stop, o.target, &fp)? {
                let leg = ledger.open_position(token).map(|p| p.entry.leg.clone());
                let leg = leg.ok_or_else(|| invalid("exit without an open position"))?;
                book(&mut ledger, sid, sim, &leg, bar, None)?;
                open = None;
            }
        }
        if open.is_some() && spec.product == Product::Intraday {
            let cb = close_bar(bar)?;
            ledger.square_off_intraday(trade_date(bar.close_time()), |p| {
                let sim = fill_time_exit(&cb, Direction::Long, &fp)?;
                let mut leg = p.entry.leg.clone();
                leg.trade_date = trade_date(cb.close_time());
                Ok::<Fill, BacktestError>(sim.into_fill(leg, cb.close_time()))
            })?;
            open = None;
        }
        let marks = BTreeMap::from([(token, snap(bar.close())?)]);
        ledger.mark(bar.close_time(), &marks)?;
        if let Some(o) = &mut open {
            o.bars_held += 1;
        }
        if i + 1 == spec.bars.len() {
            break;
        }
        let view = DataView::ending_at(spec.bars, i).ok_or_else(|| invalid("view out of range"))?;
        let portfolio = PortfolioView {
            positions: open
                .iter()
                .map(|o| OpenPosition {
                    instrument_token: inst.instrument_token,
                    entry_price: o.entry.value(),
                    stop: o.stop,
                    opened_at: o.opened_at,
                    bars_held: o.bars_held,
                })
                .collect(),
        };
        let Some(ctx) = regimes.context(&view, inst, &portfolio, spec.calendar) else {
            continue;
        };
        if let Some(o) = &mut open {
            let pos = portfolio
                .positions
                .first()
                .ok_or_else(|| invalid("no position"))?;
            let action = strategy
                .manage(&view, &ctx, pos)
                .map_err(|e| invalid(e.to_string()))?;
            action.check(pos).map_err(|e| invalid(e.to_string()))?;
            match action {
                ExitAction::Hold => {}
                ExitAction::Exit(_) => exit_next = true,
                ExitAction::TightenStop(s) => o.stop = s,
            }
            if o.time_stop_bars > 0 && o.bars_held >= o.time_stop_bars {
                exit_next = true;
            }
        } else if let Some(signal) = strategy.on_bar(&view, &ctx) {
            signal.validate().map_err(|e| invalid(e.to_string()))?;
            let equity = spec
                .capital
                .checked_add(ledger.realised_net())
                .ok_or_else(|| ov("equity"))?;
            match size_entry(spec, &signal, equity, &fp, token, bar) {
                Some(sized) if policy.decide(&signal, &sized, equity) == EntryVerdict::Approve => {
                    pending = Some(Pending {
                        signal,
                        qty: Qty(sized.qty),
                    });
                }
                _ => skipped += 1,
            }
        }
    }
    if let (Some(_), Some(last)) = (open.take(), spec.bars.last()) {
        let cb = close_bar(last)?;
        let sim = fill_time_exit(&cb, Direction::Long, &fp)?;
        let leg = ledger.open_position(token).map(|p| p.entry.leg.clone());
        let leg = leg.ok_or_else(|| invalid("exit without an open position"))?;
        book(&mut ledger, sid, sim, &leg, &cb, None)?;
    }
    Ok(RunOutput {
        trades: ledger.trades().to_vec(),
        equity: ledger.equity_curve().to_vec(),
        skipped,
    })
}

/// Size a signal with `risk::size_with_cost`. The cost closure prices entry and stop-exit legs
/// with the real cost model (DP is not in the estimate; the `risk.toml` buffer floor covers it)
/// plus an entry and a stop slippage estimate, never below the configured floor.
fn size_entry(
    spec: &RunSpec<'_>,
    signal: &Signal,
    equity: Decimal,
    fp: &FillParams<'_>,
    token: u32,
    bar: &Candle,
) -> Option<Sized> {
    let inst = spec.instrument;
    let snap = |v: Decimal| Price::snap_to_tick(v, inst.tick_size).ok();
    let (Some(entry), Some(stop)) = (snap(signal.entry_high), snap(signal.stop)) else {
        return None;
    };
    let floor = spec.limits.min_charges_slippage_buffer;
    let cost = |qty: u32| -> Tri<Decimal> {
        let leg = |side, price| Leg {
            instrument_token: token,
            symbol: inst.tradingsymbol.clone(),
            segment: Segment::Equity,
            instrument: LegInstrument::Cash,
            product: spec.product,
            side,
            qty: Qty(qty),
            price,
            trade_date: trade_date(bar.close_time()),
        };
        let one = |l: &Leg, pct, ticks| -> Option<Decimal> {
            let slip =
                slip_amount(l.price.value(), pct, ticks, fp.tick, fp.slippage_multiplier).ok()?;
            let charges = spec.cost.charges_excl_dp(l).ok()?.total()?;
            slip.checked_mul(Decimal::from(qty))?.checked_add(charges)
        };
        let (buy, sell) = (leg(LegSide::Buy, entry), leg(LegSide::Sell, stop));
        let total = one(&buy, fp.slippage.entry_pct, fp.slippage.entry_min_ticks)
            .zip(one(&sell, fp.slippage.stop_pct, fp.slippage.stop_min_ticks))
            .and_then(|(a, b)| a.checked_add(b));
        total.map_or(Tri::Unknown, |t| Tri::Known(t.max(floor)))
    };
    let input = SizingInput {
        equity,
        side: Side::Buy,
        entry: entry.value(),
        stop: stop.value(),
        lot_size: inst.lot_size,
        tick_size: inst.tick_size,
        buffer: Tri::Unknown,
        overridden: false,
    };
    size_with_cost(spec.limits, &input, &cost).ok()
}

#[cfg(test)]
mod tests;
