//! Portfolio snapshot (journal-built by F10). Every guard input is tri-state and there is no
//! `Default`: an unknown input rejects entries.

use crate::types::{Side, Tri};
use chrono::DateTime;
use chrono_tz::Tz;
use rust_decimal::Decimal;

/// One held position with the stop stored for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Position {
    pub symbol: String,
    /// The side that opened it (`Buy` = long).
    pub side: Side,
    pub qty: u32,
    /// Stop stored in the journal; `Unknown` when none is recorded.
    pub stop: Tri<Decimal>,
    /// Price the open risk to `stop` is measured from (G04b): the average entry price, or the
    /// mark when that is higher risk. `Unknown` rejects entries under G04b.
    pub risk_price: Tri<Decimal>,
}

/// Latest mark for one symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    pub symbol: String,
    pub price: Decimal,
}

/// Last stop-out time for one symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopOut {
    pub symbol: String,
    pub at: DateTime<Tz>,
}

/// State of the account at `as_of`. All guard state comes from here, never process memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortfolioSnapshot {
    pub as_of: Tri<DateTime<Tz>>,
    pub positions: Tri<Vec<Position>>,
    /// Consecutive losing trades within the current IST session day.
    pub consecutive_losses: Tri<u32>,
    /// Today's P&L: realised + unrealised, net of charges.
    pub day_pnl: Tri<Decimal>,
    /// This week's P&L: realised + unrealised, net of charges.
    pub week_pnl: Tri<Decimal>,
    pub entries_today: Tri<u32>,
    pub orders_last_minute: Tri<u32>,
    pub overrides_today: Tri<u32>,
    pub overrides_week: Tri<u32>,
    pub kill_switch: Tri<bool>,
    pub charges_buffer: Tri<Decimal>,
    /// Cash the account started with (paper: `paper_starting_cash_inr`).
    pub starting_cash: Tri<Decimal>,
    /// Cumulative realised P&L since the start, net of charges.
    pub realised_pnl: Tri<Decimal>,
    /// Mark-to-market equity now (cash plus open positions at their marks).
    pub equity_mtm: Tri<Decimal>,
    /// Latest mark per symbol with an open position (G03/G19 unrealised P&L).
    pub marks: Tri<Vec<Mark>>,
    pub last_stop_outs: Tri<Vec<StopOut>>,
}

impl PortfolioSnapshot {
    /// A snapshot where nothing is known: every entry is rejected against it.
    #[must_use]
    pub const fn unknown() -> Self {
        Self {
            as_of: Tri::Unknown,
            positions: Tri::Unknown,
            consecutive_losses: Tri::Unknown,
            day_pnl: Tri::Unknown,
            week_pnl: Tri::Unknown,
            entries_today: Tri::Unknown,
            orders_last_minute: Tri::Unknown,
            overrides_today: Tri::Unknown,
            overrides_week: Tri::Unknown,
            kill_switch: Tri::Unknown,
            charges_buffer: Tri::Unknown,
            starting_cash: Tri::Unknown,
            realised_pnl: Tri::Unknown,
            equity_mtm: Tri::Unknown,
            marks: Tri::Unknown,
            last_stop_outs: Tri::Unknown,
        }
    }

    /// The held position for `symbol`, if positions are known and one exists.
    #[must_use]
    pub fn position(&self, symbol: &str) -> Option<&Position> {
        self.positions
            .known()?
            .iter()
            .find(|p| p.symbol == symbol && p.qty > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_snapshot_has_nothing_known() {
        let s = PortfolioSnapshot::unknown();
        assert!(s.as_of.is_unknown() && s.positions.is_unknown());
        assert!(s.day_pnl.is_unknown() && s.kill_switch.is_unknown());
        assert!(s.starting_cash.is_unknown() && s.realised_pnl.is_unknown());
        assert!(s.equity_mtm.is_unknown() && s.marks.is_unknown());
        assert!(s.position("ABC").is_none());
    }

    #[test]
    fn position_lookup_ignores_flat_and_other_symbols() {
        let mut s = PortfolioSnapshot::unknown();
        s.positions = Tri::Known(vec![
            Position {
                symbol: "ABC".into(),
                side: Side::Buy,
                qty: 0,
                stop: Tri::Unknown,
                risk_price: Tri::Unknown,
            },
            Position {
                symbol: "XYZ".into(),
                side: Side::Sell,
                qty: 5,
                stop: Tri::Unknown,
                risk_price: Tri::Unknown,
            },
        ]);
        assert!(s.position("ABC").is_none());
        assert_eq!(s.position("XYZ").map(|p| p.qty), Some(5));
    }
}
