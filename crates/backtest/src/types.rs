//! Shared backtest types: legs, charges, fills, trades, equity points, provenance and errors.
//!
//! Rupee amounts are `Decimal` (serialised as strings), prices are `Price` (tick-snapped by
//! construction), quantities are `Qty` in units. Nothing here reads a clock or a config file.

use bazaar_core::money::{Price, Qty};
use bazaar_core::segment::Segment;
use chrono::{DateTime, NaiveDate};
use chrono_tz::Tz;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Direction of one leg. Converted from the risk crate's order side at the gate boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegSide {
    Buy,
    Sell,
}

/// Product type; it decides which charge rates apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Product {
    Delivery,
    Intraday,
}

/// Slippage class (keys of `[slippage.<class>]` in `config/backtest.toml`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlippageClass {
    Equity,
    IndexFutures,
    IndexOptions,
}

impl SlippageClass {
    /// The key used in the config file.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Equity => "equity",
            Self::IndexFutures => "index_futures",
            Self::IndexOptions => "index_options",
        }
    }
}

/// One executed leg, the input of the cost model (slice 1). The trade date is the IST date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Leg {
    pub instrument_token: u32,
    pub symbol: String,
    pub segment: Segment,
    pub product: Product,
    pub side: LegSide,
    pub qty: Qty,
    pub price: Price,
    pub trade_date: NaiveDate,
}

impl Leg {
    /// Turnover of the leg in rupees (`price x qty`), exact.
    #[must_use]
    pub fn turnover(&self) -> Option<Decimal> {
        self.price.value().checked_mul(Decimal::from(self.qty.0))
    }
}

/// Charges on one leg, every line a whole-rupee or 2 dp `Decimal` (rounding rules live in the
/// cost model). `dp` is the depository charge, already including its GST.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ChargeBreakdown {
    #[serde(with = "rust_decimal::serde::str")]
    pub brokerage: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub stt: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub exchange_txn: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub gst: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub sebi: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub stamp: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub dp: Decimal,
}

impl ChargeBreakdown {
    /// Sum of all lines; `None` on overflow.
    #[must_use]
    pub fn total(&self) -> Option<Decimal> {
        [
            self.brokerage,
            self.stt,
            self.exchange_txn,
            self.gst,
            self.sebi,
            self.stamp,
            self.dp,
        ]
        .into_iter()
        .try_fold(Decimal::ZERO, Decimal::checked_add)
    }

    /// Line-by-line sum of two breakdowns (a trade's entry plus exit); `None` on overflow.
    #[must_use]
    pub fn checked_add(&self, o: &Self) -> Option<Self> {
        Some(Self {
            brokerage: self.brokerage.checked_add(o.brokerage)?,
            stt: self.stt.checked_add(o.stt)?,
            exchange_txn: self.exchange_txn.checked_add(o.exchange_txn)?,
            gst: self.gst.checked_add(o.gst)?,
            sebi: self.sebi.checked_add(o.sebi)?,
            stamp: self.stamp.checked_add(o.stamp)?,
            dp: self.dp.checked_add(o.dp)?,
        })
    }
}

/// Why a fill happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FillKind {
    /// Signal on bar `i`, filled at bar `i+1` open plus slippage.
    Entry,
    /// Stop: `min(stop, open)` long, `max(stop, open)` short, then stop slippage.
    Stop,
    /// Limit target: fills at the target, only if the bar trades through it.
    Target,
    /// Time stop or end of data.
    TimeExit,
}

/// One simulated fill with its slippage and charges.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Fill {
    pub leg: Leg,
    pub kind: FillKind,
    pub bar_time: DateTime<Tz>,
    /// Price before slippage (open, stop or target as the rules say).
    pub reference_price: Price,
    /// Rupees of slippage per unit (adverse, always >= 0).
    #[serde(with = "rust_decimal::serde::str")]
    pub slippage_per_unit: Decimal,
    pub charges: ChargeBreakdown,
}

/// A closed round trip. P&L and R are net of charges and slippage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Trade {
    pub strategy_id: String,
    pub entry: Fill,
    pub exit: Fill,
    #[serde(with = "rust_decimal::serde::str")]
    pub gross_pnl: Decimal,
    pub charges: ChargeBreakdown,
    #[serde(with = "rust_decimal::serde::str")]
    pub net_pnl: Decimal,
    /// Rupees risked at entry (entry to stop, times quantity); the R denominator.
    #[serde(with = "rust_decimal::serde::str")]
    pub risk_inr: Decimal,
    /// `net_pnl / risk_inr`; `None` when the risk is zero.
    #[serde(with = "opt_decimal")]
    pub r_multiple: Option<Decimal>,
}

/// Equity after a bar closed (net of all charges so far).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EquityPoint {
    pub time: DateTime<Tz>,
    #[serde(with = "rust_decimal::serde::str")]
    pub equity: Decimal,
}

/// How a run treats a trade date before the earliest charge rate set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryPolicy {
    /// A trade date with no rate set in force is `BacktestError::NoRateSet`.
    #[default]
    Strict,
    /// Use the latest rate set for every date; provenance gets `costs_approximate = true`.
    LatestForAll,
}

/// Evidence label (policy section 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceLabel {
    /// Fewer pooled OOS trades than the floor, or too few folds, or an approximate-cost run.
    LowEvidence,
    Provisional,
    Trusted,
}

/// Where a result came from; every `BacktestResult` carries one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Provenance {
    /// Hex hashes of the config files used, keyed by file name (ordered for determinism).
    pub config_hashes: std::collections::BTreeMap<String, String>,
    pub charges_verified_from: Option<NaiveDate>,
    pub charges_verified_to: Option<NaiveDate>,
    pub data_from: Option<NaiveDate>,
    pub data_to: Option<NaiveDate>,
    pub history_policy: HistoryPolicy,
    /// True when `latest_for_all` was used: such a run is exploratory and never reaches the bar.
    pub costs_approximate: bool,
    /// Distinct (strategy id, parameter set) evaluated in-sample, plus `prior_trials`.
    pub trials: u32,
}

/// Errors of the backtest crate. Typed; never a silent default.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BacktestError {
    #[error("no charge rate set in force on {0}")]
    NoRateSet(NaiveDate),
    #[error("unknown slippage class {0}")]
    UnknownSlippageClass(String),
    #[error("arithmetic overflow in {0}")]
    Overflow(&'static str),
    #[error("invalid input: {0}")]
    Invalid(String),
}

mod opt_decimal {
    use rust_decimal::Decimal;
    use serde::Serializer;

    // serde's `serialize_with` hands over `&Option<T>`, so the signature is fixed.
    #[allow(clippy::ref_option)]
    pub(super) fn serialize<S: Serializer>(v: &Option<Decimal>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(d) => s.serialize_str(&d.to_string()),
            None => s.serialize_none(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn leg() -> Leg {
        Leg {
            instrument_token: 1,
            symbol: "TCS".into(),
            segment: Segment::Equity,
            product: Product::Delivery,
            side: LegSide::Buy,
            qty: Qty(10),
            price: Price::snap_to_tick(d("100.05"), d("0.05")).unwrap(),
            trade_date: NaiveDate::from_ymd_opt(2026, 11, 20).unwrap(),
        }
    }

    #[test]
    fn turnover_is_price_times_qty() {
        assert_eq!(leg().turnover(), Some(d("1000.50")));
    }

    #[test]
    fn charge_total_and_add_sum_every_line() {
        let a = ChargeBreakdown {
            brokerage: d("1"),
            stt: d("2"),
            exchange_txn: d("3"),
            gst: d("4"),
            sebi: d("5"),
            stamp: d("6"),
            dp: d("7"),
        };
        assert_eq!(a.total(), Some(d("28")));
        let b = a.checked_add(&a).unwrap();
        assert_eq!(b.stt, d("4"));
        assert_eq!(b.total(), Some(d("56")));
    }

    #[test]
    fn charge_overflow_is_none() {
        let a = ChargeBreakdown {
            brokerage: Decimal::MAX,
            stt: Decimal::MAX,
            ..ChargeBreakdown::default()
        };
        assert_eq!(a.total(), None);
        assert_eq!(a.checked_add(&a), None);
    }

    #[test]
    fn slippage_class_keys_match_serde_names() {
        #[derive(Deserialize)]
        struct W {
            c: SlippageClass,
        }
        for c in [
            SlippageClass::Equity,
            SlippageClass::IndexFutures,
            SlippageClass::IndexOptions,
        ] {
            let w: W = toml::from_str(&format!("c = \"{}\"", c.key())).unwrap();
            assert_eq!(w.c, c);
        }
    }

    #[test]
    fn history_policy_defaults_to_strict_and_parses() {
        #[derive(Deserialize)]
        struct W {
            p: HistoryPolicy,
        }
        assert_eq!(HistoryPolicy::default(), HistoryPolicy::Strict);
        let w: W = toml::from_str("p = \"latest_for_all\"").unwrap();
        assert_eq!(w.p, HistoryPolicy::LatestForAll);
        assert!(toml::from_str::<W>("p = \"fallback\"").is_err());
    }

    #[test]
    fn evidence_labels_order_low_to_trusted() {
        assert!(EvidenceLabel::LowEvidence < EvidenceLabel::Provisional);
        assert!(EvidenceLabel::Provisional < EvidenceLabel::Trusted);
    }

    #[test]
    fn errors_render_the_date() {
        let e = BacktestError::NoRateSet(NaiveDate::from_ymd_opt(2024, 1, 2).unwrap());
        assert_eq!(e.to_string(), "no charge rate set in force on 2024-01-02");
    }
}
