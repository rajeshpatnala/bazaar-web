//! Ports the scanner reads through. The scanner owns them so it needs no `backtest` edge:
//! F13 composition adapts F9 stats, the NSE ban list and the events calendar to these traits.

use bazaar_core::config::Event;
use chrono::{DateTime, NaiveDate};
use chrono_tz::Tz;

use crate::types::Evidence;

/// Backtest statistics for a strategy. `None` means no evidence: the caller must label it so.
pub trait EvidenceSource: Send + Sync {
    /// Stats of `strategy_id` on this instrument (out of sample, net of costs when available).
    fn for_instrument(&self, strategy_id: &str, instrument_token: u64) -> Option<Evidence>;
    /// Strategy-wide stats; the shrink prior only when they hold enough trades, else neutral.
    fn for_strategy(&self, strategy_id: &str) -> Option<Evidence>;
}

/// State of the ban list as the source reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BanListStatus {
    /// Names (tradingsymbols) in the ban period; `as_of` is the source's date, if it gave one.
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
