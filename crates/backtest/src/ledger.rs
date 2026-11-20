//! Ledger: open positions, round trips, intraday square-off, equity curve and drawdown.
//!
//! Every fill is priced here through `CostModel::charges_with_dp` (the engine entry), so the
//! depository charge lands once per scrip per IST day on the first delivery sell. One `Ledger`
//! owns one `DpLedger`; never share a ledger (or its DP state) across parallel runs. All rupee
//! amounts are `Decimal` with checked arithmetic; dates come from the fill's `trade_date` (IST),
//! never a wall clock.
//!
//! Scope: one open position per instrument token, closed in full by an opposite-side fill (no
//! pyramiding, no partial exits). Intraday may be long or short; delivery is long only (a
//! delivery sell with nothing to close is `Invalid`). A stop must sit on the loss side of the
//! entry price. An exit may not precede its entry (trade date or bar time).
//! `gross_pnl` is after slippage (the fill price already includes it) and before charges;
//! `net_pnl = gross_pnl - charges`, and equity, R and drawdown all use net figures.
//!
//! Conventions (conservative, documented in the F9 record): an open position is valued at its
//! last mark (carried forward over a missing bar) at gross unrealised minus its entry charges;
//! exit charges are NOT reserved, so a flat or closed ledger is exact but an open one is a
//! little optimistic. A trade opened without a stop has `r_multiple = None`.

use std::collections::BTreeMap;

use bazaar_core::money::Price;
use chrono::{DateTime, NaiveDate};
use chrono_tz::Tz;
use rust_decimal::{Decimal, RoundingStrategy};

use crate::cost::{CostModel, DpLedger};
use crate::types::{BacktestError, ChargeBreakdown, EquityPoint, Fill, LegSide, Product, Trade};

/// An open position: its entry fill (charges already priced), the stop that defines the risk,
/// the strategy that owns it, and its last mark (the entry price until marked).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Position {
    pub strategy_id: String,
    pub entry: Fill,
    pub stop: Option<Price>,
    pub last_mark: Price,
}

/// Worst peak-to-trough fall of an equity curve: the largest rupee fall and the largest
/// fraction of the peak (6 dp, half-up); they may occur at different points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Drawdown {
    pub max_abs: Decimal,
    pub max_pct: Decimal,
}

/// Positions, closed trades and equity for one run.
#[derive(Debug)]
pub struct Ledger<'a> {
    cost: CostModel<'a>,
    dp: DpLedger,
    capital: Decimal,
    realised: Decimal,
    positions: BTreeMap<u32, Position>,
    trades: Vec<Trade>,
    equity: Vec<EquityPoint>,
}

fn ov(what: &'static str) -> BacktestError {
    BacktestError::Overflow(what)
}

fn invalid(symbol: &str, why: &str) -> BacktestError {
    BacktestError::Invalid(format!("{symbol}: {why}"))
}

fn signed_move(side: LegSide, entry: Decimal, exit: Decimal) -> Result<Decimal, BacktestError> {
    match side {
        LegSide::Buy => exit.checked_sub(entry),
        LegSide::Sell => entry.checked_sub(exit),
    }
    .ok_or_else(|| ov("price move"))
}

impl<'a> Ledger<'a> {
    /// A fresh ledger with its own DP state and starting capital.
    #[must_use]
    pub fn new(cost: CostModel<'a>, capital: Decimal) -> Self {
        Self {
            cost,
            dp: DpLedger::new(),
            capital,
            realised: Decimal::ZERO,
            positions: BTreeMap::new(),
            trades: Vec::new(),
            equity: Vec::new(),
        }
    }

    #[must_use]
    pub fn trades(&self) -> &[Trade] {
        &self.trades
    }

    #[must_use]
    pub fn equity_curve(&self) -> &[EquityPoint] {
        &self.equity
    }

    #[must_use]
    pub fn open_position(&self, token: u32) -> Option<&Position> {
        self.positions.get(&token)
    }

    /// Cumulative net P&L of closed trades.
    #[must_use]
    pub const fn realised_net(&self) -> Decimal {
        self.realised
    }

    /// Book a fill. Prices its charges via `charges_with_dp`, then opens a position (no position
    /// open for the token) or closes it in full (opposite side, same quantity and product).
    /// `stop` only matters on an opening fill: it sets the R denominator and must be below a long
    /// entry price or above a short one.
    ///
    /// # Errors
    /// `Invalid` for a same-side fill on an open position, a partial close, a product mismatch,
    /// an exit before its entry, an opening delivery sell or a wrong-side stop; cost-model errors
    /// (`NoRateSet`, ...); `Overflow`. The ledger (DP state included) is unchanged on error.
    pub fn apply(
        &mut self,
        strategy_id: &str,
        mut fill: Fill,
        stop: Option<Price>,
    ) -> Result<Option<Trade>, BacktestError> {
        let token = fill.leg.instrument_token;
        let sym = fill.leg.symbol.clone();
        match self.positions.get(&token) {
            Some(open) => {
                if open.entry.leg.side == fill.leg.side {
                    return Err(invalid(
                        &sym,
                        "same-side fill on an open position (no pyramiding)",
                    ));
                }
                if open.entry.leg.qty != fill.leg.qty || open.entry.leg.product != fill.leg.product
                {
                    return Err(invalid(
                        &sym,
                        "exit must close the full quantity in the same product",
                    ));
                }
                if fill.leg.trade_date < open.entry.leg.trade_date
                    || fill.bar_time < open.entry.bar_time
                {
                    return Err(invalid(&sym, "exit is dated before its entry"));
                }
            }
            None => check_opening(&fill, stop)?,
        }
        let date = fill.leg.trade_date;
        fill.charges = self.cost.charges_with_dp(&mut self.dp, &fill.leg)?;
        let booked_dp = !fill.charges.dp.is_zero();
        let Some(open) = self.positions.get(&token) else {
            let last_mark = fill.leg.price;
            self.positions.insert(
                token,
                Position {
                    strategy_id: strategy_id.to_owned(),
                    entry: fill,
                    stop,
                    last_mark,
                },
            );
            return Ok(None);
        };
        let closed = build_trade(open, fill).and_then(|t| {
            let realised = self
                .realised
                .checked_add(t.net_pnl)
                .ok_or_else(|| ov("realised"))?;
            Ok((t, realised))
        });
        match closed {
            Ok((trade, realised)) => {
                self.realised = realised;
                self.positions.remove(&token);
                self.trades.push(trade.clone());
                Ok(Some(trade))
            }
            Err(e) => {
                if booked_dp {
                    self.dp.release(token, date);
                }
                Err(e)
            }
        }
    }

    /// Close every open intraday position entered on or before `date`, in token order, with the
    /// exit fill `exit_for` builds (the engine prices it at the square-off bar). Stops at the
    /// first error: positions closed before it stay closed, the rest stay open.
    ///
    /// # Errors
    /// The first error from `exit_for` or `apply`.
    pub fn square_off_intraday(
        &mut self,
        date: NaiveDate,
        mut exit_for: impl FnMut(&Position) -> Result<Fill, BacktestError>,
    ) -> Result<Vec<Trade>, BacktestError> {
        let due: Vec<u32> = self
            .positions
            .iter()
            .filter(|(_, p)| {
                p.entry.leg.product == Product::Intraday && p.entry.leg.trade_date <= date
            })
            .map(|(t, _)| *t)
            .collect();
        let mut out = Vec::new();
        for token in due {
            let Some(p) = self.positions.get(&token) else {
                continue;
            };
            let exit = exit_for(p)?;
            let id = p.strategy_id.clone();
            if let Some(t) = self.apply(&id, exit, None)? {
                out.push(t);
            }
        }
        Ok(out)
    }

    /// Record equity after a bar closed: capital + realised net - entry charges of open
    /// positions + unrealised gross at the latest mark. `marks` updates the last mark per token;
    /// a token with no mark keeps its previous one (carried forward, so a data gap cannot hide
    /// an earlier adverse move). Exit charges of open positions are not reserved.
    ///
    /// # Errors
    /// `Invalid` if `time` is earlier than the last recorded point; `Overflow`. The ledger is
    /// unchanged on error.
    pub fn mark(
        &mut self,
        time: DateTime<Tz>,
        marks: &BTreeMap<u32, Price>,
    ) -> Result<Decimal, BacktestError> {
        if self.equity.last().is_some_and(|p| time < p.time) {
            return Err(BacktestError::Invalid(
                "equity point earlier than the previous one".into(),
            ));
        }
        let mut equity = self
            .capital
            .checked_add(self.realised)
            .ok_or_else(|| ov("equity"))?;
        for (token, p) in &self.positions {
            let last = marks.get(token).copied().unwrap_or(p.last_mark);
            let unrealised =
                signed_move(p.entry.leg.side, p.entry.leg.price.value(), last.value())?
                    .checked_mul(Decimal::from(p.entry.leg.qty.0))
                    .ok_or_else(|| ov("unrealised"))?;
            let paid = p.entry.charges.total().ok_or_else(|| ov("charges"))?;
            equity = equity
                .checked_add(unrealised)
                .and_then(|e| e.checked_sub(paid))
                .ok_or_else(|| ov("equity"))?;
        }
        for (token, p) in &mut self.positions {
            if let Some(m) = marks.get(token) {
                p.last_mark = *m;
            }
        }
        self.equity.push(EquityPoint { time, equity });
        Ok(equity)
    }

    /// Drawdown of the recorded equity curve, measured from the starting capital (a run that
    /// loses from its first bar is not shown a smaller fall).
    ///
    /// # Errors
    /// `Overflow`.
    pub fn drawdown(&self) -> Result<Drawdown, BacktestError> {
        max_drawdown(self.capital, &self.equity)
    }
}

/// Checks on a fill that opens a position.
fn check_opening(fill: &Fill, stop: Option<Price>) -> Result<(), BacktestError> {
    let leg = &fill.leg;
    if leg.product == Product::Delivery && leg.side == LegSide::Sell {
        return Err(invalid(
            &leg.symbol,
            "delivery sell with no position to close",
        ));
    }
    if let Some(s) = stop {
        let ok = match leg.side {
            LegSide::Buy => s < leg.price,
            LegSide::Sell => s > leg.price,
        };
        if !ok {
            return Err(invalid(
                &leg.symbol,
                "stop must be on the loss side of the entry price",
            ));
        }
    }
    Ok(())
}

fn build_trade(open: &Position, exit: Fill) -> Result<Trade, BacktestError> {
    let entry = open.entry.clone();
    let qty = Decimal::from(entry.leg.qty.0);
    let gross = signed_move(
        entry.leg.side,
        entry.leg.price.value(),
        exit.leg.price.value(),
    )?
    .checked_mul(qty)
    .ok_or_else(|| ov("gross pnl"))?;
    let charges: ChargeBreakdown = entry
        .charges
        .checked_add(&exit.charges)
        .ok_or_else(|| ov("charges"))?;
    let net = gross
        .checked_sub(charges.total().ok_or_else(|| ov("charges"))?)
        .ok_or_else(|| ov("net pnl"))?;
    let risk = match open.stop {
        Some(s) => entry
            .leg
            .price
            .value()
            .checked_sub(s.value())
            .and_then(|d| d.abs().checked_mul(qty))
            .ok_or_else(|| ov("risk"))?,
        None => Decimal::ZERO,
    };
    let r_multiple = net
        .checked_div(risk)
        .map(|r| r.round_dp_with_strategy(4, RoundingStrategy::MidpointAwayFromZero));
    Ok(Trade {
        strategy_id: open.strategy_id.clone(),
        entry,
        exit,
        gross_pnl: gross,
        charges,
        net_pnl: net,
        risk_inr: risk,
        r_multiple,
    })
}

/// Largest peak-to-trough fall of `curve`, with the peak seeded at `starting_equity` (a curve
/// that never rises above it still shows its fall). An empty curve gives zero. The percent is
/// skipped while the peak is not positive.
///
/// # Errors
/// `Overflow`.
pub fn max_drawdown(
    starting_equity: Decimal,
    curve: &[EquityPoint],
) -> Result<Drawdown, BacktestError> {
    let mut peak = starting_equity;
    let mut out = Drawdown {
        max_abs: Decimal::ZERO,
        max_pct: Decimal::ZERO,
    };
    for p in curve {
        peak = peak.max(p.equity);
        let fall = peak.checked_sub(p.equity).ok_or_else(|| ov("drawdown"))?;
        out.max_abs = out.max_abs.max(fall);
        if peak > Decimal::ZERO {
            let pct = fall
                .checked_div(peak)
                .ok_or_else(|| ov("drawdown"))?
                .round_dp_with_strategy(6, RoundingStrategy::MidpointAwayFromZero);
            out.max_pct = out.max_pct.max(pct);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
