//! Cost model: Indian charges per leg, driven only by the typed `config/charges.toml`.
//!
//! Rounding reproduces the owner's calculator samples within Rs 0.05 per line (SEBI rounding is
//! UNVERIFIED; hypothesis: nearest 0.05): STT and stamp to whole rupees, brokerage, exchange
//! transaction, SEBI and GST to 2 dp, all half-up and only after the full multiplication.
//! GST = `gst.rate_pct` x (brokerage + exchange txn + SEBI); STT, stamp, IPFT and DP are outside
//! the base. IPFT (from `ipft_pct`, an exchange levy) is folded into the `exchange_txn` line.
//! The engine entry is [`CostModel::charges_with_dp`]; the leg's venue is an explicit argument.

use std::collections::HashSet;

use bazaar_core::config::{
    Brokerage, ChargeExchange, ChargeSegment, ChargeSide, ChargesConfig, DpCharge, RateSet,
    TurnoverBasis,
};
use bazaar_core::segment::Segment;
use chrono::NaiveDate;
use rust_decimal::{Decimal, RoundingStrategy};

use crate::types::{
    BacktestError, ChargeBreakdown, HistoryPolicy, Leg, LegInstrument, LegSide, Product,
};

/// Rupees per crore, the SEBI fee base.
const CRORE: Decimal = Decimal::from_parts(10_000_000, 0, 0, false, 0);

/// Whether a leg can carry the depository charge (a delivery sell; once per scrip-day in
/// practice). The one predicate for both the charge and the provenance range, so they cannot drift.
fn levies_dp(leg: &Leg) -> bool {
    leg.product == Product::Delivery && leg.side == LegSide::Sell
}

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

    /// Undo a DP slot booked by `charges_with_dp` when the caller's later step failed.
    pub(crate) fn release(&mut self, token: u32, date: NaiveDate) {
        self.paid.remove(&(token, date));
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
    /// A model for one venue. `Leg` carries no exchange, so it is explicit (no silent NSE default).
    #[must_use]
    pub const fn new(
        cfg: &'a ChargesConfig,
        policy: HistoryPolicy,
        exchange: ChargeExchange,
    ) -> Self {
        Self {
            cfg,
            policy,
            exchange,
        }
    }

    /// The history policy this model charges under.
    #[must_use]
    pub const fn policy(&self) -> HistoryPolicy {
        self.policy
    }

    /// True when `latest_for_all` is in use: provenance must say `costs_approximate = true`.
    #[must_use]
    pub fn costs_approximate(&self) -> bool {
        self.policy == HistoryPolicy::LatestForAll
    }

    /// True when this leg's cost used a rate set other than the one in force on its trade date
    /// (only under `latest_for_all`); slice 4 counts these for provenance.
    ///
    /// # Errors
    /// As [`Self::charges_excl_dp`] for a missing segment/side row.
    pub fn leg_is_approximate(&self, leg: &Leg) -> Result<bool, BacktestError> {
        if self.policy == HistoryPolicy::Strict {
            return Ok(false);
        }
        let latest = self.lookup(leg, NaiveDate::MAX)?;
        Ok(self
            .lookup(leg, leg.trade_date)
            .map_or(true, |s| s.effective_from != latest.effective_from))
    }

    /// Earliest and latest `verified_on` over what `legs` were actually charged with: the rate set
    /// each leg used (per the history policy), the GST table, and the DP table for delivery sells
    /// (there is one `[dp]` table and the first delivery sell of a run always pays it, so its date
    /// is exact whenever a delivery sell is present). `None` for no legs.
    ///
    /// Not included: `[option_exercise]`. Exercise STT is not charged yet; when it is, this must
    /// add that table's `verified_on` or the range silently narrows (tracked as QUAN-94-5 in the
    /// F9 record's Deferred list).
    ///
    /// # Errors
    /// As [`Self::charges_excl_dp`] for a missing rate set, GST table or DP table.
    pub fn used_verified_range<'l>(
        &self,
        legs: impl IntoIterator<Item = &'l Leg>,
    ) -> Result<Option<(NaiveDate, NaiveDate)>, BacktestError> {
        let mut range: Option<(NaiveDate, NaiveDate)> = None;
        let mut note = |d: NaiveDate| {
            range = Some(range.map_or((d, d), |(lo, hi)| (lo.min(d), hi.max(d))));
        };
        for leg in legs {
            note(self.rate_set(leg)?.verified_on);
            note(
                self.cfg
                    .gst
                    .as_ref()
                    .ok_or_else(|| BacktestError::Invalid("charges.toml has no [gst]".into()))?
                    .verified_on,
            );
            if levies_dp(leg) {
                note(
                    self.cfg
                        .dp
                        .as_ref()
                        .ok_or_else(|| BacktestError::Invalid("charges.toml has no [dp]".into()))?
                        .verified_on,
                );
            }
        }
        Ok(range)
    }

    /// Charges on one leg with the DEPOSITORY CHARGE OMITTED (`dp` is always 0): a delivery sell
    /// booked through this is Rs 15.34-class too cheap. The engine must call
    /// [`Self::charges_with_dp`]; this stateless form is for tests and per-line checks.
    ///
    /// # Errors
    /// `NoRateSet` (strict policy, no set in force on the trade date), `Invalid` for a zero
    /// quantity or price, an impossible segment/product/instrument mix, a rate row whose
    /// turnover basis does not match the instrument, or a missing row; `Overflow` on overflow.
    pub fn charges_excl_dp(&self, leg: &Leg) -> Result<ChargeBreakdown, BacktestError> {
        if leg.qty.0 == 0 {
            return Err(BacktestError::Invalid("zero quantity".into()));
        }
        if leg.price.value().is_zero() {
            return Err(BacktestError::Invalid("zero price".into()));
        }
        let set = self.rate_set(leg)?;
        let want = if matches!(leg.instrument, LegInstrument::Option(_)) {
            TurnoverBasis::Premium
        } else {
            TurnoverBasis::Notional
        };
        if set.turnover_basis != want {
            return Err(BacktestError::Invalid(format!(
                "{}: rate row turnover basis {:?} does not match the instrument ({want:?})",
                leg.symbol, set.turnover_basis
            )));
        }
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

    /// The engine entry: [`Self::charges_excl_dp`] plus the depository charge on the first delivery sell of a scrip
    /// on an IST day (once per scrip per day, never on a buy).
    ///
    /// # Errors
    /// As [`Self::charges_excl_dp`]; `Invalid` if `[dp]` is missing.
    pub fn charges_with_dp(
        &self,
        ledger: &mut DpLedger,
        leg: &Leg,
    ) -> Result<ChargeBreakdown, BacktestError> {
        let mut c = self.charges_excl_dp(leg)?;
        if levies_dp(leg)
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
        // Latest-for-all means the newest set in the file for this row, whatever its date (a
        // future-dated set included: there is no clock here); strict uses the trade date.
        let date = match self.policy {
            HistoryPolicy::Strict => leg.trade_date,
            HistoryPolicy::LatestForAll => NaiveDate::MAX,
        };
        self.lookup(leg, date)
    }

    fn lookup(&self, leg: &Leg, date: NaiveDate) -> Result<&'a RateSet, BacktestError> {
        let segment = charge_segment(leg)?;
        let side = match leg.side {
            LegSide::Buy => ChargeSide::Buy,
            LegSide::Sell => ChargeSide::Sell,
        };
        self.cfg
            .rate_set_for(self.exchange, segment, side, date)
            .map_err(|e| {
                // NoRateSet only when some set exists for the row but none was in force yet;
                // a row missing from the file altogether is a config gap, not a date problem.
                let row_exists =
                    self.cfg.rate_set.iter().any(|r| {
                        r.exchange == self.exchange && r.segment == segment && r.side == side
                    });
                if row_exists {
                    BacktestError::NoRateSet(leg.trade_date)
                } else {
                    BacktestError::Invalid(e.to_string())
                }
            })
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
