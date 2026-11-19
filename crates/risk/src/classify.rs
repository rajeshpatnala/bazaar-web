//! The gate classifies every order; the caller's label is never trusted.
//!
//! Opposite side and qty <= held = `Exit`; any excess over held, or a flip, = `Entry` for the
//! excess; qty 0 with a `new_stop` on a held position = `StopMove`. Unknown positions = `Entry`.

use crate::snapshot::PortfolioSnapshot;
use crate::types::{OrderIntent, Side};
use rust_decimal::Decimal;
use std::fmt;

/// What an order really is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Entry,
    Exit,
    StopMove,
}

/// Direction of a stop move relative to the stored stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopDirection {
    /// Closer to price: lowers the loss (allowed).
    Tighten,
    /// Further from price: raises the loss (G09 rejects).
    Widen,
    Unchanged,
    /// No stored stop to compare with.
    Unknown,
}

/// Result of classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Classification {
    pub action: Action,
    /// Quantity that opens or adds exposure (entry rules apply to this).
    pub entry_qty: u32,
    /// Quantity that closes held exposure.
    pub exit_qty: u32,
    /// Set for `StopMove` only.
    pub stop_direction: Option<StopDirection>,
}

/// An intent that cannot be classified (the gate rejects it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassifyError {
    /// Quantity 0 without a stop move.
    ZeroQty,
    /// A stop move for a symbol with no known held position.
    StopMoveWithoutPosition,
    /// A stop change together with a non-zero quantity: send them as separate orders.
    StopWithQty,
}

impl fmt::Display for ClassifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroQty => f.write_str("order quantity is 0 and no stop change was given"),
            Self::StopMoveWithoutPosition => {
                f.write_str("a stop change needs a known open position in this symbol")
            }
            Self::StopWithQty => {
                f.write_str("a stop change cannot be combined with an order quantity")
            }
        }
    }
}

impl std::error::Error for ClassifyError {}

/// Direction of `new` versus `old` for a position opened on `side`.
#[must_use]
pub fn stop_direction(side: Side, old: Decimal, new: Decimal) -> StopDirection {
    if new == old {
        return StopDirection::Unchanged;
    }
    match (side, new > old) {
        (Side::Buy, true) | (Side::Sell, false) => StopDirection::Tighten,
        (Side::Buy, false) | (Side::Sell, true) => StopDirection::Widen,
    }
}

/// Classify `intent` against the snapshot's positions.
pub fn classify(
    intent: &OrderIntent,
    snapshot: &PortfolioSnapshot,
) -> Result<Classification, ClassifyError> {
    let qty = intent.qty.0;
    if qty > 0 && intent.new_stop.is_some() {
        return Err(ClassifyError::StopWithQty);
    }
    // More than one open row for a symbol is ambiguous: treat the order as a full entry.
    let rows = snapshot.positions.known().map_or(0, |v| {
        v.iter()
            .filter(|p| p.symbol == intent.symbol && p.qty > 0)
            .count()
    });
    let held = if rows > 1 {
        None
    } else {
        snapshot.position(&intent.symbol)
    };

    if qty == 0 {
        let Some(new_stop) = intent.new_stop else {
            return Err(ClassifyError::ZeroQty);
        };
        let Some(pos) = held else {
            return Err(ClassifyError::StopMoveWithoutPosition);
        };
        let dir = pos.stop.known().map_or(StopDirection::Unknown, |old| {
            stop_direction(pos.side, *old, new_stop)
        });
        return Ok(Classification {
            action: Action::StopMove,
            entry_qty: 0,
            exit_qty: 0,
            stop_direction: Some(dir),
        });
    }

    let entry = |entry_qty, exit_qty| Classification {
        action: Action::Entry,
        entry_qty,
        exit_qty,
        stop_direction: None,
    };
    Ok(match held {
        Some(p) if p.side != intent.side && qty <= p.qty => Classification {
            action: Action::Exit,
            entry_qty: 0,
            exit_qty: qty,
            stop_direction: None,
        },
        Some(p) if p.side != intent.side => entry(qty.saturating_sub(p.qty), p.qty),
        _ => entry(qty, 0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::Position;
    use crate::types::Tri;
    use bazaar_core::money::Qty;

    fn d(n: i64) -> Decimal {
        Decimal::from(n)
    }

    fn intent(side: Side, qty: u32, new_stop: Option<Decimal>) -> OrderIntent {
        OrderIntent {
            symbol: "ABC".into(),
            side,
            qty: Qty(qty),
            price: d(500),
            stop: Some(d(490)),
            new_stop,
            lot_size: 1,
            tick_size: Decimal::new(5, 2),
        }
    }

    fn snap(side: Side, qty: u32, stop: Tri<Decimal>) -> PortfolioSnapshot {
        let mut s = PortfolioSnapshot::unknown();
        s.positions = Tri::Known(vec![Position {
            symbol: "ABC".into(),
            side,
            qty,
            stop,
            avg_cost: Tri::Unknown,
        }]);
        s
    }

    fn parts(c: Classification) -> (Action, u32, u32) {
        (c.action, c.entry_qty, c.exit_qty)
    }

    #[test]
    fn no_position_is_entry() {
        let mut s = PortfolioSnapshot::unknown();
        s.positions = Tri::Known(vec![]);
        let c = classify(&intent(Side::Buy, 10, None), &s).unwrap();
        assert_eq!(parts(c), (Action::Entry, 10, 0));
    }

    #[test]
    fn unknown_positions_make_everything_an_entry() {
        let c = classify(&intent(Side::Sell, 10, None), &PortfolioSnapshot::unknown()).unwrap();
        assert_eq!(parts(c), (Action::Entry, 10, 0));
    }

    #[test]
    fn same_side_adds_are_entries() {
        let c = classify(
            &intent(Side::Buy, 5, None),
            &snap(Side::Buy, 10, Tri::Unknown),
        )
        .unwrap();
        assert_eq!(parts(c), (Action::Entry, 5, 0));
    }

    #[test]
    fn opposite_up_to_held_is_exit() {
        for q in [1, 9, 10] {
            let s = snap(Side::Buy, 10, Tri::Unknown);
            let c = classify(&intent(Side::Sell, q, None), &s).unwrap();
            assert_eq!(parts(c), (Action::Exit, 0, q));
        }
    }

    #[test]
    fn oversized_exit_is_entry_for_the_excess() {
        let s = snap(Side::Buy, 10, Tri::Unknown);
        let c = classify(&intent(Side::Sell, 15, None), &s).unwrap();
        assert_eq!(parts(c), (Action::Entry, 5, 10));
    }

    #[test]
    fn flip_short_to_long_is_entry_for_the_excess() {
        let s = snap(Side::Sell, 4, Tri::Unknown);
        let c = classify(&intent(Side::Buy, 12, None), &s).unwrap();
        assert_eq!(parts(c), (Action::Entry, 8, 4));
    }

    #[test]
    fn stop_move_direction_long_and_short() {
        let long = snap(Side::Buy, 10, Tri::Known(d(490)));
        let up = classify(&intent(Side::Buy, 0, Some(d(495))), &long).unwrap();
        assert_eq!(up.action, Action::StopMove);
        assert_eq!(up.stop_direction, Some(StopDirection::Tighten));
        let down = classify(&intent(Side::Buy, 0, Some(d(480))), &long).unwrap();
        assert_eq!(down.stop_direction, Some(StopDirection::Widen));
        let same = classify(&intent(Side::Buy, 0, Some(d(490))), &long).unwrap();
        assert_eq!(same.stop_direction, Some(StopDirection::Unchanged));

        let short = snap(Side::Sell, 10, Tri::Known(d(510)));
        let t = classify(&intent(Side::Sell, 0, Some(d(505))), &short).unwrap();
        assert_eq!(t.stop_direction, Some(StopDirection::Tighten));
        let w = classify(&intent(Side::Sell, 0, Some(d(520))), &short).unwrap();
        assert_eq!(w.stop_direction, Some(StopDirection::Widen));
    }

    #[test]
    fn stop_move_without_stored_stop_is_unknown_direction() {
        let s = snap(Side::Buy, 10, Tri::Unknown);
        let c = classify(&intent(Side::Buy, 0, Some(d(495))), &s).unwrap();
        assert_eq!(c.stop_direction, Some(StopDirection::Unknown));
    }

    #[test]
    fn zero_qty_errors() {
        let s = snap(Side::Buy, 10, Tri::Unknown);
        assert_eq!(
            classify(&intent(Side::Buy, 0, None), &s),
            Err(ClassifyError::ZeroQty)
        );
        let none = PortfolioSnapshot::unknown();
        assert_eq!(
            classify(&intent(Side::Buy, 0, Some(d(495))), &none),
            Err(ClassifyError::StopMoveWithoutPosition)
        );
        assert!(ClassifyError::ZeroQty.to_string().contains("quantity is 0"));
        assert!(
            ClassifyError::StopMoveWithoutPosition
                .to_string()
                .contains("open position")
        );
    }

    #[test]
    fn stop_with_qty_is_an_error() {
        let s = snap(Side::Buy, 94, Tri::Known(d(490)));
        // Exit of 1 share that also widens the stop 490 -> 400 must not classify.
        let exit = intent(Side::Sell, 1, Some(d(400)));
        assert_eq!(classify(&exit, &s), Err(ClassifyError::StopWithQty));
        let entry = intent(Side::Buy, 5, Some(d(495)));
        assert_eq!(classify(&entry, &s), Err(ClassifyError::StopWithQty));
        assert!(ClassifyError::StopWithQty.to_string().contains("quantity"));
    }

    #[test]
    fn two_open_rows_for_a_symbol_are_an_entry() {
        let row = |side, qty| Position {
            symbol: "ABC".into(),
            side,
            qty,
            stop: Tri::Unknown,
            avg_cost: Tri::Unknown,
        };
        let mut s = PortfolioSnapshot::unknown();
        s.positions = Tri::Known(vec![row(Side::Buy, 10), row(Side::Sell, 20)]);
        let c = classify(&intent(Side::Sell, 10, None), &s).unwrap();
        assert_eq!(parts(c), (Action::Entry, 10, 0));
        let m = classify(&intent(Side::Buy, 0, Some(d(495))), &s);
        assert_eq!(m, Err(ClassifyError::StopMoveWithoutPosition));
    }

    #[test]
    fn caller_cannot_relabel_an_entry() {
        // The intent has no exit flag; a sell with no long held is a (short) entry.
        let mut s = PortfolioSnapshot::unknown();
        s.positions = Tri::Known(vec![]);
        assert_eq!(
            classify(&intent(Side::Sell, 10, None), &s).unwrap().action,
            Action::Entry
        );
    }
}
