//! Entry sizing: `risk::size_with_cost` priced by the real cost model.

use bazaar_core::instrument::Instrument;
use bazaar_core::money::{Price, Qty};
use bazaar_core::segment::Segment;
use bazaar_risk::sizing::{Sized, SizingInput, size_with_cost};
use bazaar_risk::types::{Side, Tri};
use bazaar_strategies::Signal;
use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::RunSpec;
use super::state::{ov, snap};
use crate::fill::{FillParams, slip_amount};
use crate::types::{BacktestError, Leg, LegInstrument, LegSide, Product};

pub(super) fn cash_leg(
    inst: &Instrument,
    token: u32,
    product: Product,
    side: LegSide,
    qty: Qty,
    price: Price,
    trade_date: NaiveDate,
) -> Leg {
    Leg {
        instrument_token: token,
        symbol: inst.tradingsymbol.clone(),
        segment: Segment::Equity,
        instrument: LegInstrument::Cash,
        product,
        side,
        qty,
        price,
        trade_date,
    }
}

/// Size a signal with `risk::size_with_cost`. The cost closure prices entry and stop-exit legs
/// with the real cost model plus an entry and a stop slippage estimate, never below the
/// `risk.toml` floor. DP (first delivery sell of the day, `[dp]`) is not in the estimate: a known
/// understatement of about Rs 15 per trade against the risk budget. Cost-model errors
/// (`NoRateSet`, `Overflow`) propagate; only a real sizing refusal (budget or cap below one lot)
/// is `None`.
pub(super) fn size_entry(
    spec: &RunSpec<'_>,
    fp: &FillParams<'_>,
    signal: &Signal,
    equity: Decimal,
    token: u32,
    date: NaiveDate,
) -> Result<Option<Sized>, BacktestError> {
    let inst = spec.instrument;
    let entry = snap(inst.tick_size, signal.entry_high)?;
    let stop = snap(inst.tick_size, signal.stop)?;
    let floor = spec.limits.min_charges_slippage_buffer;
    let estimate = |qty: u32| -> Result<Decimal, BacktestError> {
        let leg = |side, price| cash_leg(inst, token, spec.product, side, Qty(qty), price, date);
        let one = |l: &Leg, pct, ticks| -> Result<Decimal, BacktestError> {
            let slip = slip_amount(l.price.value(), pct, ticks, fp.tick, fp.slippage_multiplier)?;
            let charges = spec
                .cost
                .charges_excl_dp(l)?
                .total()
                .ok_or_else(|| ov("charges"))?;
            slip.checked_mul(Decimal::from(qty))
                .and_then(|s| s.checked_add(charges))
                .ok_or_else(|| ov("sizing cost"))
        };
        let buy = one(
            &leg(LegSide::Buy, entry),
            fp.slippage.entry_pct,
            fp.slippage.entry_min_ticks,
        )?;
        let sell = one(
            &leg(LegSide::Sell, stop),
            fp.slippage.stop_pct,
            fp.slippage.stop_min_ticks,
        )?;
        let total = buy.checked_add(sell).ok_or_else(|| ov("sizing cost"))?;
        Ok(total.max(floor))
    };
    // Surface a cost-model error once, up front; the closure below then only sees known costs.
    estimate(inst.lot_size)?;
    let cost = |qty: u32| estimate(qty).map_or(Tri::Unknown, Tri::Known);
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
    Ok(size_with_cost(spec.limits, &input, &cost).ok())
}
