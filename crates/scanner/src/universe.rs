//! Universe filters: the ban list gate (fail closed) and the liquidity measure. All thresholds
//! come from the `[scanner]` config; nothing here reads a clock (the caller passes `now`).

use std::collections::BTreeSet;

use bazaar_core::candle::Candle;
use chrono::{DateTime, Duration};
use chrono_tz::Tz;
use rust_decimal::Decimal;

use crate::ports::BanListStatus;

/// Window of the "median 20-day traded value" measure named in the plan and the config key.
pub const MEDIAN_WINDOW_BARS: usize = 20;

/// Why a name did not reach the strategies. Always reported, never silent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropReason {
    BanListUnavailable(String),
    BanListUndated,
    BanListStale {
        age_hours: i64,
    },
    Banned,
    Illiquid {
        median_traded_value: Decimal,
    },
    CircuitLocked,
    CircuitStatusUnknown,
    DecisionBarNotClosed,
    ViewUnavailable,
    ShortHistory {
        bars: usize,
        needed: usize,
    },
    NoBarOnDecisionDate,
    RegimeFailed(String),
    /// A strategy's signal was already past its `valid_until` at the clock.
    SignalExpired {
        strategy_id: String,
    },
}

/// Upper-case, trimmed form used to compare tradingsymbols.
#[must_use]
pub fn normalise(symbol: &str) -> String {
    symbol.trim().to_uppercase()
}

/// The set of banned names, or the reason the list cannot be trusted. Unavailable, undated,
/// dated in the future, or older than `max_age_hours` all fail closed: every candidate is dropped.
///
/// # Errors
/// The `DropReason` to attach to every candidate.
pub fn banned_set(
    status: &BanListStatus,
    now: DateTime<Tz>,
    max_age_hours: u32,
) -> Result<BTreeSet<String>, DropReason> {
    match status {
        BanListStatus::Unavailable { reason } => {
            Err(DropReason::BanListUnavailable(reason.clone()))
        }
        BanListStatus::Available { as_of: None, .. } => Err(DropReason::BanListUndated),
        BanListStatus::Available {
            symbols,
            as_of: Some(as_of),
        } => {
            let age = now.signed_duration_since(*as_of);
            if age < Duration::zero() || age > Duration::hours(i64::from(max_age_hours)) {
                return Err(DropReason::BanListStale {
                    age_hours: age.num_hours(),
                });
            }
            Ok(symbols.iter().map(|s| normalise(s)).collect())
        }
    }
}

/// Median of `close * volume` over the last `window` bars; `None` with fewer than `window` bars
/// or on overflow (the caller drops the name). Volumes must be adjusted like the prices.
#[must_use]
pub fn median_traded_value(bars: &[Candle], window: usize) -> Option<Decimal> {
    let tail = bars.get(bars.len().checked_sub(window)?..)?;
    let mut v: Vec<Decimal> = tail
        .iter()
        .map(|b| b.close().checked_mul(Decimal::from(b.volume())))
        .collect::<Option<_>>()?;
    v.sort();
    let n = v.len();
    if n == 0 {
        return None;
    }
    let hi = *v.get(n / 2)?;
    if n % 2 == 1 {
        return Some(hi);
    }
    let lo = *v.get(n / 2 - 1)?;
    Some((lo + hi) / Decimal::TWO)
}

#[cfg(test)]
mod tests;
