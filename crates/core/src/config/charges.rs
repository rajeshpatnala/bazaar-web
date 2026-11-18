//! `charges.toml`: effective-dated statutory and broker rate sets, owner-editable.
//!
//! Schema: `bz-backtest-risk` (`references/charges-toml.md`). Rates are TOML strings parsed to
//! `Decimal` (a TOML float is a parse error). No rate lives in Rust; a lookup for a date with no
//! rate set in force is an error, never a fallback. The cost arithmetic itself is the backtest
//! crate's `CostModel`; this file only loads and selects rates.

use std::collections::HashSet;
use std::fmt;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::Deserialize;

use super::files::MAX_TEXT_CHARS;
use super::{ConfigError, ConfigFile, impl_header};

/// Most `[[rate_set]]` rows accepted (bound before any write path exists).
pub const MAX_RATE_SETS: usize = 1000;

/// Exchange a rate set applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
pub enum ChargeExchange {
    #[serde(rename = "NSE")]
    Nse,
    #[serde(rename = "BSE")]
    Bse,
}

/// Product segment for charges (distinct from the calendar `Segment`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChargeSegment {
    EquityDelivery,
    EquityIntraday,
    EquityFutures,
    EquityOptions,
}

/// Buy or sell leg.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChargeSide {
    Buy,
    Sell,
}

/// What the percentage rates are applied to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnoverBasis {
    Notional,
    Premium,
}

/// How a run treats history older than the newest rate set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryPolicy {
    EffectiveDated,
    LatestForAll,
}

/// Brokerage rule of a rate set.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Brokerage {
    Zero {},
    Flat {
        #[serde(deserialize_with = "super::decimal_str::deserialize")]
        flat: Decimal,
    },
    Pct {
        #[serde(deserialize_with = "super::decimal_str::deserialize")]
        pct: Decimal,
    },
    FlatOrPctLower {
        #[serde(deserialize_with = "super::decimal_str::deserialize")]
        flat: Decimal,
        #[serde(deserialize_with = "super::decimal_str::deserialize")]
        pct: Decimal,
    },
}

/// `[meta]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChargesMeta {
    pub broker: String,
    pub currency: String,
    pub max_age_days: u32,
    pub history_policy: HistoryPolicy,
}

/// `[gst]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GstRate {
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub rate_pct: Decimal,
    pub verified_on: NaiveDate,
    pub source: String,
}

/// `[dp]`: depository charge per scrip per day on delivery sells.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DpCharge {
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub per_scrip_total: Decimal,
    pub includes_gst: bool,
    pub verified_on: NaiveDate,
    pub source: String,
}

/// `[option_exercise]`: STT on the intrinsic value of exercised options.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptionExercise {
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub stt_pct: Decimal,
    pub verified_on: NaiveDate,
    pub source: String,
}

/// One `[[rate_set]]`: the rates for (exchange, segment, side) from `effective_from`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RateSet {
    pub exchange: ChargeExchange,
    pub segment: ChargeSegment,
    pub side: ChargeSide,
    pub effective_from: NaiveDate,
    pub verified_on: NaiveDate,
    pub source: String,
    pub turnover_basis: TurnoverBasis,
    pub brokerage: Brokerage,
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub stt_ctt_pct: Decimal,
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub exchange_txn_pct: Decimal,
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub sebi_per_crore: Decimal,
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub ipft_pct: Decimal,
    #[serde(deserialize_with = "super::decimal_str::deserialize")]
    pub stamp_pct: Decimal,
}

/// A lookup found no rate set in force. Never answered with a fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoRateSet {
    pub exchange: ChargeExchange,
    pub segment: ChargeSegment,
    pub side: ChargeSide,
    pub date: NaiveDate,
}

impl fmt::Display for NoRateSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "charges.toml: no rate set in force for {:?} {:?} {:?} on {}",
            self.exchange, self.segment, self.side, self.date
        )
    }
}

impl std::error::Error for NoRateSet {}

/// `charges.toml`. A placeholder file has no tables; a real one needs `[meta]` and `[gst]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChargesConfig {
    pub schema_version: u32,
    #[serde(default)]
    pub placeholder: bool,
    #[serde(default)]
    pub verified_on: Option<NaiveDate>,
    pub source: String,
    #[serde(default)]
    pub meta: Option<ChargesMeta>,
    #[serde(default)]
    pub gst: Option<GstRate>,
    #[serde(default)]
    pub dp: Option<DpCharge>,
    #[serde(default)]
    pub option_exercise: Option<OptionExercise>,
    #[serde(default)]
    pub rate_set: Vec<RateSet>,
}

impl ChargesConfig {
    /// The rate set in force on `date`: the one with the latest `effective_from <= date`.
    pub fn rate_set_for(
        &self,
        exchange: ChargeExchange,
        segment: ChargeSegment,
        side: ChargeSide,
        date: NaiveDate,
    ) -> Result<&RateSet, NoRateSet> {
        self.rate_set
            .iter()
            .filter(|r| {
                r.exchange == exchange
                    && r.segment == segment
                    && r.side == side
                    && r.effective_from <= date
            })
            .max_by_key(|r| r.effective_from)
            .ok_or(NoRateSet {
                exchange,
                segment,
                side,
                date,
            })
    }
}

fn bad(key: impl Into<String>, reason: &str) -> ConfigError {
    ConfigError::invalid(ChargesConfig::FILE, key, reason)
}

fn check_text(key: &str, s: &str) -> Result<(), ConfigError> {
    if s.trim().is_empty() {
        return Err(bad(key, "must not be empty"));
    }
    if s.chars().count() > MAX_TEXT_CHARS {
        return Err(bad(key, "too long"));
    }
    Ok(())
}

fn check_rate(key: &str, v: Decimal, max: Option<Decimal>) -> Result<(), ConfigError> {
    if v.is_sign_negative() && !v.is_zero() {
        return Err(bad(key, "must not be negative"));
    }
    if max.is_some_and(|m| v > m) {
        return Err(bad(key, "above the allowed maximum"));
    }
    Ok(())
}

impl ChargesConfig {
    fn validate_sets(&self) -> Result<(), ConfigError> {
        if self.rate_set.len() > MAX_RATE_SETS {
            return Err(bad("rate_set", "too many rate sets"));
        }
        let pct_max = Some(Decimal::ONE_HUNDRED);
        let mut seen = HashSet::new();
        for (i, r) in self.rate_set.iter().enumerate() {
            let k = |f: &str| format!("rate_set[{i}].{f}");
            check_text(&k("source"), &r.source)?;
            if !seen.insert((r.exchange, r.segment, r.side, r.effective_from)) {
                return Err(bad(
                    format!("rate_set[{i}]"),
                    "duplicate (exchange, segment, side, effective_from)",
                ));
            }
            match &r.brokerage {
                Brokerage::Zero {} => {}
                Brokerage::Flat { flat } => check_rate(&k("brokerage.flat"), *flat, None)?,
                Brokerage::Pct { pct } => check_rate(&k("brokerage.pct"), *pct, pct_max)?,
                Brokerage::FlatOrPctLower { flat, pct } => {
                    check_rate(&k("brokerage.flat"), *flat, None)?;
                    check_rate(&k("brokerage.pct"), *pct, pct_max)?;
                }
            }
            check_rate(&k("stt_ctt_pct"), r.stt_ctt_pct, pct_max)?;
            check_rate(&k("exchange_txn_pct"), r.exchange_txn_pct, pct_max)?;
            check_rate(&k("sebi_per_crore"), r.sebi_per_crore, None)?;
            check_rate(&k("ipft_pct"), r.ipft_pct, pct_max)?;
            check_rate(&k("stamp_pct"), r.stamp_pct, pct_max)?;
        }
        Ok(())
    }

    fn validate_tables(&self) -> Result<(), ConfigError> {
        if self.placeholder {
            let any = self.meta.is_some()
                || self.gst.is_some()
                || self.dp.is_some()
                || self.option_exercise.is_some()
                || !self.rate_set.is_empty();
            return if any {
                Err(bad("placeholder", "a placeholder file must hold no tables"))
            } else {
                Ok(())
            };
        }
        let Some(meta) = &self.meta else {
            return Err(bad("meta", "is required"));
        };
        check_text("meta.broker", &meta.broker)?;
        check_text("meta.currency", &meta.currency)?;
        let Some(gst) = &self.gst else {
            return Err(bad("gst", "is required"));
        };
        check_text("gst.source", &gst.source)?;
        check_rate("gst.rate_pct", gst.rate_pct, Some(Decimal::ONE_HUNDRED))?;
        let uses = |s: ChargeSegment| self.rate_set.iter().any(|r| r.segment == s);
        match &self.dp {
            Some(dp) => {
                check_text("dp.source", &dp.source)?;
                check_rate("dp.per_scrip_total", dp.per_scrip_total, None)?;
            }
            None if uses(ChargeSegment::EquityDelivery) => {
                return Err(bad("dp", "is required with equity_delivery rate sets"));
            }
            None => {}
        }
        match &self.option_exercise {
            Some(o) => {
                check_text("option_exercise.source", &o.source)?;
                check_rate(
                    "option_exercise.stt_pct",
                    o.stt_pct,
                    Some(Decimal::ONE_HUNDRED),
                )?;
            }
            None if uses(ChargeSegment::EquityOptions) => {
                return Err(bad(
                    "option_exercise",
                    "is required with equity_options rate sets",
                ));
            }
            None => {}
        }
        Ok(())
    }
}

impl ConfigFile for ChargesConfig {
    const FILE: &'static str = "charges.toml";
    impl_header!();

    fn validate(&self) -> Result<(), ConfigError> {
        self.validate_tables()?;
        self.validate_sets()
    }
}
