//! Fill model (F9 policy): next-open entries, gap-aware stops, targets, time exits.
//!
//! Every function sees only the bar being filled (and order levels fixed on earlier closed
//! bars), so no accessor can reach past it. Money is `Decimal`; slippage is adverse and rounded
//! to the instrument tick (buys up, sells down); tick and lot size are inputs from the
//! instrument master, never constants. Charges are not computed here (cost model, slice 1): the
//! produced `Fill` carries zero charges for the engine to fill in.

use bazaar_core::candle::Candle;
use bazaar_core::money::{Price, Qty};
use chrono::{DateTime, NaiveDate};
use chrono_tz::Tz;
use rust_decimal::{Decimal, RoundingStrategy};

use crate::config::{Fills, Slippage};
use crate::types::{BacktestError, ChargeBreakdown, Fill, FillKind, Leg, LegSide};

const HUNDRED: Decimal = Decimal::from_parts(100, 0, 0, false, 0);

/// Which way an open position points; a long exits by selling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Long,
    Short,
}

impl Direction {
    const fn entry_side(self) -> LegSide {
        match self {
            Self::Long => LegSide::Buy,
            Self::Short => LegSide::Sell,
        }
    }
    const fn exit_side(self) -> LegSide {
        match self {
            Self::Long => LegSide::Sell,
            Self::Short => LegSide::Buy,
        }
    }
}

/// Inputs for one instrument: tick from the instrument master as of the bar date plus the
/// configured slippage class and target rule.
#[derive(Debug, Clone, Copy)]
pub struct FillParams<'a> {
    pub tick: Decimal,
    pub slippage: &'a Slippage,
    pub fills: &'a Fills,
}

/// A simulated execution price, before it is attached to a leg.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimFill {
    pub kind: FillKind,
    pub side: LegSide,
    /// Price before slippage (open, stop or target), snapped to tick.
    pub reference: Price,
    /// Price after adverse slippage, tick-rounded adversely.
    pub price: Price,
    /// Rupees per unit, `>= 0`.
    pub slippage_per_unit: Decimal,
}

impl SimFill {
    /// Attach to a leg template (`price` and `side` are overwritten); charges stay zero until
    /// the cost model prices the leg.
    #[must_use]
    pub fn into_fill(self, mut leg: Leg, bar_time: DateTime<Tz>) -> Fill {
        leg.price = self.price;
        leg.side = self.side;
        Fill {
            leg,
            kind: self.kind,
            bar_time,
            reference_price: self.reference,
            slippage_per_unit: self.slippage_per_unit,
            charges: ChargeBreakdown::default(),
        }
    }
}

fn ov(what: &'static str) -> BacktestError {
    BacktestError::Overflow(what)
}

fn check_tick(tick: Decimal) -> Result<(), BacktestError> {
    if tick <= Decimal::ZERO {
        return Err(BacktestError::Invalid("tick must be positive".into()));
    }
    Ok(())
}

/// Round to a multiple of `tick` with a directed `strategy`.
fn to_tick(
    x: Decimal,
    tick: Decimal,
    strategy: RoundingStrategy,
) -> Result<Decimal, BacktestError> {
    x.checked_div(tick)
        .ok_or_else(|| ov("tick rounding"))?
        .round_dp_with_strategy(0, strategy)
        .checked_mul(tick)
        .ok_or_else(|| ov("tick rounding"))
}

/// Snap `x` to the nearest tick (a reference price is data, not an order).
fn reference(x: Decimal, tick: Decimal) -> Result<Price, BacktestError> {
    Price::snap_to_tick(x, tick).map_err(|e| BacktestError::Invalid(e.to_string()))
}

/// Slippage in rupees per unit: `max(price x pct / 100, min_ticks x tick)`, rounded up to tick.
fn slip_amount(
    reference: Decimal,
    pct: Decimal,
    min_ticks: u32,
    tick: Decimal,
) -> Result<Decimal, BacktestError> {
    let pct_amt = reference
        .checked_mul(pct)
        .and_then(|v| v.checked_div(HUNDRED))
        .ok_or_else(|| ov("slippage"))?;
    let floor_amt = tick
        .checked_mul(Decimal::from(min_ticks))
        .ok_or_else(|| ov("slippage"))?;
    to_tick(
        pct_amt.max(floor_amt),
        tick,
        RoundingStrategy::ToPositiveInfinity,
    )
}

/// Apply adverse slippage: a buy pays more (rounded up), a sell receives less (rounded down,
/// never below zero).
fn slipped(
    kind: FillKind,
    side: LegSide,
    reference_px: Price,
    pct: Decimal,
    min_ticks: u32,
    tick: Decimal,
) -> Result<SimFill, BacktestError> {
    let r = reference_px.value();
    let slip = slip_amount(r, pct, min_ticks, tick)?;
    let (raw, strategy) = match side {
        LegSide::Buy => (
            r.checked_add(slip).ok_or_else(|| ov("fill price"))?,
            RoundingStrategy::ToPositiveInfinity,
        ),
        LegSide::Sell => (
            r.checked_sub(slip)
                .ok_or_else(|| ov("fill price"))?
                .max(Decimal::ZERO),
            RoundingStrategy::ToNegativeInfinity,
        ),
    };
    let px = reference(to_tick(raw, tick, strategy)?, tick)?;
    let applied = px
        .value()
        .checked_sub(r)
        .ok_or_else(|| ov("slippage"))?
        .abs();
    Ok(SimFill {
        kind,
        side,
        reference: reference_px,
        price: px,
        slippage_per_unit: applied,
    })
}

/// Entry: the signal closed on the previous bar; fill at `next_bar` open plus entry slippage.
///
/// # Errors
/// `Invalid` for a non-positive tick, `Overflow` on arithmetic overflow.
pub fn fill_entry(
    next_bar: &Candle,
    dir: Direction,
    p: &FillParams<'_>,
) -> Result<SimFill, BacktestError> {
    check_tick(p.tick)?;
    let open = reference(next_bar.open(), p.tick)?;
    slipped(
        FillKind::Entry,
        dir.entry_side(),
        open,
        p.slippage.entry_pct,
        p.slippage.entry_min_ticks,
        p.tick,
    )
}

/// Time stop or end of data: exit at `bar` open plus entry-class slippage.
///
/// # Errors
/// As [`fill_entry`].
pub fn fill_time_exit(
    bar: &Candle,
    dir: Direction,
    p: &FillParams<'_>,
) -> Result<SimFill, BacktestError> {
    check_tick(p.tick)?;
    let open = reference(bar.open(), p.tick)?;
    slipped(
        FillKind::TimeExit,
        dir.exit_side(),
        open,
        p.slippage.entry_pct,
        p.slippage.entry_min_ticks,
        p.tick,
    )
}

/// Check the stop and target of an open position against `bar` (the levels were fixed on closed
/// data before it). Returns the exit if one happens.
///
/// - Stop (long: `low <= stop`, short: `high >= stop`) fills at `min(stop, open)` long or
///   `max(stop, open)` short (a gap through the stop fills at the open), then stop slippage.
/// - Target fills at the target price only if the bar trades through it by
///   `fills.target_through_ticks`; a gap beyond the target earns no credit, and the target takes
///   entry-class slippage (`config/backtest.toml` comment). The stop wins a same-bar tie, since
///   the order of high and low inside a bar is unknowable.
///
/// # Errors
/// As [`fill_entry`].
pub fn check_exit(
    bar: &Candle,
    dir: Direction,
    stop: Decimal,
    target: Option<Decimal>,
    p: &FillParams<'_>,
) -> Result<Option<SimFill>, BacktestError> {
    check_tick(p.tick)?;
    let (open, high, low) = (bar.open(), bar.high(), bar.low());
    let stop_hit = match dir {
        Direction::Long => low <= stop,
        Direction::Short => high >= stop,
    };
    if stop_hit {
        let raw = match dir {
            Direction::Long => stop.min(open),
            Direction::Short => stop.max(open),
        };
        return slipped(
            FillKind::Stop,
            dir.exit_side(),
            reference(raw, p.tick)?,
            p.slippage.stop_pct,
            p.slippage.stop_min_ticks,
            p.tick,
        )
        .map(Some);
    }
    let Some(t) = target else { return Ok(None) };
    let through = p
        .tick
        .checked_mul(Decimal::from(p.fills.target_through_ticks))
        .ok_or_else(|| ov("target through"))?;
    let hit = match dir {
        Direction::Long => high >= t.checked_add(through).ok_or_else(|| ov("target"))?,
        Direction::Short => low <= t.checked_sub(through).ok_or_else(|| ov("target"))?,
    };
    if !hit {
        return Ok(None);
    }
    slipped(
        FillKind::Target,
        dir.exit_side(),
        reference(t, p.tick)?,
        p.slippage.entry_pct,
        p.slippage.entry_min_ticks,
        p.tick,
    )
    .map(Some)
}

/// Floor a unit quantity to whole lots (`lot_size` from the instrument master as of the date).
///
/// # Errors
/// `Invalid` for a zero lot size.
pub fn floor_to_lots(units: u32, lot_size: u32) -> Result<Qty, BacktestError> {
    if lot_size == 0 {
        return Err(BacktestError::Invalid("lot size must be positive".into()));
    }
    Ok(Qty(units / lot_size * lot_size))
}

/// IST trade date of a bar time (the bar time is already `Asia/Kolkata`).
#[must_use]
pub fn trade_date(bar_time: DateTime<Tz>) -> NaiveDate {
    bar_time.date_naive()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bazaar_core::candle::Interval;
    use bazaar_core::segment::Segment;
    use chrono::TimeZone;
    use chrono_tz::Asia::Kolkata;

    use crate::types::{LegInstrument, Product};

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn bar(open: &str, high: &str, low: &str, close: &str) -> Candle {
        let time = Kolkata.with_ymd_and_hms(2026, 3, 2, 15, 30, 0).unwrap();
        Candle::new(
            time,
            Interval::Day,
            [d(open), d(high), d(low), d(close)],
            1,
            false,
        )
        .unwrap()
    }

    fn eq_slip() -> Slippage {
        Slippage {
            source: "t".into(),
            unverified: true,
            entry_pct: d("0.05"),
            entry_min_ticks: 1,
            stop_pct: d("0.10"),
            stop_min_ticks: 1,
        }
    }

    fn fills() -> Fills {
        Fills {
            source: "t".into(),
            unverified: true,
            target_through_ticks: 1,
            stress_slippage_multiplier: d("2"),
        }
    }

    fn run<T>(f: impl FnOnce(&FillParams<'_>) -> T) -> T {
        let (s, fl) = (eq_slip(), fills());
        f(&FillParams {
            tick: d("0.05"),
            slippage: &s,
            fills: &fl,
        })
    }

    #[test]
    fn long_entry_fills_next_open_plus_slippage_rounded_up() {
        // open 1000.00: 0.05% = 0.50 -> 1000.50
        let f =
            run(|p| fill_entry(&bar("1000", "1010", "990", "1005"), Direction::Long, p)).unwrap();
        assert_eq!(f.kind, FillKind::Entry);
        assert_eq!(f.side, LegSide::Buy);
        assert_eq!(f.price.value(), d("1000.50"));
        assert_eq!(f.slippage_per_unit, d("0.50"));
    }

    #[test]
    fn min_ticks_floor_applies_on_cheap_price() {
        // open 10.00: 0.05% = 0.005 < one tick 0.05 -> 10.05
        let f = run(|p| fill_entry(&bar("10", "11", "9", "10"), Direction::Long, p)).unwrap();
        assert_eq!(f.price.value(), d("10.05"));
    }

    #[test]
    fn slippage_rounds_up_to_tick() {
        // open 1001: 0.05% = 0.5005 -> ceil to 0.55 -> 1001.55
        let f =
            run(|p| fill_entry(&bar("1001", "1010", "990", "1005"), Direction::Long, p)).unwrap();
        assert_eq!(f.price.value(), d("1001.55"));
    }

    #[test]
    fn short_entry_sells_below_open() {
        let f =
            run(|p| fill_entry(&bar("1000", "1010", "990", "1005"), Direction::Short, p)).unwrap();
        assert_eq!(f.side, LegSide::Sell);
        assert_eq!(f.price.value(), d("999.50"));
    }

    #[test]
    fn long_stop_normal_fill_at_stop_minus_stop_slippage() {
        // stop 980 touched, open 1000 above it: ref 980, 0.10% = 0.98 -> ceil 1.00 -> 979.00
        let f = run(|p| {
            check_exit(
                &bar("1000", "1010", "975", "985"),
                Direction::Long,
                d("980"),
                None,
                p,
            )
        })
        .unwrap()
        .unwrap();
        assert_eq!(f.kind, FillKind::Stop);
        assert_eq!(f.reference.value(), d("980"));
        assert_eq!(f.price.value(), d("979.00"));
        assert_eq!(f.slippage_per_unit, d("1.00"));
    }

    #[test]
    fn gap_down_through_long_stop_fills_at_open_not_stop() {
        // stop 980, bar opens 950: ref = min(980, 950) = 950; 0.10% = 0.95 -> 949.05
        let f = run(|p| {
            check_exit(
                &bar("950", "960", "940", "955"),
                Direction::Long,
                d("980"),
                None,
                p,
            )
        })
        .unwrap()
        .unwrap();
        assert_eq!(f.reference.value(), d("950"));
        assert_eq!(f.price.value(), d("949.05"));
    }

    #[test]
    fn short_stop_gap_up_fills_at_open_plus_slippage() {
        // short stop 1020, opens 1050: ref 1050, 0.10% = 1.05 -> 1051.05
        let f = run(|p| {
            check_exit(
                &bar("1050", "1060", "1045", "1055"),
                Direction::Short,
                d("1020"),
                None,
                p,
            )
        })
        .unwrap()
        .unwrap();
        assert_eq!(f.side, LegSide::Buy);
        assert_eq!(f.reference.value(), d("1050"));
        assert_eq!(f.price.value(), d("1051.05"));
    }

    #[test]
    fn short_stop_normal_fill_uses_stop_when_open_below() {
        // ref = max(1020, 1000) = 1020; 1.02 -> ceil 1.05 -> 1021.05
        let f = run(|p| {
            check_exit(
                &bar("1000", "1025", "995", "1010"),
                Direction::Short,
                d("1020"),
                None,
                p,
            )
        })
        .unwrap()
        .unwrap();
        assert_eq!(f.price.value(), d("1021.05"));
    }

    #[test]
    fn same_bar_stop_and_target_tie_stop_wins() {
        let f = run(|p| {
            check_exit(
                &bar("1000", "1100", "970", "1050"),
                Direction::Long,
                d("980"),
                Some(d("1050")),
                p,
            )
        })
        .unwrap()
        .unwrap();
        assert_eq!(f.kind, FillKind::Stop);
        assert_eq!(f.price.value(), d("979.00"));
    }

    #[test]
    fn target_fills_at_target_only_when_traded_through() {
        // target 1050 needs high >= 1050.05; high 1050 -> no fill
        let none = run(|p| {
            check_exit(
                &bar("1000", "1050", "990", "1040"),
                Direction::Long,
                d("980"),
                Some(d("1050")),
                p,
            )
        })
        .unwrap();
        assert!(none.is_none());
        // high 1050.05 -> fills at 1050 less entry-class slippage 0.525 -> ceil 0.55 -> 1049.45
        let f = run(|p| {
            check_exit(
                &bar("1000", "1050.05", "990", "1040"),
                Direction::Long,
                d("980"),
                Some(d("1050")),
                p,
            )
        })
        .unwrap()
        .unwrap();
        assert_eq!(f.kind, FillKind::Target);
        assert_eq!(f.reference.value(), d("1050"));
        assert_eq!(f.price.value(), d("1049.45"));
    }

    #[test]
    fn target_gap_up_gets_no_credit() {
        // opens 1080 above target 1050: still the target price, not the open
        let f = run(|p| {
            check_exit(
                &bar("1080", "1090", "1075", "1085"),
                Direction::Long,
                d("980"),
                Some(d("1050")),
                p,
            )
        })
        .unwrap()
        .unwrap();
        assert_eq!(f.reference.value(), d("1050"));
        assert_eq!(f.price.value(), d("1049.45"));
    }

    #[test]
    fn short_target_needs_low_through_it() {
        let f = run(|p| {
            check_exit(
                &bar("1000", "1010", "949.95", "960"),
                Direction::Short,
                d("1020"),
                Some(d("950")),
                p,
            )
        })
        .unwrap()
        .unwrap();
        assert_eq!(f.kind, FillKind::Target);
        assert_eq!(f.side, LegSide::Buy);
        // 950 + 0.475 -> ceil 0.50 = 950.50
        assert_eq!(f.price.value(), d("950.50"));
    }

    #[test]
    fn no_exit_inside_the_range() {
        let r = run(|p| {
            check_exit(
                &bar("1000", "1040", "990", "1020"),
                Direction::Long,
                d("980"),
                Some(d("1050")),
                p,
            )
        })
        .unwrap();
        assert!(r.is_none());
    }

    #[test]
    fn time_exit_sells_at_open_minus_entry_slippage() {
        let f = run(|p| fill_time_exit(&bar("1000", "1010", "990", "1005"), Direction::Long, p))
            .unwrap();
        assert_eq!(f.kind, FillKind::TimeExit);
        assert_eq!(f.side, LegSide::Sell);
        assert_eq!(f.price.value(), d("999.50"));
    }

    #[test]
    fn sell_price_never_goes_below_zero() {
        let s = Slippage {
            entry_min_ticks: 1000,
            ..eq_slip()
        };
        let fl = fills();
        let p = FillParams {
            tick: d("0.05"),
            slippage: &s,
            fills: &fl,
        };
        let f = fill_time_exit(&bar("1", "2", "1", "1"), Direction::Long, &p).unwrap();
        assert_eq!(f.price.value(), Decimal::ZERO);
        assert_eq!(f.slippage_per_unit, d("1"));
    }

    #[test]
    fn non_positive_tick_is_an_error() {
        let (s, fl) = (eq_slip(), fills());
        let p = FillParams {
            tick: Decimal::ZERO,
            slippage: &s,
            fills: &fl,
        };
        let b = bar("10", "11", "9", "10");
        assert!(fill_entry(&b, Direction::Long, &p).is_err());
        assert!(fill_time_exit(&b, Direction::Long, &p).is_err());
        assert!(check_exit(&b, Direction::Long, d("9"), None, &p).is_err());
    }

    #[test]
    fn lots_floor_and_zero_lot_errors() {
        assert_eq!(floor_to_lots(130, 50).unwrap(), Qty(100));
        assert_eq!(floor_to_lots(49, 50).unwrap(), Qty(0));
        assert!(floor_to_lots(10, 0).is_err());
    }

    #[test]
    fn into_fill_carries_price_side_and_zero_charges() {
        let sim =
            run(|p| fill_entry(&bar("1000", "1010", "990", "1005"), Direction::Long, p)).unwrap();
        let leg = Leg {
            instrument_token: 1,
            symbol: "TCS".into(),
            segment: Segment::Equity,
            instrument: LegInstrument::Cash,
            product: Product::Delivery,
            side: LegSide::Sell,
            qty: Qty(3),
            price: sim.reference,
            trade_date: trade_date(bar("1", "1", "1", "1").close_time()),
        };
        let t = bar("1", "1", "1", "1").close_time();
        let f = sim.into_fill(leg, t);
        assert_eq!(f.leg.price, sim.price);
        assert_eq!(f.leg.side, LegSide::Buy);
        assert_eq!(f.charges, ChargeBreakdown::default());
        assert_eq!(
            f.leg.trade_date,
            NaiveDate::from_ymd_opt(2026, 3, 2).unwrap()
        );
    }
}
