//! The loop's state machine: pending entry, open position, deferred exit, equity marks.

use std::collections::BTreeMap;

use bazaar_core::candle::Candle;
use bazaar_core::dataview::DataView;
use bazaar_core::money::{Price, Qty};
use bazaar_strategies::{ExitAction, OpenPosition, PortfolioView, Signal, Strategy};
use rust_decimal::Decimal;

use super::sizing::{cash_leg, size_entry};
use super::{EntryPolicy, EntryVerdict, RegimeProvider, RunSpec};
use crate::fill::trade_date;
use crate::fill::{Direction, FillParams, SimFill, check_exit, fill_entry, fill_time_exit};
use crate::ledger::Ledger;
use crate::types::{BacktestError, Fill, Leg, LegSide, Product, Trade};

pub(super) struct Pending {
    signal: Signal,
    qty: Qty,
}

pub(super) struct Open {
    entry: Price,
    stop: Decimal,
    target: Option<Decimal>,
    time_stop_bars: u32,
    bars_held: u32,
    opened_at: chrono::DateTime<chrono_tz::Tz>,
}

pub(crate) fn invalid(why: impl Into<String>) -> BacktestError {
    BacktestError::Invalid(why.into())
}

pub(crate) fn ov(what: &'static str) -> BacktestError {
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

pub(crate) fn snap(tick: Decimal, v: Decimal) -> Result<Price, BacktestError> {
    Price::snap_to_tick(v, tick).map_err(|e| invalid(e.to_string()))
}

/// The loop's state; one per run.
pub(super) struct Engine<'a, 'b> {
    pub(super) spec: &'b RunSpec<'a>,
    pub(super) fp: FillParams<'b>,
    pub(super) ledger: Ledger<'a>,
    pub(super) token: u32,
    pub(super) sid: &'static str,
    pub(super) pending: Option<Pending>,
    pub(super) open: Option<Open>,
    pub(super) exit_next: bool,
    pub(super) skipped: u32,
}

impl Engine<'_, '_> {
    fn tick(&self) -> Decimal {
        self.spec.instrument.tick_size
    }

    fn held_leg(&self) -> Result<Leg, BacktestError> {
        self.ledger
            .open_position(self.token)
            .map(|p| p.entry.leg.clone())
            .ok_or_else(|| invalid("exit without an open position"))
    }

    /// Close the open position with `sim` on `bar`.
    fn close(&mut self, sim: SimFill, bar: &Candle) -> Result<(), BacktestError> {
        let leg = self.held_leg()?;
        book(&mut self.ledger, self.sid, sim, &leg, bar, None)?;
        self.open = None;
        Ok(())
    }

    /// Fill the pending entry (if any) at `bar`'s open: dropped when stale or gapped past the zone.
    pub(super) fn enter(&mut self, bar: &Candle) -> Result<(), BacktestError> {
        let Some(p) = self.pending.take() else {
            return Ok(());
        };
        if bar.close_time() > p.signal.valid_until {
            self.skipped += 1;
            return Ok(());
        }
        let Some(sim) = fill_entry(bar, Direction::Long, Some(p.signal.entry_high), &self.fp)?
        else {
            self.skipped += 1;
            return Ok(());
        };
        let leg = cash_leg(
            self.spec.instrument,
            self.token,
            self.spec.product,
            LegSide::Buy,
            p.qty,
            sim.price,
            trade_date(bar.close_time()),
        );
        let stop = snap(self.tick(), p.signal.stop)?;
        book(&mut self.ledger, self.sid, sim, &leg, bar, Some(stop))?;
        self.open = Some(Open {
            entry: sim.price,
            stop: p.signal.stop,
            target: p.signal.targets.first().copied(),
            time_stop_bars: p.signal.exit_plan.time_stop_bars,
            bars_held: 0,
            opened_at: bar.close_time(),
        });
        Ok(())
    }

    /// A strategy or time-stop exit decided on the previous close fills at this open.
    pub(super) fn exit_at_open(&mut self, bar: &Candle) -> Result<(), BacktestError> {
        if std::mem::take(&mut self.exit_next) && self.open.is_some() {
            let sim = fill_time_exit(bar, Direction::Long, &self.fp)?;
            self.close(sim, bar)?;
        }
        Ok(())
    }

    pub(super) fn check_stops(&mut self, bar: &Candle) -> Result<(), BacktestError> {
        let Some(o) = &self.open else { return Ok(()) };
        // `entry` only validates the stop's side: a trailed stop may sit above the fill.
        let above = o.stop.checked_add(self.tick()).ok_or_else(|| ov("stop"))?;
        let side_ref = snap(self.tick(), o.entry.value().max(above))?;
        if let Some(sim) = check_exit(bar, Direction::Long, side_ref, o.stop, o.target, &self.fp)? {
            self.close(sim, bar)?;
        }
        Ok(())
    }

    /// Intraday (daily-bar approximation): out at the close of the entry bar.
    pub(super) fn square_off(&mut self, bar: &Candle) -> Result<(), BacktestError> {
        if self.open.is_none() || self.spec.product != Product::Intraday {
            return Ok(());
        }
        let cb = close_bar(bar)?;
        let fp = &self.fp;
        self.ledger
            .square_off_intraday(trade_date(bar.close_time()), |p| {
                let sim = fill_time_exit(&cb, Direction::Long, fp)?;
                let mut leg = p.entry.leg.clone();
                leg.trade_date = trade_date(cb.close_time());
                Ok::<Fill, BacktestError>(sim.into_fill(leg, cb.close_time()))
            })?;
        self.open = None;
        Ok(())
    }

    /// Close any open position at the last close, then record the bar's equity (so the final
    /// point includes the exit's slippage and charges).
    pub(super) fn end_of_bar(&mut self, bar: &Candle, last: bool) -> Result<(), BacktestError> {
        if last && self.open.is_some() {
            let cb = close_bar(bar)?;
            let sim = fill_time_exit(&cb, Direction::Long, &self.fp)?;
            self.close(sim, &cb)?;
        }
        let marks = BTreeMap::from([(self.token, snap(self.tick(), bar.close())?)]);
        self.ledger.mark(bar.close_time(), &marks)?;
        if let Some(o) = &mut self.open {
            o.bars_held += 1;
        }
        Ok(())
    }

    /// The decision at the close of bar `i`: manage the position or look for an entry.
    pub(super) fn decide(
        &mut self,
        i: usize,
        strategy: &mut dyn Strategy,
        regimes: &dyn RegimeProvider,
        policy: &mut dyn EntryPolicy,
    ) -> Result<(), BacktestError> {
        let spec = self.spec;
        policy.observe(self.ledger.trades());
        let view = DataView::ending_at(spec.bars, i).ok_or_else(|| invalid("view out of range"))?;
        let portfolio = PortfolioView {
            positions: self
                .open
                .iter()
                .map(|o| OpenPosition {
                    instrument_token: spec.instrument.instrument_token,
                    entry_price: o.entry.value(),
                    stop: o.stop,
                    opened_at: o.opened_at,
                    bars_held: o.bars_held,
                })
                .collect(),
        };
        let ctx = regimes.context(&view, spec.instrument, &portfolio, spec.calendar);
        if let Some(c) = &ctx {
            // Structural guard: a regime from the future or another session is a hard error.
            c.check_closed_bar(&view)
                .map_err(|e| invalid(e.to_string()))?;
        }
        if let Some(o) = &mut self.open {
            if let (Some(c), Some(pos)) = (&ctx, portfolio.positions.first()) {
                let action = strategy
                    .manage(&view, c, pos)
                    .map_err(|e| invalid(e.to_string()))?;
                action.check(pos).map_err(|e| invalid(e.to_string()))?;
                match action {
                    ExitAction::Hold => {}
                    ExitAction::Exit(_) => self.exit_next = true,
                    ExitAction::TightenStop(s) => {
                        o.stop = snap(spec.instrument.tick_size, s)?.value();
                    }
                }
            }
            // The time stop does not need a regime.
            if o.time_stop_bars > 0 && o.bars_held >= o.time_stop_bars {
                self.exit_next = true;
            }
        } else if let Some(signal) = ctx.as_ref().and_then(|c| strategy.on_bar(&view, c)) {
            signal.validate().map_err(|e| invalid(e.to_string()))?;
            let equity = spec
                .capital
                .checked_add(self.ledger.realised_net())
                .ok_or_else(|| ov("equity"))?;
            // Charges are priced on the fill day (the next bar), not the decision day.
            let fill_day = spec
                .bars
                .get(i + 1)
                .or_else(|| view.last())
                .map(|b| trade_date(b.close_time()))
                .ok_or_else(|| invalid("no bar"))?;
            match size_entry(spec, &self.fp, &signal, equity, self.token, fill_day)? {
                Some(sized) if policy.decide(&signal, &sized, equity) == EntryVerdict::Approve => {
                    self.pending = Some(Pending {
                        signal,
                        qty: Qty(sized.qty),
                    });
                }
                _ => self.skipped += 1,
            }
        }
        Ok(())
    }
}
