//! Behaviour guards: G08 cool-off, G09 no stop widening, G10 entry zone, G11 no averaging down,
//! G17 re-entry lockout, G18 entries per day. All limits come from `RuleCtx` (effective limits);
//! all state comes from the snapshot. Unknown state rejects entries; exits and stop moves toward
//! safety are never blocked by the entry-only rules.

use crate::classify::{Action, Classification, StopDirection};
use crate::config::RuleCtx;
use crate::money_fmt::rupees;
use crate::snapshot::PortfolioSnapshot;
use crate::types::{OrderIntent, Side, Tri};
use crate::verdict::{RuleId, RuleResult};
use chrono::Duration;
use rust_decimal::Decimal;

/// Entry zone stored with the recommendation (F10 passes it from the stored recommendation, never
/// from the request).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryZone {
    pub low: Decimal,
    pub high: Decimal,
}

/// True when the order opens or adds exposure.
fn is_entry(class: &Classification) -> bool {
    class.action == Action::Entry && class.entry_qty > 0
}

fn not_an_entry(rule: RuleId) -> RuleResult {
    RuleResult::pass(rule, "exit or stop move", "entry-only rule")
}

fn unknown(rule: RuleId, what: &str) -> RuleResult {
    RuleResult::fail(
        rule,
        "unknown",
        "known value needed",
        &format!(
            "New entries are blocked because {what} cannot be confirmed; it must be readable from the journal"
        ),
    )
}

/// G08: after N consecutive losses in the session day, no new entries until the session ends.
#[must_use]
pub fn g08_cooloff(ctx: &RuleCtx, class: &Classification, snap: &PortfolioSnapshot) -> RuleResult {
    let rule = RuleId::G08;
    if !is_entry(class) {
        return not_an_entry(rule);
    }
    let Tri::Known(losses) = snap.consecutive_losses else {
        return unknown(rule, "today's run of losing trades");
    };
    let limit = ctx.limits.cooloff_after_losses;
    let (obs, lim) = (format!("{losses} losses in a row"), format!("{limit}"));
    if losses >= limit {
        RuleResult::fail(
            rule,
            obs,
            lim,
            &format!(
                "New entries are paused until the end of the session after {losses} losses in a row (limit {limit})"
            ),
        )
    } else {
        RuleResult::pass(rule, obs, format!("under {lim} losses in a row"))
    }
}

/// G09: a stop may only move toward profit. Unknown stored stop or unknown positions reject.
#[must_use]
pub fn g09_stop_widening(
    intent: &OrderIntent,
    class: &Classification,
    snap: &PortfolioSnapshot,
) -> RuleResult {
    let rule = RuleId::G09;
    if intent.new_stop.is_some() && snap.positions.is_unknown() {
        return unknown(rule, "your open positions");
    }
    if class.action != Action::StopMove {
        return RuleResult::pass(rule, "no stop change", "stop only moves toward profit");
    }
    match class.stop_direction {
        Some(StopDirection::Tighten) => RuleResult::pass(
            rule,
            "stop moved toward profit",
            "stop only moves toward profit",
        ),
        Some(StopDirection::Unchanged) => {
            RuleResult::pass(rule, "stop unchanged", "stop only moves toward profit")
        }
        Some(StopDirection::Widen) => RuleResult::fail(
            rule,
            "stop moved away from price",
            "toward profit only",
            "The stop can only move toward profit; moving it away raises the amount at risk",
        ),
        Some(StopDirection::Unknown) | None => RuleResult::fail(
            rule,
            "unknown",
            "stored stop needed",
            "The stop change is blocked because the stop stored for this position cannot be confirmed",
        ),
    }
}

/// G10: the live price must be inside the stored entry zone (widened by the tolerance in bps).
#[must_use]
pub fn g10_entry_zone(
    ctx: &RuleCtx,
    class: &Classification,
    zone: Tri<EntryZone>,
    live_price: Tri<Decimal>,
) -> RuleResult {
    let rule = RuleId::G10;
    if !is_entry(class) {
        return not_an_entry(rule);
    }
    let (Tri::Known(z), Tri::Known(price)) = (zone, live_price) else {
        return unknown(rule, "the entry zone or the live price");
    };
    let tol = Decimal::new(i64::from(ctx.limits.entry_zone_tolerance_bps), 4);
    let bounds = Decimal::ONE
        .checked_sub(tol)
        .and_then(|f| z.low.checked_mul(f))
        .zip(
            Decimal::ONE
                .checked_add(tol)
                .and_then(|f| z.high.checked_mul(f)),
        );
    let Some((low, high)) = bounds.filter(|(l, h)| z.low <= z.high && l <= h) else {
        return RuleResult::fail(
            rule,
            "invalid zone",
            "low not above high",
            "New entries are blocked because the stored entry zone is not usable",
        );
    };
    let limit = format!("{} to {}", rupees(low), rupees(high));
    let obs = rupees(price);
    if price > high {
        RuleResult::fail(
            rule,
            obs,
            limit,
            &format!(
                "Price {} is above the entry zone ({}); missed, re-scan instead of chasing",
                rupees(price),
                rupees(high)
            ),
        )
    } else if price < low {
        RuleResult::fail(
            rule,
            obs,
            limit,
            &format!(
                "Price {} is below the entry zone ({}); the setup has changed, re-scan",
                rupees(price),
                rupees(low)
            ),
        )
    } else {
        RuleResult::pass(rule, obs, limit)
    }
}

/// G11: no add to a position priced worse than its average cost (long: below, short: above).
#[must_use]
pub fn g11_averaging_down(
    intent: &OrderIntent,
    class: &Classification,
    snap: &PortfolioSnapshot,
) -> RuleResult {
    let rule = RuleId::G11;
    if !is_entry(class) {
        return not_an_entry(rule);
    }
    let Tri::Known(rows) = &snap.positions else {
        return unknown(rule, "your open positions");
    };
    let open: Vec<_> = rows
        .iter()
        .filter(|p| p.symbol == intent.symbol && p.qty > 0)
        .collect();
    let same_side = open.iter().filter(|p| p.side == intent.side).count();
    if open.len() > 1 {
        return unknown(rule, "which position this order adds to");
    }
    let Some(pos) = open.first().filter(|p| p.side == intent.side) else {
        return RuleResult::pass(rule, "no position to add to", "no averaging down");
    };
    let Tri::Known(avg) = pos.avg_cost else {
        return unknown(rule, "the average cost of the position");
    };
    let worse = match intent.side {
        Side::Buy => intent.price < avg,
        Side::Sell => intent.price > avg,
    };
    let (obs, lim) = (
        rupees(intent.price),
        format!("average cost {}", rupees(avg)),
    );
    if worse {
        RuleResult::fail(
            rule,
            obs,
            lim,
            &format!(
                "Adding at {} is worse than your average cost of {}; adding to a losing position is blocked",
                rupees(intent.price),
                rupees(avg)
            ),
        )
    } else {
        RuleResult::pass(rule, obs, lim)
    }
}

/// G17: after a stop-out in an instrument, no new entry in it for N minutes.
#[must_use]
pub fn g17_reentry_lockout(
    ctx: &RuleCtx,
    intent: &OrderIntent,
    class: &Classification,
    snap: &PortfolioSnapshot,
) -> RuleResult {
    let rule = RuleId::G17;
    if !is_entry(class) {
        return not_an_entry(rule);
    }
    let Tri::Known(outs) = &snap.last_stop_outs else {
        return unknown(rule, "your recent stop-outs");
    };
    let minutes = ctx.limits.reentry_lockout_minutes;
    let lock = Duration::minutes(i64::from(minutes));
    let blocked = outs
        .iter()
        .filter(|o| o.symbol == intent.symbol)
        .map(|o| ctx.now.signed_duration_since(o.at))
        // A stop-out time in the future means a clock or journal fault: treat as locked.
        .find(|elapsed| *elapsed < lock);
    match blocked {
        Some(elapsed) if elapsed >= Duration::zero() => RuleResult::fail(
            rule,
            format!("{} min since stop-out", elapsed.num_minutes()),
            format!("{minutes} min"),
            &format!(
                "{} was stopped out {} min ago; no new entry in it for {minutes} min",
                intent.symbol,
                elapsed.num_minutes()
            ),
        ),
        Some(_) => RuleResult::fail(
            rule,
            "stop-out time in the future",
            format!("{minutes} min"),
            "New entries are blocked because a recorded stop-out time is later than now",
        ),
        None => RuleResult::pass(rule, "no recent stop-out", format!("{minutes} min lockout")),
    }
}

/// G18: new entries per day are capped; exits are never blocked.
#[must_use]
pub fn g18_entries_per_day(
    ctx: &RuleCtx,
    class: &Classification,
    snap: &PortfolioSnapshot,
) -> RuleResult {
    let rule = RuleId::G18;
    if !is_entry(class) {
        return not_an_entry(rule);
    }
    let Tri::Known(done) = snap.entries_today else {
        return unknown(rule, "today's entry count");
    };
    let max = ctx.limits.max_new_entries_per_day;
    let (obs, lim) = (format!("{done} entries today"), format!("{max} a day"));
    if done >= max {
        RuleResult::fail(
            rule,
            obs,
            lim,
            &format!("You have already made {done} new entries today (limit {max} a day)"),
        )
    } else {
        RuleResult::pass(rule, obs, lim)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::classify;
    use crate::config::{Baseline, GateConfig};
    use crate::snapshot::{Position, StopOut};
    use bazaar_core::money::Qty;
    use chrono::{DateTime, TimeZone};
    use chrono_tz::{Asia::Kolkata, Tz};

    const TEXT: &str = include_str!("../../../../fixtures/config/risk.toml");

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn now() -> DateTime<Tz> {
        Kolkata.with_ymd_and_hms(2026, 1, 5, 11, 0, 0).unwrap()
    }

    fn ctx() -> RuleCtx {
        GateConfig::from_toml_str(TEXT)
            .unwrap()
            .rule_ctx(&Baseline::none(), now())
    }

    fn intent(side: Side, qty: u32, price: &str, new_stop: Option<&str>) -> OrderIntent {
        OrderIntent {
            symbol: "ABC".into(),
            side,
            qty: Qty(qty),
            price: d(price),
            stop: Some(d("490")),
            new_stop: new_stop.map(d),
            lot_size: 1,
            tick_size: d("0.05"),
        }
    }

    fn pos(side: Side, qty: u32, stop: Tri<Decimal>, avg: Tri<Decimal>) -> Position {
        Position {
            symbol: "ABC".into(),
            side,
            qty,
            stop,
            avg_cost: avg,
        }
    }

    fn snap_with(positions: Vec<Position>) -> PortfolioSnapshot {
        let mut s = PortfolioSnapshot::unknown();
        s.positions = Tri::Known(positions);
        s
    }

    fn cls(i: &OrderIntent, s: &PortfolioSnapshot) -> Classification {
        classify(i, s).unwrap()
    }

    fn entry(i: &OrderIntent, s: &PortfolioSnapshot) -> Classification {
        cls(i, s)
    }

    #[test]
    fn g08_boundaries_and_unknown() {
        let c = ctx();
        let i = intent(Side::Buy, 10, "500", None);
        let mut s = snap_with(vec![]);
        let kls = entry(&i, &s);
        let limit = c.limits.cooloff_after_losses;
        s.consecutive_losses = Tri::Known(limit - 1);
        assert!(g08_cooloff(&c, &kls, &s).passed());
        s.consecutive_losses = Tri::Known(limit);
        let res = g08_cooloff(&c, &kls, &s);
        assert!(!res.passed() && res.message().ends_with("(G08)"));
        s.consecutive_losses = Tri::Known(limit + 1);
        assert!(!g08_cooloff(&c, &kls, &s).passed());
        s.consecutive_losses = Tri::Unknown;
        assert!(!g08_cooloff(&c, &kls, &s).passed());
    }

    #[test]
    fn g08_never_blocks_exits_even_when_locked() {
        let c = ctx();
        let mut s = snap_with(vec![pos(Side::Buy, 10, Tri::Unknown, Tri::Unknown)]);
        s.consecutive_losses = Tri::Known(99);
        let i = intent(Side::Sell, 10, "500", None);
        assert!(g08_cooloff(&c, &cls(&i, &s), &s).passed());
    }

    #[test]
    fn g09_widen_unknown_and_tighten() {
        let p = |stop| snap_with(vec![pos(Side::Buy, 10, stop, Tri::Unknown)]);
        let s = p(Tri::Known(d("490")));
        let up = intent(Side::Buy, 0, "500", Some("495"));
        let down = intent(Side::Buy, 0, "500", Some("485"));
        let same = intent(Side::Buy, 0, "500", Some("490"));
        assert!(g09_stop_widening(&up, &cls(&up, &s), &s).passed());
        assert!(g09_stop_widening(&same, &cls(&same, &s), &s).passed());
        let res = g09_stop_widening(&down, &cls(&down, &s), &s);
        assert!(!res.passed() && res.message().ends_with("(G09)"));
        let s2 = p(Tri::Unknown);
        assert!(!g09_stop_widening(&up, &cls(&up, &s2), &s2).passed());
    }

    #[test]
    fn g09_short_position_direction_and_unknown_positions() {
        let s = snap_with(vec![pos(
            Side::Sell,
            10,
            Tri::Known(d("510")),
            Tri::Unknown,
        )]);
        let widen = intent(Side::Buy, 0, "500", Some("520"));
        let tighten = intent(Side::Buy, 0, "500", Some("505"));
        assert!(!g09_stop_widening(&widen, &cls(&widen, &s), &s).passed());
        assert!(g09_stop_widening(&tighten, &cls(&tighten, &s), &s).passed());
        // Unknown positions with a stop change reject even with a hand-made classification.
        let fake = Classification {
            action: Action::Entry,
            entry_qty: 0,
            exit_qty: 0,
            stop_direction: None,
        };
        let res = g09_stop_widening(&tighten, &fake, &PortfolioSnapshot::unknown());
        assert!(!res.passed());
    }

    #[test]
    fn g09_ignores_orders_without_a_stop_change() {
        let i = intent(Side::Buy, 10, "500", None);
        let s = snap_with(vec![]);
        assert!(g09_stop_widening(&i, &cls(&i, &s), &s).passed());
    }

    fn zone() -> Tri<EntryZone> {
        Tri::Known(EntryZone {
            low: d("498"),
            high: d("502"),
        })
    }

    #[test]
    fn g10_inside_above_below_and_unknown() {
        let c = ctx();
        let i = intent(Side::Buy, 10, "500", None);
        let kls = entry(&i, &snap_with(vec![]));
        let tol = c.limits.entry_zone_tolerance_bps;
        assert_eq!(tol, 5, "fixture tolerance is 5 bps");
        for (p, ok) in [
            ("500", true),
            ("498", true),
            ("502", true),
            ("502.25", true),
            ("502.26", false),
            ("497.76", true),
            ("497.75", false),
        ] {
            assert_eq!(
                g10_entry_zone(&c, &kls, zone(), Tri::Known(d(p))).passed(),
                ok,
                "{p}"
            );
        }
        assert!(!g10_entry_zone(&c, &kls, Tri::Unknown, Tri::Known(d("500"))).passed());
        assert!(!g10_entry_zone(&c, &kls, zone(), Tri::Unknown).passed());
        let bad = Tri::Known(EntryZone {
            low: d("510"),
            high: d("500"),
        });
        assert!(!g10_entry_zone(&c, &kls, bad, Tri::Known(d("505"))).passed());
        let res = g10_entry_zone(&c, &kls, zone(), Tri::Known(d("510")));
        assert!(res.message().contains("missed") && res.message().ends_with("(G10)"));
    }

    #[test]
    fn g10_skips_exits() {
        let c = ctx();
        let s = snap_with(vec![pos(Side::Buy, 10, Tri::Unknown, Tri::Unknown)]);
        let i = intent(Side::Sell, 10, "500", None);
        assert!(g10_entry_zone(&c, &cls(&i, &s), Tri::Unknown, Tri::Unknown).passed());
    }

    #[test]
    fn g11_long_short_flip_and_unknowns() {
        let held = |side, avg| snap_with(vec![pos(side, 10, Tri::Unknown, avg)]);
        let avg = Tri::Known(d("500"));
        let buy = |p| intent(Side::Buy, 5, p, None);
        let s = held(Side::Buy, avg);
        assert!(g11_averaging_down(&buy("500"), &cls(&buy("500"), &s), &s).passed());
        assert!(g11_averaging_down(&buy("501"), &cls(&buy("501"), &s), &s).passed());
        let res = g11_averaging_down(&buy("499.95"), &cls(&buy("499.95"), &s), &s);
        assert!(!res.passed() && res.message().ends_with("(G11)"));
        let s = held(Side::Sell, avg);
        let sell = |p| intent(Side::Sell, 5, p, None);
        assert!(!g11_averaging_down(&sell("500.05"), &cls(&sell("500.05"), &s), &s).passed());
        assert!(g11_averaging_down(&sell("499"), &cls(&sell("499"), &s), &s).passed());
        // Unknown average cost or unknown positions reject an add.
        let s = held(Side::Buy, Tri::Unknown);
        assert!(!g11_averaging_down(&buy("510"), &cls(&buy("510"), &s), &s).passed());
        let u = PortfolioSnapshot::unknown();
        assert!(!g11_averaging_down(&buy("510"), &cls(&buy("510"), &u), &u).passed());
        // A flip's excess opens a new position: not an add.
        let s = held(Side::Sell, avg);
        let flip = intent(Side::Buy, 15, "400", None);
        assert!(g11_averaging_down(&flip, &cls(&flip, &s), &s).passed());
        // A fresh position has nothing to average.
        let s = snap_with(vec![]);
        assert!(g11_averaging_down(&buy("1"), &cls(&buy("1"), &s), &s).passed());
    }

    #[test]
    fn g11_two_rows_reject_and_exits_pass() {
        let rows = vec![
            pos(Side::Buy, 10, Tri::Unknown, Tri::Known(d("500"))),
            pos(Side::Sell, 5, Tri::Unknown, Tri::Known(d("500"))),
        ];
        let s = snap_with(rows);
        let i = intent(Side::Buy, 5, "600", None);
        assert!(!g11_averaging_down(&i, &cls(&i, &s), &s).passed());
        let s = snap_with(vec![pos(Side::Buy, 10, Tri::Unknown, Tri::Known(d("500")))]);
        let exit = intent(Side::Sell, 10, "400", None);
        assert!(g11_averaging_down(&exit, &cls(&exit, &s), &s).passed());
    }

    fn stop_out(symbol: &str, mins_ago: i64) -> StopOut {
        StopOut {
            symbol: symbol.into(),
            at: now() - Duration::minutes(mins_ago),
        }
    }

    #[test]
    fn g17_lockout_boundary_per_instrument() {
        let c = ctx();
        let lock = i64::from(c.limits.reentry_lockout_minutes);
        let i = intent(Side::Buy, 10, "500", None);
        let mut s = snap_with(vec![]);
        let kls = entry(&i, &s);
        s.last_stop_outs = Tri::Known(vec![stop_out("ABC", lock - 1)]);
        let res = g17_reentry_lockout(&c, &i, &kls, &s);
        assert!(!res.passed() && res.message().ends_with("(G17)"));
        s.last_stop_outs = Tri::Known(vec![stop_out("ABC", lock)]);
        assert!(g17_reentry_lockout(&c, &i, &kls, &s).passed());
        s.last_stop_outs = Tri::Known(vec![stop_out("XYZ", 1)]);
        assert!(g17_reentry_lockout(&c, &i, &kls, &s).passed());
        s.last_stop_outs = Tri::Known(vec![]);
        assert!(g17_reentry_lockout(&c, &i, &kls, &s).passed());
        s.last_stop_outs = Tri::Unknown;
        assert!(!g17_reentry_lockout(&c, &i, &kls, &s).passed());
    }

    #[test]
    fn g17_future_stop_out_rejects_and_exit_passes() {
        let c = ctx();
        let i = intent(Side::Buy, 10, "500", None);
        let mut s = snap_with(vec![]);
        let kls = entry(&i, &s);
        s.last_stop_outs = Tri::Known(vec![stop_out("ABC", -5)]);
        assert!(!g17_reentry_lockout(&c, &i, &kls, &s).passed());
        let mut s = snap_with(vec![pos(Side::Buy, 10, Tri::Unknown, Tri::Unknown)]);
        s.last_stop_outs = Tri::Known(vec![stop_out("ABC", 1)]);
        let exit = intent(Side::Sell, 10, "500", None);
        assert!(g17_reentry_lockout(&c, &exit, &cls(&exit, &s), &s).passed());
    }

    #[test]
    fn g17_uses_the_stricter_baseline() {
        let mut b = Baseline::none();
        b.reentry_lockout_minutes = Some(120);
        let c = GateConfig::from_toml_str(TEXT).unwrap().rule_ctx(&b, now());
        let i = intent(Side::Buy, 10, "500", None);
        let mut s = snap_with(vec![]);
        let kls = entry(&i, &s);
        s.last_stop_outs = Tri::Known(vec![stop_out("ABC", 100)]);
        assert!(!g17_reentry_lockout(&c, &i, &kls, &s).passed());
    }

    #[test]
    fn g18_fourth_entry_blocked_exits_never() {
        let c = ctx();
        let max = c.limits.max_new_entries_per_day;
        let i = intent(Side::Buy, 10, "500", None);
        let mut s = snap_with(vec![]);
        let kls = entry(&i, &s);
        s.entries_today = Tri::Known(max - 1);
        assert!(g18_entries_per_day(&c, &kls, &s).passed());
        s.entries_today = Tri::Known(max);
        let res = g18_entries_per_day(&c, &kls, &s);
        assert!(!res.passed() && res.message().ends_with("(G18)"));
        s.entries_today = Tri::Unknown;
        assert!(!g18_entries_per_day(&c, &kls, &s).passed());
        let mut s = snap_with(vec![pos(Side::Buy, 10, Tri::Unknown, Tri::Unknown)]);
        s.entries_today = Tri::Known(max + 5);
        let exit = intent(Side::Sell, 10, "500", None);
        assert!(g18_entries_per_day(&c, &cls(&exit, &s), &s).passed());
    }

    #[test]
    fn rule_levels_are_read_from_the_table() {
        let i = intent(Side::Buy, 10, "500", None);
        let s = snap_with(vec![]);
        let res = g18_entries_per_day(&ctx(), &entry(&i, &s), &s);
        assert_eq!(res.level(), crate::verdict::OverrideLevel::No);
    }
}
