//! F9 charges for the paper account and for the gate (BZ-521 S0).
//!
//! Two adapters over the F9 `CostModel` (`bazaar-backtest`), both NSE equity cash only and both
//! reading rates ONLY from a parsed `charges.toml` (no rate, percentage or minimum lives here):
//!
//! - [`F9Charges`] implements the paper account's `ChargeSource`: exact charges for one fill
//!   (`mis` = intraday, `cnc` = delivery), trade date from the `Clock`, `HistoryPolicy::Strict`.
//!   The depository charge is levied on EVERY CNC sell fill (a fresh `DpLedger` per call): when
//!   several sells of one scrip land on a day this over-states cost, never under-states it.
//! - [`GateCost`] implements the gate's `CostModel`, which has no product: it returns the worse
//!   of the MIS and CNC charges plus the configured equity paper slippage, so it never
//!   understates and may block a trade by a few rupees.
//!
//! Anything missing or odd (rate set, segment row, GST or DP table, zero quantity, non-positive
//! price, overflow, unsupported product, odd symbol, a zero total for a non-zero notional) is
//! `Unavailable` / `Tri::Unknown`; never zero and never a default. Known limit: the symbol is
//! only checked for being a plain token; the paper universe (S2) restricts it to cash equities.

use std::sync::Arc;

use bazaar_backtest::cost::{CostModel as F9CostModel, DpLedger};
use bazaar_backtest::types::{HistoryPolicy, Leg, LegInstrument, LegSide, Product};
use bazaar_core::clock::Clock;
use bazaar_core::config::{ChargeExchange, ChargesConfig};
use bazaar_core::money::{Price, Qty};
use bazaar_core::ports::SourceError;
use bazaar_core::segment::Segment;
use bazaar_paper::account::ChargeSource;
use bazaar_risk::gate::CostModel;
use bazaar_risk::types::{Side, Tri};
use rust_decimal::Decimal;
use thiserror::Error;

/// Longest symbol accepted (guards against junk reaching the leg).
const MAX_SYMBOL_CHARS: usize = 32;
/// Basis points per unit.
const BPS_DIVISOR: Decimal = Decimal::from_parts(10_000, 0, 0, false, 0);

/// Why an adapter could not be built (construction time, never per call).
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CostsError {
    /// `charges.toml` is a placeholder: it carries no rates.
    #[error("charges.toml is a placeholder")]
    Placeholder,
}

fn symbol_ok(s: &str) -> bool {
    !s.is_empty()
        && s.chars().count() <= MAX_SYMBOL_CHARS
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '&' || c == '-' || c == '_')
}

/// Total charge for one NSE equity cash leg, or `None` on any problem.
fn total_charge(
    cfg: &ChargesConfig,
    trade_date: chrono::NaiveDate,
    symbol: &str,
    side: LegSide,
    qty: u32,
    price: Decimal,
    product: Product,
) -> Option<Decimal> {
    if qty == 0 || price <= Decimal::ZERO || !symbol_ok(symbol) {
        return None;
    }
    // Lossless "tick": one unit in the last decimal place of the price, so `Price` keeps the
    // exact value (turnover is not rounded); the market tick is the instrument master's job.
    let tick = Decimal::new(1, price.scale());
    let price = Price::snap_to_tick(price, tick).ok()?;
    let leg = Leg {
        // The DP ledger is fresh per call, so the token is never used for de-duplication.
        instrument_token: 0,
        symbol: symbol.to_owned(),
        segment: Segment::Equity,
        instrument: LegInstrument::Cash,
        product,
        side,
        qty: Qty(qty),
        price,
        trade_date,
    };
    let model = F9CostModel::new(cfg, HistoryPolicy::Strict, ChargeExchange::Nse);
    let total = model
        .charges_with_dp(&mut DpLedger::new(), &leg)
        .ok()?
        .total()?;
    // A zero charge on a non-zero notional means a missing rate, not a free trade.
    (total > Decimal::ZERO).then_some(total)
}

/// Exact F9 charges for a paper fill (`ChargeSource`).
#[derive(Clone)]
pub struct F9Charges {
    cfg: Arc<ChargesConfig>,
    clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for F9Charges {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("F9Charges").finish_non_exhaustive()
    }
}

impl F9Charges {
    /// # Errors
    /// `Placeholder` when the file carries no rates.
    pub fn new(cfg: Arc<ChargesConfig>, clock: Arc<dyn Clock>) -> Result<Self, CostsError> {
        if cfg.placeholder {
            return Err(CostsError::Placeholder);
        }
        Ok(Self { cfg, clock })
    }
}

impl ChargeSource for F9Charges {
    fn charge(
        &self,
        symbol: &str,
        is_buy: bool,
        qty: u32,
        price: Decimal,
        product: &str,
    ) -> Result<Decimal, SourceError> {
        let product = match product {
            "mis" => Product::Intraday,
            "cnc" => Product::Delivery,
            _ => return Err(SourceError::Unavailable),
        };
        let side = if is_buy { LegSide::Buy } else { LegSide::Sell };
        let date = self.clock.now_ist().date_naive();
        total_charge(&self.cfg, date, symbol, side, qty, price, product)
            .ok_or(SourceError::Unavailable)
    }
}

/// Gate cost: worst of MIS and CNC equity charges plus equity paper slippage.
#[derive(Clone)]
pub struct GateCost {
    cfg: Arc<ChargesConfig>,
    clock: Arc<dyn Clock>,
    slippage_bps: u32,
}

impl std::fmt::Debug for GateCost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GateCost")
            .field("slippage_bps", &self.slippage_bps)
            .finish_non_exhaustive()
    }
}

impl GateCost {
    /// `slippage_bps` is `execution.paper_slippage_bps.equity` from `risk.toml`.
    ///
    /// # Errors
    /// `Placeholder` when the file carries no rates.
    pub fn new(
        cfg: Arc<ChargesConfig>,
        clock: Arc<dyn Clock>,
        slippage_bps: u32,
    ) -> Result<Self, CostsError> {
        if cfg.placeholder {
            return Err(CostsError::Placeholder);
        }
        Ok(Self {
            cfg,
            clock,
            slippage_bps,
        })
    }

    fn worst(&self, side: Side, qty: u32, price: Decimal) -> Option<Decimal> {
        let date = self.clock.now_ist().date_naive();
        let side = match side {
            Side::Buy => LegSide::Buy,
            Side::Sell => LegSide::Sell,
        };
        // The gate has no symbol: any valid placeholder token; rates do not depend on it.
        let one = |p| total_charge(&self.cfg, date, "GATE", side, qty, price, p);
        let charges = one(Product::Intraday)?.max(one(Product::Delivery)?);
        let slip = price
            .checked_mul(Decimal::from(qty))?
            .checked_mul(Decimal::from(self.slippage_bps))?
            .checked_div(BPS_DIVISOR)?;
        charges.checked_add(slip)
    }
}

impl CostModel for GateCost {
    fn cost(&self, side: Side, qty: u32, price: Decimal) -> Tri<Decimal> {
        self.worst(side, qty, price)
            .map_or(Tri::Unknown, Tri::Known)
    }
}

#[cfg(test)]
mod tests;
