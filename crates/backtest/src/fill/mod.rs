//! Fill model (F9 policy): next-open entries, gap-aware stops, targets, time exits.
//!
//! Every function sees only the bar being filled (and order levels fixed on earlier closed
//! bars), so no accessor can reach past it. Money is `Decimal`; slippage is adverse and rounded
//! to the instrument tick (buys up, sells down); tick and lot size are inputs from the
//! instrument master, never constants. Charges are not computed here (cost model, slice 1): the
//! produced `Fill` carries zero charges for the engine to fill in. Slice 4 (engine) owns a test
//! that no `Fill` reaches P&L with the default breakdown (QUAN-84-6) and the AC4 `lookahead_`
//! property and peeking-double tests; here every function takes only the bar it fills.

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
    /// Scales the percent and the min-ticks floor before tick rounding (1 normally; the 2x
    /// stress run passes `fills.stress_slippage_multiplier`).
    pub slippage_multiplier: Decimal,
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

/// An order level (stop or target) rounded against the trader: `up` rounds up, else down.
fn level(x: Decimal, tick: Decimal, up: bool) -> Result<Price, BacktestError> {
    let strategy = if up {
        RoundingStrategy::ToPositiveInfinity
    } else {
        RoundingStrategy::ToNegativeInfinity
    };
    reference(to_tick(x, tick, strategy)?, tick)
}

/// Slippage in rupees per unit: `max(price x pct / 100, min_ticks x tick)`, rounded up to tick.
pub(crate) fn slip_amount(
    reference: Decimal,
    pct: Decimal,
    min_ticks: u32,
    tick: Decimal,
    mult: Decimal,
) -> Result<Decimal, BacktestError> {
    let pct_amt = reference
        .checked_mul(pct)
        .and_then(|v| v.checked_mul(mult))
        .and_then(|v| v.checked_div(HUNDRED))
        .ok_or_else(|| ov("slippage"))?;
    let floor_amt = tick
        .checked_mul(Decimal::from(min_ticks))
        .and_then(|v| v.checked_mul(mult))
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
    mult: Decimal,
) -> Result<SimFill, BacktestError> {
    let r = reference_px.value();
    let slip = slip_amount(r, pct, min_ticks, tick, mult)?;
    let (raw, strategy) = match side {
        LegSide::Buy => (
            r.checked_add(slip).ok_or_else(|| ov("fill price"))?,
            RoundingStrategy::ToPositiveInfinity,
        ),
        LegSide::Sell => {
            let raw = r.checked_sub(slip).ok_or_else(|| ov("fill price"))?;
            if raw < Decimal::ZERO {
                return Err(BacktestError::Invalid(
                    "slippage exceeds the sell price".into(),
                ));
            }
            (raw, RoundingStrategy::ToNegativeInfinity)
        }
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
/// `zone_limit` is the entry-zone edge (long: zone top, short: zone bottom; policy section 2,
/// tolerance 0): an open beyond it means no fill and the signal expires, never a chase
/// (`Ok(None)`). Limit-style entries belong to the engine slice (slice 4).
///
/// # Errors
/// `Invalid` for a non-positive tick, `Overflow` on arithmetic overflow.
pub fn fill_entry(
    next_bar: &Candle,
    dir: Direction,
    zone_limit: Option<Decimal>,
    p: &FillParams<'_>,
) -> Result<Option<SimFill>, BacktestError> {
    check_tick(p.tick)?;
    let raw_open = next_bar.open();
    if let Some(z) = zone_limit {
        let beyond = match dir {
            Direction::Long => raw_open > z,
            Direction::Short => raw_open < z,
        };
        if beyond {
            return Ok(None);
        }
    }
    let open = reference(raw_open, p.tick)?;
    slipped(
        FillKind::Entry,
        dir.entry_side(),
        open,
        p.slippage.entry_pct,
        p.slippage.entry_min_ticks,
        p.tick,
        p.slippage_multiplier,
    )
    .map(Some)
}

/// Time stop or end of data: exit at `bar` open plus entry-class slippage (F9 policy section 1:
/// one "entry / target / time-exit" column; only stops use the stop column).
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
        p.slippage_multiplier,
    )
}

/// Check the stop and target of an open position against `bar` (the levels were fixed on closed
/// data before it). Returns the exit if one happens. `entry` is the slipped entry fill price,
/// used only to validate level sides.
///
/// - Stop (long: `low <= stop`, short: `high >= stop`) fills at `min(stop, open)` long or
///   `max(stop, open)` short (a gap through the stop fills at the open), then stop slippage.
/// - Target fills at the target price only if the bar trades through it by
///   `fills.target_through_ticks`; a gap beyond the target earns no credit. The target takes
///   entry-class slippage on purpose (F9 policy section 1, "entry / target / time-exit"; also
///   the `config/backtest.toml` comment): do not "fix" it to zero. Entry-class slippage and no
///   gap credit together are a deliberately conservative choice (decided, CR-84-1).
/// - The stop wins a same-bar tie, since the order of high and low inside a bar is unknowable.
/// - Stop and target are order levels: rounded against the trader (long stop and short target
///   down, long target and short stop up), never to the nearest tick.
///
/// # Errors
/// `Invalid` when the stop is not strictly on the loss side of `entry` (long: below, short:
/// above) or the target not on the profit side, or for a non-positive tick; `Overflow`.
pub fn check_exit(
    bar: &Candle,
    dir: Direction,
    entry: Price,
    stop: Decimal,
    target: Option<Decimal>,
    p: &FillParams<'_>,
) -> Result<Option<SimFill>, BacktestError> {
    check_tick(p.tick)?;
    let e = entry.value();
    let long = dir == Direction::Long;
    let (stop_ok, target_ok) = if long {
        (stop < e, target.is_none_or(|t| t > e))
    } else {
        (stop > e, target.is_none_or(|t| t < e))
    };
    if !stop_ok {
        return Err(BacktestError::Invalid(
            "stop is on the wrong side of the entry".into(),
        ));
    }
    if !target_ok {
        return Err(BacktestError::Invalid(
            "target is on the wrong side of the entry".into(),
        ));
    }
    let (open, high, low) = (bar.open(), bar.high(), bar.low());
    // Stop rounds against the trader: a long sells lower, a short buys higher.
    let stop_lvl = level(stop, p.tick, !long)?.value();
    let stop_hit = if long {
        low <= stop_lvl
    } else {
        high >= stop_lvl
    };
    if stop_hit {
        let raw = if long {
            stop_lvl.min(open)
        } else {
            stop_lvl.max(open)
        };
        return slipped(
            FillKind::Stop,
            dir.exit_side(),
            reference(raw, p.tick)?,
            p.slippage.stop_pct,
            p.slippage.stop_min_ticks,
            p.tick,
            p.slippage_multiplier,
        )
        .map(Some);
    }
    let Some(t) = target else { return Ok(None) };
    // Target rounds against the trader: a long target is further up, a short one further down.
    let target_lvl = level(t, p.tick, long)?;
    let through = p
        .tick
        .checked_mul(Decimal::from(p.fills.target_through_ticks))
        .ok_or_else(|| ov("target through"))?;
    let hit = if long {
        high >= target_lvl
            .value()
            .checked_add(through)
            .ok_or_else(|| ov("target"))?
    } else {
        low <= target_lvl
            .value()
            .checked_sub(through)
            .ok_or_else(|| ov("target"))?
    };
    if !hit {
        return Ok(None);
    }
    slipped(
        FillKind::Target,
        dir.exit_side(),
        target_lvl,
        p.slippage.entry_pct,
        p.slippage.entry_min_ticks,
        p.tick,
        p.slippage_multiplier,
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

    fn px(s: &str) -> Price {
        Price::snap_to_tick(d(s), d("0.05")).unwrap()
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

    fn with<T>(slip: &Slippage, mult: &str, f: impl FnOnce(&FillParams<'_>) -> T) -> T {
        let fl = fills();
        f(&FillParams {
            tick: d("0.05"),
            slippage: slip,
            fills: &fl,
            slippage_multiplier: d(mult),
        })
    }

    fn run<T>(f: impl FnOnce(&FillParams<'_>) -> T) -> T {
        with(&eq_slip(), "1", f)
    }

    fn entry(b: &Candle, dir: Direction) -> SimFill {
        run(|p| fill_entry(b, dir, None, p)).unwrap().unwrap()
    }

    fn exit_long(b: &Candle, stop: &str, target: Option<&str>) -> Option<SimFill> {
        run(|p| check_exit(b, Direction::Long, px("1000"), d(stop), target.map(d), p)).unwrap()
    }

    fn exit_short(b: &Candle, stop: &str, target: Option<&str>) -> Option<SimFill> {
        run(|p| check_exit(b, Direction::Short, px("1000"), d(stop), target.map(d), p)).unwrap()
    }

    #[test]
    fn long_entry_fills_next_open_plus_slippage_rounded_up() {
        // open 1000.00: 0.05% = 0.50 -> 1000.50
        let f = entry(&bar("1000", "1010", "990", "1005"), Direction::Long);
        assert_eq!(f.kind, FillKind::Entry);
        assert_eq!(f.side, LegSide::Buy);
        assert_eq!(f.price.value(), d("1000.50"));
        assert_eq!(f.slippage_per_unit, d("0.50"));
    }

    #[test]
    fn min_ticks_floor_applies_on_cheap_price() {
        // open 10.00: 0.05% = 0.005 < one tick 0.05 -> 10.05
        let f = entry(&bar("10", "11", "9", "10"), Direction::Long);
        assert_eq!(f.price.value(), d("10.05"));
    }

    #[test]
    fn slippage_rounds_up_to_tick() {
        // open 1001: 0.05% = 0.5005 -> ceil to 0.55 -> 1001.55
        let f = entry(&bar("1001", "1010", "990", "1005"), Direction::Long);
        assert_eq!(f.price.value(), d("1001.55"));
    }

    #[test]
    fn short_entry_sells_below_open() {
        let f = entry(&bar("1000", "1010", "990", "1005"), Direction::Short);
        assert_eq!(f.side, LegSide::Sell);
        assert_eq!(f.price.value(), d("999.50"));
    }

    #[test]
    fn off_tick_open_snaps_to_nearest_tick_as_data() {
        // 1000.02 -> 1000.00 (data), then +0.50
        let f = entry(&bar("1000.02", "1010", "990", "1005"), Direction::Long);
        assert_eq!(f.reference.value(), d("1000.00"));
        assert_eq!(f.price.value(), d("1000.50"));
    }

    #[test]
    fn open_beyond_zone_expires_the_signal_no_chase() {
        let b = bar("1000.05", "1010", "990", "1005");
        let none = run(|p| fill_entry(&b, Direction::Long, Some(d("1000")), p)).unwrap();
        assert!(none.is_none());
        // exactly at the zone top still fills (tolerance 0)
        let at = bar("1000", "1010", "990", "1005");
        let some = run(|p| fill_entry(&at, Direction::Long, Some(d("1000")), p)).unwrap();
        assert!(some.is_some());
        // short: below the zone bottom expires
        let low = bar("999.95", "1010", "990", "1005");
        let s = run(|p| fill_entry(&low, Direction::Short, Some(d("1000")), p)).unwrap();
        assert!(s.is_none());
    }

    #[test]
    fn stress_multiplier_scales_percent_and_min_ticks_floor() {
        // 2x: 0.05% x 2 = 1.00 on 1000 -> 1001.00; on 10: floor 1 tick x 2 = 0.10 -> 10.10
        let b1 = bar("1000", "1010", "990", "1005");
        let hi = with(&eq_slip(), "2", |p| {
            fill_entry(&b1, Direction::Long, None, p)
        })
        .unwrap()
        .unwrap();
        assert_eq!(hi.price.value(), d("1001.00"));
        let b2 = bar("10", "11", "9", "10");
        let lo = with(&eq_slip(), "2", |p| {
            fill_entry(&b2, Direction::Long, None, p)
        })
        .unwrap()
        .unwrap();
        assert_eq!(lo.price.value(), d("10.10"));
    }

    #[test]
    fn index_futures_class_uses_min_ticks_only() {
        let s = Slippage {
            entry_pct: d("0"),
            stop_pct: d("0"),
            stop_min_ticks: 2,
            ..eq_slip()
        };
        let b = bar("20000", "20100", "19900", "20050");
        let e = with(&s, "1", |p| fill_entry(&b, Direction::Long, None, p))
            .unwrap()
            .unwrap();
        assert_eq!(e.price.value(), d("20000.05"));
        let x = with(&s, "1", |p| {
            check_exit(&b, Direction::Long, px("20000"), d("19950"), None, p)
        })
        .unwrap()
        .unwrap();
        assert_eq!(x.price.value(), d("19949.90"));
    }

    #[test]
    fn index_options_class_uses_premium_percent() {
        let s = Slippage {
            entry_pct: d("0.5"),
            entry_min_ticks: 2,
            stop_pct: d("1"),
            stop_min_ticks: 3,
            ..eq_slip()
        };
        let b = bar("100", "110", "90", "105");
        let e = with(&s, "1", |p| fill_entry(&b, Direction::Long, None, p))
            .unwrap()
            .unwrap();
        assert_eq!(e.price.value(), d("100.50"));
        // stop 80 on premium: 1% = 0.80 > 3 ticks 0.15 -> 79.20
        let b2 = bar("95", "96", "79", "85");
        let x = with(&s, "1", |p| {
            check_exit(&b2, Direction::Long, px("100"), d("80"), None, p)
        })
        .unwrap()
        .unwrap();
        assert_eq!(x.price.value(), d("79.20"));
    }

    #[test]
    fn long_stop_normal_fill_at_stop_minus_stop_slippage() {
        // ref 980, 0.10% = 0.98 -> ceil 1.00 -> 979.00
        let f = exit_long(&bar("1000", "1010", "975", "985"), "980", None).unwrap();
        assert_eq!(f.kind, FillKind::Stop);
        assert_eq!(f.reference.value(), d("980"));
        assert_eq!(f.price.value(), d("979.00"));
        assert_eq!(f.slippage_per_unit, d("1.00"));
    }

    #[test]
    fn stop_min_ticks_floor_on_cheap_price() {
        // stop 10.00 on a 10.50 entry; 0.10% of 10 = 0.01 < 1 tick -> 9.95
        let b = bar("10.20", "10.30", "9.90", "10");
        let f = run(|p| check_exit(&b, Direction::Long, px("10.50"), d("10"), None, p))
            .unwrap()
            .unwrap();
        assert_eq!(f.price.value(), d("9.95"));
    }

    #[test]
    fn gap_down_through_long_stop_fills_at_open_not_stop() {
        // stop 980, opens 950: ref 950; 0.10% = 0.95 -> 949.05
        let f = exit_long(&bar("950", "960", "940", "955"), "980", None).unwrap();
        assert_eq!(f.reference.value(), d("950"));
        assert_eq!(f.price.value(), d("949.05"));
    }

    #[test]
    fn stop_boundaries_are_inclusive() {
        assert!(exit_long(&bar("1000", "1010", "980", "985"), "980", None).is_some());
        assert!(exit_long(&bar("1000", "1010", "980.05", "985"), "980", None).is_none());
        assert!(exit_short(&bar("1000", "1020", "990", "1010"), "1020", None).is_some());
        assert!(exit_short(&bar("1000", "1019.95", "990", "1010"), "1020", None).is_none());
    }

    #[test]
    fn short_stop_gap_up_fills_at_open_plus_slippage() {
        // ref 1050, 0.10% = 1.05 -> 1051.05
        let f = exit_short(&bar("1050", "1060", "1045", "1055"), "1020", None).unwrap();
        assert_eq!(f.side, LegSide::Buy);
        assert_eq!(f.reference.value(), d("1050"));
        assert_eq!(f.price.value(), d("1051.05"));
    }

    #[test]
    fn short_stop_normal_fill_uses_stop_when_open_below() {
        // ref 1020; 1.02 -> ceil 1.05 -> 1021.05
        let f = exit_short(&bar("1000", "1025", "995", "1010"), "1020", None).unwrap();
        assert_eq!(f.price.value(), d("1021.05"));
    }

    #[test]
    fn same_bar_stop_and_target_tie_stop_wins() {
        let f = exit_long(&bar("1000", "1100", "970", "1050"), "980", Some("1050")).unwrap();
        assert_eq!(f.kind, FillKind::Stop);
        assert_eq!(f.price.value(), d("979.00"));
    }

    #[test]
    fn target_fills_at_target_only_when_traded_through() {
        // target 1050 needs high >= 1050.05; high 1050 -> no fill
        assert!(exit_long(&bar("1000", "1050", "990", "1040"), "980", Some("1050")).is_none());
        // high 1050.05 -> 1050 less 0.525 -> ceil 0.55 -> 1049.45
        let f = exit_long(&bar("1000", "1050.05", "990", "1040"), "980", Some("1050")).unwrap();
        assert_eq!(f.kind, FillKind::Target);
        assert_eq!(f.reference.value(), d("1050"));
        assert_eq!(f.price.value(), d("1049.45"));
    }

    #[test]
    fn target_gap_up_gets_no_credit() {
        let f = exit_long(&bar("1080", "1090", "1075", "1085"), "980", Some("1050")).unwrap();
        assert_eq!(f.reference.value(), d("1050"));
        assert_eq!(f.price.value(), d("1049.45"));
    }

    #[test]
    fn short_target_needs_low_through_it() {
        let f = exit_short(&bar("1000", "1010", "949.95", "960"), "1020", Some("950")).unwrap();
        assert_eq!(f.kind, FillKind::Target);
        assert_eq!(f.side, LegSide::Buy);
        // 950 + 0.475 -> ceil 0.50 = 950.50
        assert_eq!(f.price.value(), d("950.50"));
    }

    #[test]
    fn off_tick_levels_round_against_the_trader() {
        // long stop 980.03 -> 980.00 (down): fills 979.00, not 979.05
        let s = exit_long(&bar("1000", "1010", "975", "985"), "980.03", None).unwrap();
        assert_eq!(s.reference.value(), d("980.00"));
        assert_eq!(s.price.value(), d("979.00"));
        // long target 1049.97 -> 1050.00 (up): high 1050.00 is not enough, 1050.05 is
        assert!(exit_long(&bar("1000", "1050", "990", "1040"), "980", Some("1049.97")).is_none());
        let t = exit_long(
            &bar("1000", "1050.05", "990", "1040"),
            "980",
            Some("1049.97"),
        )
        .unwrap();
        assert_eq!(t.reference.value(), d("1050.00"));
        // short stop 1020.02 -> 1020.05 (up): high 1020.00 no hit, 1020.05 hits -> 1021.10
        assert!(exit_short(&bar("1000", "1020", "990", "1010"), "1020.02", None).is_none());
        let ss = exit_short(&bar("1000", "1020.05", "990", "1010"), "1020.02", None).unwrap();
        assert_eq!(ss.reference.value(), d("1020.05"));
        assert_eq!(ss.price.value(), d("1021.10"));
        // short target 950.02 -> 950.00 (down)
        let st = exit_short(
            &bar("1000", "1010", "949.95", "960"),
            "1020",
            Some("950.02"),
        )
        .unwrap();
        assert_eq!(st.reference.value(), d("950.00"));
    }

    #[test]
    fn stop_and_target_on_the_wrong_side_are_errors() {
        let b = bar("1000", "1010", "990", "1005");
        let long = |stop: &str, target: Option<&str>| {
            run(|p| check_exit(&b, Direction::Long, px("1000"), d(stop), target.map(d), p))
        };
        assert!(long("1000", None).is_err());
        assert!(long("1010", None).is_err());
        assert!(long("980", Some("990")).is_err());
        let short = |stop: &str, target: Option<&str>| {
            run(|p| check_exit(&b, Direction::Short, px("1000"), d(stop), target.map(d), p))
        };
        assert!(short("1000", None).is_err());
        assert!(short("1020", Some("1010")).is_err());
    }

    #[test]
    fn no_exit_inside_the_range() {
        assert!(exit_long(&bar("1000", "1040", "990", "1020"), "980", Some("1050")).is_none());
    }

    #[test]
    fn fill_prices_respect_bounds_over_a_grid() {
        // long stop fill never above min(stop, open); long target fill never above the target
        for open in (900..=1000).step_by(5) {
            let o = open.to_string();
            let f = exit_long(&bar(&o, "1000", "850", "900"), "980", None).unwrap();
            assert!(f.price.value() <= d("980").min(Decimal::from(open)));
            assert!(f.price.value() < f.reference.value());
        }
        for high in (1051..=1100).step_by(7) {
            let h = high.to_string();
            let f = exit_long(&bar("1000", &h, "990", "1040"), "980", Some("1050")).unwrap();
            assert!(f.price.value() <= d("1050"));
        }
    }

    #[test]
    fn time_exit_sells_at_open_minus_entry_slippage() {
        let b = bar("1000", "1010", "990", "1005");
        let f = run(|p| fill_time_exit(&b, Direction::Long, p)).unwrap();
        assert_eq!(f.kind, FillKind::TimeExit);
        assert_eq!(f.side, LegSide::Sell);
        assert_eq!(f.price.value(), d("999.50"));
    }

    #[test]
    fn sell_slippage_beyond_the_price_is_an_error() {
        let s = Slippage {
            entry_min_ticks: 1000,
            ..eq_slip()
        };
        let b = bar("1", "2", "1", "1");
        let r = with(&s, "1", |p| fill_time_exit(&b, Direction::Long, p));
        assert!(matches!(r, Err(BacktestError::Invalid(_))));
    }

    #[test]
    fn non_positive_tick_is_an_error() {
        let (s, fl) = (eq_slip(), fills());
        let p = FillParams {
            tick: Decimal::ZERO,
            slippage: &s,
            fills: &fl,
            slippage_multiplier: d("1"),
        };
        let b = bar("10", "11", "9", "10");
        assert!(fill_entry(&b, Direction::Long, None, &p).is_err());
        assert!(fill_time_exit(&b, Direction::Long, &p).is_err());
        assert!(check_exit(&b, Direction::Long, px("10"), d("9"), None, &p).is_err());
    }

    #[test]
    fn lots_floor_and_zero_lot_errors() {
        assert_eq!(floor_to_lots(130, 50).unwrap(), Qty(100));
        assert_eq!(floor_to_lots(49, 50).unwrap(), Qty(0));
        assert!(floor_to_lots(10, 0).is_err());
    }

    #[test]
    fn into_fill_carries_price_side_and_zero_charges() {
        let sim = entry(&bar("1000", "1010", "990", "1005"), Direction::Long);
        let t = bar("1", "1", "1", "1").close_time();
        let leg = Leg {
            instrument_token: 1,
            symbol: "TCS".into(),
            segment: Segment::Equity,
            instrument: LegInstrument::Cash,
            product: Product::Delivery,
            side: LegSide::Sell,
            qty: Qty(3),
            price: sim.reference,
            trade_date: trade_date(t),
        };
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
