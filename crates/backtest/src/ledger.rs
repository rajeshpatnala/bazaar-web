//! Ledger: open positions, round trips, intraday square-off, equity curve and drawdown.
//!
//! Every fill is priced here through `CostModel::charges_with_dp` (the engine entry), so the
//! depository charge lands once per scrip per IST day on the first delivery sell. One `Ledger`
//! owns one `DpLedger`; never share a ledger (or its DP state) across parallel runs. All rupee
//! amounts are `Decimal`; dates come from the fill's `trade_date` (IST), never a wall clock.
//!
//! Scope: one open position per instrument token, closed in full by an opposite-side fill (no
//! pyramiding, no partial exits). Long and short are both supported (`entry.leg.side`).
//! `gross_pnl` is after slippage (the fill price already includes it) and before charges;
//! `net_pnl = gross_pnl - charges`, and equity, R and drawdown all use net figures.

use std::collections::BTreeMap;

use bazaar_core::money::Price;
use chrono::{DateTime, NaiveDate};
use chrono_tz::Tz;
use rust_decimal::{Decimal, RoundingStrategy};

use crate::cost::{CostModel, DpLedger};
use crate::types::{BacktestError, ChargeBreakdown, EquityPoint, Fill, LegSide, Product, Trade};

/// An open position: its entry fill (charges already priced), the stop that defines the risk,
/// and the strategy that owns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Position {
    pub strategy_id: String,
    pub entry: Fill,
    pub stop: Option<Price>,
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

fn signed_move(side: LegSide, entry: Decimal, exit: Decimal) -> Decimal {
    match side {
        LegSide::Buy => exit - entry,
        LegSide::Sell => entry - exit,
    }
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
    /// `stop` only matters on an opening fill: it sets the R denominator.
    ///
    /// # Errors
    /// `Invalid` for a same-side fill on an open position, a partial close or a product mismatch;
    /// cost-model errors (`NoRateSet`, ...); `Overflow`. The ledger is unchanged on error.
    pub fn apply(
        &mut self,
        strategy_id: &str,
        mut fill: Fill,
        stop: Option<Price>,
    ) -> Result<Option<Trade>, BacktestError> {
        let token = fill.leg.instrument_token;
        if let Some(open) = self.positions.get(&token) {
            if open.entry.leg.side == fill.leg.side {
                return Err(BacktestError::Invalid(format!(
                    "{}: same-side fill on an open position (no pyramiding)",
                    fill.leg.symbol
                )));
            }
            if open.entry.leg.qty != fill.leg.qty || open.entry.leg.product != fill.leg.product {
                return Err(BacktestError::Invalid(format!(
                    "{}: exit must close the full quantity in the same product",
                    fill.leg.symbol
                )));
            }
        }
        // Price on a scratch copy of the DP state so an error leaves the ledger untouched.
        let mut dp = self.dp.clone();
        fill.charges = self.cost.charges_with_dp(&mut dp, &fill.leg)?;
        let Some(open) = self.positions.get(&token) else {
            self.dp = dp;
            self.positions.insert(
                token,
                Position {
                    strategy_id: strategy_id.to_owned(),
                    entry: fill,
                    stop,
                },
            );
            return Ok(None);
        };
        let trade = build_trade(open, fill)?;
        let realised = self
            .realised
            .checked_add(trade.net_pnl)
            .ok_or_else(|| ov("realised"))?;
        self.dp = dp;
        self.realised = realised;
        self.positions.remove(&token);
        self.trades.push(trade.clone());
        Ok(Some(trade))
    }

    /// Close every open intraday position entered on or before `date`, in token order, with the
    /// exit fill `exit_for` builds (the engine prices it at the square-off bar).
    ///
    /// # Errors
    /// The first error from `exit_for` or `apply`; positions closed before it stay closed.
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
    /// positions + unrealised gross at `marks` (a position with no mark is valued at its entry).
    ///
    /// # Errors
    /// `Overflow`.
    pub fn mark(
        &mut self,
        time: DateTime<Tz>,
        marks: &BTreeMap<u32, Price>,
    ) -> Result<Decimal, BacktestError> {
        let mut equity = self
            .capital
            .checked_add(self.realised)
            .ok_or_else(|| ov("equity"))?;
        for (token, p) in &self.positions {
            let entry = p.entry.leg.price.value();
            let last = marks.get(token).map_or(entry, |m| m.value());
            let unrealised = signed_move(p.entry.leg.side, entry, last)
                .checked_mul(Decimal::from(p.entry.leg.qty.0))
                .ok_or_else(|| ov("unrealised"))?;
            let paid = p.entry.charges.total().ok_or_else(|| ov("charges"))?;
            equity = equity
                .checked_add(unrealised)
                .and_then(|e| e.checked_sub(paid))
                .ok_or_else(|| ov("equity"))?;
        }
        self.equity.push(EquityPoint { time, equity });
        Ok(equity)
    }

    /// Drawdown of the recorded equity curve.
    ///
    /// # Errors
    /// `Overflow`.
    pub fn drawdown(&self) -> Result<Drawdown, BacktestError> {
        max_drawdown(&self.equity)
    }
}

fn build_trade(open: &Position, exit: Fill) -> Result<Trade, BacktestError> {
    let entry = open.entry.clone();
    let qty = Decimal::from(entry.leg.qty.0);
    let gross = signed_move(
        entry.leg.side,
        entry.leg.price.value(),
        exit.leg.price.value(),
    )
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
        Some(s) => (entry.leg.price.value() - s.value())
            .abs()
            .checked_mul(qty)
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

/// Largest peak-to-trough fall of `curve` (empty or rising curve: zero).
///
/// # Errors
/// `Overflow`.
pub fn max_drawdown(curve: &[EquityPoint]) -> Result<Drawdown, BacktestError> {
    let mut peak: Option<Decimal> = None;
    let mut out = Drawdown {
        max_abs: Decimal::ZERO,
        max_pct: Decimal::ZERO,
    };
    for p in curve {
        let pk = peak.map_or(p.equity, |k| k.max(p.equity));
        peak = Some(pk);
        let fall = pk.checked_sub(p.equity).ok_or_else(|| ov("drawdown"))?;
        out.max_abs = out.max_abs.max(fall);
        if pk > Decimal::ZERO {
            let pct = fall
                .checked_div(pk)
                .ok_or_else(|| ov("drawdown"))?
                .round_dp_with_strategy(6, RoundingStrategy::MidpointAwayFromZero);
            out.max_pct = out.max_pct.max(pct);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
