//! Cost model: Indian charges per leg, driven only by the typed `config/charges.toml`.
//!
//! Rounding (verified against the owner's calculator samples): STT and stamp to whole rupees,
//! brokerage, exchange transaction, SEBI and GST to 2 dp, all half-up and only after the full
//! multiplication. GST = `gst.rate_pct` x (brokerage + exchange txn + SEBI); STT, stamp, IPFT
//! and DP are outside the base. IPFT (an exchange levy) is folded into the `exchange_txn` line.

use std::collections::HashSet;

use bazaar_core::config::{
    Brokerage, ChargeExchange, ChargeSegment, ChargeSide, ChargesConfig, DpCharge, RateSet,
};
use bazaar_core::segment::Segment;
use chrono::NaiveDate;
use rust_decimal::{Decimal, RoundingStrategy};

use crate::types::{
    BacktestError, ChargeBreakdown, HistoryPolicy, Leg, LegInstrument, LegSide, Product,
};

/// Rupees per crore, the SEBI fee base.
const CRORE: Decimal = Decimal::from_parts(10_000_000, 0, 0, false, 0);

/// Tracks which (scrip, IST day) already paid the depository charge.
#[derive(Debug, Default, Clone)]
pub struct DpLedger {
    paid: HashSet<(u32, NaiveDate)>,
}

impl DpLedger {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Computes per-leg charges from a borrowed `ChargesConfig`.
#[derive(Debug, Clone, Copy)]
pub struct CostModel<'a> {
    cfg: &'a ChargesConfig,
    policy: HistoryPolicy,
    exchange: ChargeExchange,
}

impl<'a> CostModel<'a> {
    /// A model on NSE (the default venue of the backtest).
    #[must_use]
    pub const fn new(cfg: &'a ChargesConfig, policy: HistoryPolicy) -> Self {
        Self {
            cfg,
            policy,
            exchange: ChargeExchange::Nse,
        }
    }

    /// Same model on another exchange (`Leg` carries no venue).
    #[must_use]
    pub const fn with_exchange(mut self, exchange: ChargeExchange) -> Self {
        self.exchange = exchange;
        self
    }

    /// True when `latest_for_all` is in use: provenance must say `costs_approximate = true`.
    #[must_use]
    pub fn costs_approximate(&self) -> bool {
        self.policy == HistoryPolicy::LatestForAll
    }

    /// Charges on one leg, DP excluded (stateless; see [`Self::charges_with_dp`]).
    ///
    /// # Errors
    /// `NoRateSet` (strict policy, no set in force on the trade date), `Invalid` for an
    /// impossible segment/product/instrument mix, `Overflow` on arithmetic overflow.
    pub fn charges(&self, leg: &Leg) -> Result<ChargeBreakdown, BacktestError> {
        let set = self.rate_set(leg)?;
        let gst_rate = self.gst_rate()?;
        let turnover = leg.turnover().ok_or(BacktestError::Overflow("turnover"))?;
        let pct = |rate: Decimal| mul_div(turnover, rate, Decimal::ONE_HUNDRED);
        let brokerage = match &set.brokerage {
            Brokerage::Zero {} => Decimal::ZERO,
            Brokerage::Flat { flat } => *flat,
            Brokerage::Pct { pct: p } => pct(*p)?,
            Brokerage::FlatOrPctLower { flat, pct: p } => pct(*p)?.min(*flat),
        };
        let brokerage = round(brokerage, 2);
        let stt = round(pct(set.stt_ctt_pct)?, 0);
        let txn = round(pct(set.exchange_txn_pct)?, 2);
        let ipft = round(pct(set.ipft_pct)?, 2);
        let sebi = round(mul_div(turnover, set.sebi_per_crore, CRORE)?, 2);
        let stamp = round(pct(set.stamp_pct)?, 0);
        let base = brokerage
            .checked_add(txn)
            .and_then(|v| v.checked_add(sebi))
            .ok_or(BacktestError::Overflow("gst base"))?;
        Ok(ChargeBreakdown {
            brokerage,
            stt,
            exchange_txn: txn
                .checked_add(ipft)
                .ok_or(BacktestError::Overflow("exchange txn"))?,
            gst: round(mul_div(base, gst_rate, Decimal::ONE_HUNDRED)?, 2),
            sebi,
            stamp,
            dp: Decimal::ZERO,
        })
    }

    /// As [`Self::charges`], plus the depository charge on the first delivery sell of a scrip
    /// on an IST day (once per scrip per day, never on a buy).
    ///
    /// # Errors
    /// As [`Self::charges`]; `Invalid` if `[dp]` is missing.
    pub fn charges_with_dp(
        &self,
        ledger: &mut DpLedger,
        leg: &Leg,
    ) -> Result<ChargeBreakdown, BacktestError> {
        let mut c = self.charges(leg)?;
        if leg.product == Product::Delivery
            && leg.side == LegSide::Sell
            && !ledger
                .paid
                .contains(&(leg.instrument_token, leg.trade_date))
        {
            let dp = self
                .cfg
                .dp
                .as_ref()
                .ok_or_else(|| BacktestError::Invalid("charges.toml has no [dp]".into()))?;
            c.dp = self.dp_amount(dp)?;
            ledger.paid.insert((leg.instrument_token, leg.trade_date));
        }
        Ok(c)
    }

    fn gst_rate(&self) -> Result<Decimal, BacktestError> {
        self.cfg
            .gst
            .as_ref()
            .map(|g| g.rate_pct)
            .ok_or_else(|| BacktestError::Invalid("charges.toml has no [gst]".into()))
    }

    fn dp_amount(&self, dp: &DpCharge) -> Result<Decimal, BacktestError> {
        let gross = if dp.includes_gst {
            dp.per_scrip_total
        } else {
            let g = mul_div(dp.per_scrip_total, self.gst_rate()?, Decimal::ONE_HUNDRED)?;
            dp.per_scrip_total
                .checked_add(g)
                .ok_or(BacktestError::Overflow("dp"))?
        };
        Ok(round(gross, 2))
    }

    fn rate_set(&self, leg: &Leg) -> Result<&'a RateSet, BacktestError> {
        let segment = charge_segment(leg)?;
        let side = match leg.side {
            LegSide::Buy => ChargeSide::Buy,
            LegSide::Sell => ChargeSide::Sell,
        };
        // Latest-for-all asks for the newest set whatever the date; strict uses the trade date.
        let date = match self.policy {
            HistoryPolicy::Strict => leg.trade_date,
            HistoryPolicy::LatestForAll => NaiveDate::MAX,
        };
        self.cfg
            .rate_set_for(self.exchange, segment, side, date)
            .map_err(|_| BacktestError::NoRateSet(leg.trade_date))
    }
}

/// Map the leg onto a charge segment; derivatives never carry `Delivery`, cash never `Carry`.
fn charge_segment(leg: &Leg) -> Result<ChargeSegment, BacktestError> {
    let bad = |m: &str| BacktestError::Invalid(format!("{} {m}", leg.symbol));
    match (leg.instrument, leg.product) {
        (LegInstrument::Cash, Product::Carry) => Err(bad("cash leg cannot use the carry product")),
        (LegInstrument::Cash, _) if leg.segment != Segment::Equity => {
            Err(bad("cash leg must be in the equity segment"))
        }
        (LegInstrument::Cash, Product::Delivery) => Ok(ChargeSegment::EquityDelivery),
        (LegInstrument::Cash, Product::Intraday) => Ok(ChargeSegment::EquityIntraday),
        (_, Product::Delivery) => Err(bad("derivative leg cannot use the delivery product")),
        _ if leg.segment != Segment::EquityDerivatives => Err(bad(
            "derivative leg must be in the equity derivatives segment",
        )),
        (LegInstrument::Future, _) => Ok(ChargeSegment::EquityFutures),
        (LegInstrument::Option(_), _) => Ok(ChargeSegment::EquityOptions),
    }
}

fn mul_div(a: Decimal, b: Decimal, d: Decimal) -> Result<Decimal, BacktestError> {
    a.checked_mul(b)
        .and_then(|v| v.checked_div(d))
        .ok_or(BacktestError::Overflow("charge"))
}

fn round(v: Decimal, dp: u32) -> Decimal {
    v.round_dp_with_strategy(dp, RoundingStrategy::MidpointAwayFromZero)
}

#[cfg(test)]
mod tests;
