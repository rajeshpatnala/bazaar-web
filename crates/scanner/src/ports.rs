//! Ports the scanner reads through. The scanner owns them so it needs no `backtest` edge:
//! F13 composition adapts F9 stats, the NSE ban list and the events calendar to these traits.

use bazaar_core::config::Event;
use chrono::{DateTime, NaiveDate};
use chrono_tz::Tz;

use crate::gate_ctx::{ContextUnavailable, ScanGateContext};
use crate::types::Evidence;

/// Source of the gate inputs (portfolio snapshot, limits, costs, data and mode evidence). Sync, no
/// network: the composition (BZ-519) fills it from stores already read. Called once per scan.
pub trait GateContextSource: Send + Sync {
    /// # Errors
    /// `ContextUnavailable` when any part cannot be read: every idea is then blocked.
    fn context(&self) -> Result<ScanGateContext, ContextUnavailable>;
}

/// Backtest statistics for a strategy. `None` means no evidence: the caller must label it so.
pub trait EvidenceSource: Send + Sync {
    /// Stats of `strategy_id` on this instrument (out of sample, net of costs when available).
    /// Only trades closed before `as_of` (the decision bar) may count: no look-ahead.
    fn for_instrument(
        &self,
        strategy_id: &str,
        instrument_token: u64,
        as_of: DateTime<Tz>,
    ) -> Option<Evidence>;
    /// Strategy-wide stats (same `as_of` cutoff); the shrink prior only when they hold enough
    /// trades, else `None` (no evidence).
    fn for_strategy(&self, strategy_id: &str, as_of: DateTime<Tz>) -> Option<Evidence>;
}

/// State of the ban list as the source reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BanListStatus {
    /// Names (tradingsymbols) in the ban period, compared trimmed and upper-case. Adapters (F13)
    /// must cap the list and event sizes; `as_of` is the instant the source published the list. Its
    /// age is measured against the clock, not the decision date, so a Friday-evening list is stale on a
    /// Monday-morning scan (fails closed; owner to confirm, RISK-102-10).
    Available {
        symbols: Vec<String>,
        as_of: Option<DateTime<Tz>>,
    },
    /// No list could be obtained.
    Unavailable { reason: String },
}

/// Ban list source. The scanner fails closed on `Unavailable`, an undated list, or a list older
/// than `max_ban_list_age_hours`.
pub trait BanList: Send + Sync {
    fn status(&self) -> BanListStatus;
}

/// Event calendar (results, policy dates, ex-dates). An empty calendar is reported as a warning,
/// never read as "no events".
pub trait EventCalendar: Send + Sync {
    /// Events from `from` within `days` calendar days touching `tradingsymbol` or the market.
    fn upcoming(&self, tradingsymbol: &str, from: NaiveDate, days: u32) -> Vec<Event>;
    /// True when the calendar holds no real rows (placeholder), so absence proves nothing.
    fn is_empty(&self) -> bool;
}
