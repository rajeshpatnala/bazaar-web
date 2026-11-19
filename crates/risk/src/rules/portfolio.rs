//! Portfolio rules: G02 (risk per trade), G04a (open positions), G04b (total open risk) and
//! G14 (position value cap). Pure checks: thresholds come from `RuleCtx.limits`, equity from
//! `sizing_equity` (the figure `size` uses), and the entry is valued at `ref_price`, the worst
//! case `max(intent.price, live price)` computed once by the gate and never re-derived here.
//!
//! Entry-only: exits and stop moves pass. Any unknown input (positions, stops, marks, equity,
//! charges buffer) rejects an entry. Comparisons are exact `Decimal` (cross-multiplied, no
//! rounding); an overflow rejects. A same-side add is judged on the combined position
//! (RISK-49-5, RISK-50-3); a flip's closing part leaves the position out and only the new
//! excess counts.

use crate::classify::{Action, Classification};
use crate::config::RuleCtx;
use crate::money_fmt::{pct, rupees};
use crate::sizing::sizing_equity;
use crate::snapshot::{PortfolioSnapshot, Position};
use crate::types::{OrderIntent, Side, Tri};
use crate::verdict::{RuleId, RuleResult};
use rust_decimal::Decimal;

const OVERFLOW: &str = "the figures overflowed";

fn unsure(rule: RuleId, limit: &str, why: &str) -> RuleResult {
    RuleResult::fail(
        rule,
        "cannot confirm",
        limit,
        &format!("Cannot confirm this check: {why}. When unsure, the app says no."),
    )
}

/// `amount x 100 > pct x equity` is over the cap. `None` on overflow.
fn over_cap(amount: Decimal, cap_pct: Decimal, equity: Decimal) -> Option<bool> {
    Some(amount.checked_mul(Decimal::ONE_HUNDRED)? > cap_pct.checked_mul(equity)?)
}

/// Rupee value of a cap, for the message only.
fn cap_amount(cap_pct: Decimal, equity: Decimal) -> Decimal {
    cap_pct
        .checked_mul(equity)
        .and_then(|v| v.checked_div(Decimal::ONE_HUNDRED))
        .unwrap_or(Decimal::ZERO)
}

/// True when the order opens exposure; otherwise the rule has nothing to check.
fn is_entry(cls: &Classification) -> bool {
    cls.action == Action::Entry && cls.entry_qty > 0
}

/// Positions that stay open after this order: a flip closes the same-symbol opposite position.
fn surviving<'a>(
    positions: &'a [Position],
    intent: &'a OrderIntent,
) -> impl Iterator<Item = &'a Position> {
    positions
        .iter()
        .filter(move |p| p.qty > 0 && (p.symbol != intent.symbol || p.side == intent.side))
}

/// Rupees at risk from `from` to `stop` for `qty` (a stop in profit counts as 0).
fn risk_of(side: Side, qty: u32, from: Decimal, stop: Decimal) -> Option<Decimal> {
    let per = match side {
        Side::Buy => from.checked_sub(stop)?,
        Side::Sell => stop.checked_sub(from)?,
    };
    per.max(Decimal::ZERO).checked_mul(Decimal::from(qty))
}

/// Risk of the new quantity from the reference price to the intent's own stop.
fn new_risk(
    intent: &OrderIntent,
    cls: &Classification,
    ref_price: Decimal,
) -> Result<Decimal, String> {
    let stop = intent.stop.ok_or("the order has no stop")?;
    let wrong_side = match intent.side {
        Side::Buy => stop >= ref_price,
        Side::Sell => stop <= ref_price,
    };
    if ref_price <= Decimal::ZERO || wrong_side {
        return Err("the stop is not on the loss side of the price".into());
    }
    risk_of(intent.side, cls.entry_qty, ref_price, stop).ok_or_else(|| OVERFLOW.into())
}

/// Risk of one held position from its stored risk price to its stored stop.
fn held_risk(p: &Position) -> Result<Decimal, String> {
    let (Tri::Known(from), Tri::Known(stop)) = (p.risk_price, p.stop) else {
        return Err(format!("{} has no stored stop or risk price", p.symbol));
    };
    risk_of(p.side, p.qty, from, stop).ok_or_else(|| OVERFLOW.into())
}

/// Pass or fail a cap check on `total`; `what` completes "This ... would risk/be worth".
fn verdict(
    rule: RuleId,
    total: Decimal,
    cap_pct: Decimal,
    equity: Decimal,
    what: &str,
) -> RuleResult {
    let limit = pct(cap_pct);
    match over_cap(total, cap_pct, equity) {
        Some(false) => RuleResult::pass(rule, rupees(total), limit),
        Some(true) => RuleResult::fail(
            rule,
            rupees(total),
            &limit,
            &format!(
                "{} {}, above your {} cap ({limit} of your account).",
                what,
                rupees(total),
                rupees(cap_amount(cap_pct, equity))
            ),
        ),
        None => unsure(rule, &limit, OVERFLOW),
    }
}

/// G02: risk to the stop plus the charges allowance stays within `max_risk_per_trade_pct` of
/// equity. A same-side add is judged on the combined position: the held risk (stored stop) plus
/// the new quantity's own stop, so a wider new stop raises the total (RISK-49-5).
#[must_use]
pub fn g02(
    ctx: &RuleCtx,
    intent: &OrderIntent,
    cls: &Classification,
    snap: &PortfolioSnapshot,
    ref_price: Decimal,
) -> RuleResult {
    let rule = RuleId::G02;
    let l = &ctx.limits;
    let limit = pct(l.max_risk_per_trade_pct);
    if !is_entry(cls) {
        return RuleResult::pass(rule, "no new exposure", limit);
    }
    let (Some(positions), Ok(equity), Some(buffer)) = (
        snap.positions.known(),
        sizing_equity(snap, l),
        snap.charges_buffer.known(),
    ) else {
        return unsure(
            rule,
            &limit,
            "positions, account value or the charges allowance is unknown",
        );
    };
    let total = new_risk(intent, cls, ref_price).and_then(|new| {
        let mut sum = new
            .checked_add((*buffer).max(l.min_charges_slippage_buffer))
            .ok_or(OVERFLOW)?;
        let same = |p: &&Position| p.qty > 0 && p.symbol == intent.symbol && p.side == intent.side;
        for p in positions.iter().filter(same) {
            sum = sum.checked_add(held_risk(p)?).ok_or(OVERFLOW)?;
        }
        Ok(sum)
    });
    match total {
        Ok(t) => verdict(
            rule,
            t,
            l.max_risk_per_trade_pct,
            equity,
            "This trade risks",
        ),
        Err(why) => unsure(rule, &limit, &why),
    }
}

/// G04a: the number of open symbols after this order stays within `max_open_positions`.
/// Adding to a symbol already held does not open another position.
#[must_use]
pub fn g04a(
    ctx: &RuleCtx,
    intent: &OrderIntent,
    cls: &Classification,
    snap: &PortfolioSnapshot,
) -> RuleResult {
    let rule = RuleId::G04a;
    let max = ctx.limits.max_open_positions;
    let limit = max.to_string();
    if !is_entry(cls) {
        return RuleResult::pass(rule, "no new exposure", limit);
    }
    let Some(positions) = snap.positions.known() else {
        return unsure(rule, &limit, "open positions are unknown");
    };
    let mut symbols: Vec<&str> = surviving(positions, intent)
        .map(|p| p.symbol.as_str())
        .collect();
    symbols.push(&intent.symbol);
    symbols.sort_unstable();
    symbols.dedup();
    let after = u32::try_from(symbols.len()).unwrap_or(u32::MAX);
    if after <= max {
        RuleResult::pass(rule, after.to_string(), limit)
    } else {
        RuleResult::fail(
            rule,
            after.to_string(),
            &limit,
            &format!(
                "This would make {after} open positions; your limit is {max}. Close one first."
            ),
        )
    }
}

/// G04b: risk of all open positions plus the new quantity stays within
/// `max_total_open_risk_pct` of equity. Charges are not counted here (G02 owns them).
#[must_use]
pub fn g04b(
    ctx: &RuleCtx,
    intent: &OrderIntent,
    cls: &Classification,
    snap: &PortfolioSnapshot,
    ref_price: Decimal,
) -> RuleResult {
    let rule = RuleId::G04b;
    let l = &ctx.limits;
    let limit = pct(l.max_total_open_risk_pct);
    if !is_entry(cls) {
        return RuleResult::pass(rule, "no new exposure", limit);
    }
    let (Some(positions), Ok(equity)) = (snap.positions.known(), sizing_equity(snap, l)) else {
        return unsure(rule, &limit, "positions or account value are unknown");
    };
    let total = new_risk(intent, cls, ref_price).and_then(|new| {
        let mut sum = new;
        for p in surviving(positions, intent) {
            sum = sum.checked_add(held_risk(p)?).ok_or(OVERFLOW)?;
        }
        Ok(sum)
    });
    match total {
        Ok(t) => verdict(
            rule,
            t,
            l.max_total_open_risk_pct,
            equity,
            "All your open trades together would risk",
        ),
        Err(why) => unsure(rule, &limit, &why),
    }
}

/// G14: value of the position after this order (a same-side holding at its mark plus the new
/// quantity at `ref_price`) stays within `max_position_value_pct` of equity (RISK-50-3).
#[must_use]
pub fn g14(
    ctx: &RuleCtx,
    intent: &OrderIntent,
    cls: &Classification,
    snap: &PortfolioSnapshot,
    ref_price: Decimal,
) -> RuleResult {
    let rule = RuleId::G14;
    let l = &ctx.limits;
    let limit = pct(l.max_position_value_pct);
    if !is_entry(cls) {
        return RuleResult::pass(rule, "no new exposure", limit);
    }
    let (Some(positions), Some(marks), Ok(equity)) = (
        snap.positions.known(),
        snap.marks.known(),
        sizing_equity(snap, l),
    ) else {
        return unsure(
            rule,
            &limit,
            "positions, marks or account value are unknown",
        );
    };
    if ref_price <= Decimal::ZERO {
        return unsure(rule, &limit, "the reference price is not positive");
    }
    let mut value = ref_price.checked_mul(Decimal::from(cls.entry_qty));
    let same = |p: &&Position| p.qty > 0 && p.symbol == intent.symbol && p.side == intent.side;
    for p in positions.iter().filter(same) {
        let Some(mark) = marks.iter().find(|m| m.symbol == p.symbol) else {
            return unsure(rule, &limit, &format!("{} has no mark price", p.symbol));
        };
        value = value
            .zip(mark.price.checked_mul(Decimal::from(p.qty)))
            .and_then(|(a, b)| a.checked_add(b));
    }
    match value {
        Some(v) => verdict(
            rule,
            v,
            l.max_position_value_pct,
            equity,
            "This position would be worth",
        ),
        None => unsure(rule, &limit, OVERFLOW),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::classify;
    use crate::config::{Baseline, GateConfig};
    use crate::snapshot::Mark;
    use bazaar_core::money::Qty;
    use chrono::TimeZone;
    use chrono_tz::Asia::Kolkata;

    const TEXT: &str = include_str!("../../../../fixtures/config/risk.toml");

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn ctx() -> RuleCtx {
        let now = Kolkata.with_ymd_and_hms(2026, 11, 19, 11, 0, 0).unwrap();
        GateConfig::from_toml_str(TEXT)
            .unwrap()
            .rule_ctx(&Baseline::none(), now)
    }

    fn intent(sym: &str, side: Side, qty: u32, stop: Option<&str>) -> OrderIntent {
        OrderIntent {
            symbol: sym.into(),
            side,
            qty: Qty(qty),
            price: d("500"),
            stop: stop.map(d),
            new_stop: None,
            lot_size: 1,
            tick_size: d("0.05"),
        }
    }

    fn pos(sym: &str, side: Side, qty: u32, stop: Tri<Decimal>) -> Position {
        Position {
            symbol: sym.into(),
            side,
            qty,
            stop,
            risk_price: Tri::Known(d("500")),
            avg_cost: Tri::Known(d("500")),
        }
    }

    fn snap(positions: Vec<Position>) -> PortfolioSnapshot {
        let mut s = PortfolioSnapshot::unknown();
        s.marks = Tri::Known(
            positions
                .iter()
                .map(|p| Mark {
                    symbol: p.symbol.clone(),
                    price: d("500"),
                })
                .collect(),
        );
        s.positions = Tri::Known(positions);
        s.charges_buffer = Tri::Known(d("15"));
        s.starting_cash = Tri::Known(Decimal::from(100_000));
        s.realised_pnl = Tri::Known(Decimal::ZERO);
        s.equity_mtm = Tri::Known(Decimal::from(100_000));
        s
    }

    fn cls(i: &OrderIntent, s: &PortfolioSnapshot) -> Classification {
        classify(i, s).unwrap()
    }

    fn msg_ok(r: &RuleResult, id: &str) -> bool {
        !r.passed() && r.message().ends_with(&format!("({id})"))
    }

    // Fixture: risk 0.5% (Rs 500), open risk 2% (Rs 2,000), 2 positions, value 20% (Rs 20,000).

    #[test]
    fn g02_boundaries_include_charges() {
        let c = ctx();
        let s = snap(vec![]);
        // Rs 15 buffer: qty 97 x 5 = 485 + 15 = 500 passes, 98 x 5 = 490 + 15 = 505 fails.
        for (qty, ok) in [(96, true), (97, true), (98, false)] {
            let i = intent("ABC", Side::Buy, qty, Some("495"));
            let r = g02(&c, &i, &cls(&i, &s), &s, d("500"));
            assert_eq!(r.passed(), ok, "qty {qty}");
        }
        let i = intent("ABC", Side::Buy, 98, Some("495"));
        let r = g02(&c, &i, &cls(&i, &s), &s, d("500"));
        assert!(msg_ok(&r, "G02") && r.message().contains("Rs 505"));
    }

    #[test]
    fn g02_uses_worst_case_reference_price() {
        let c = ctx();
        let s = snap(vec![]);
        let i = intent("ABC", Side::Buy, 97, Some("495"));
        let r = g02(&c, &i, &cls(&i, &s), &s, d("501"));
        assert!(!r.passed());
    }

    #[test]
    fn g02_add_with_wider_stop_raises_combined_risk() {
        let c = ctx();
        let s = snap(vec![pos("ABC", Side::Buy, 50, Tri::Known(d("495")))]);
        // Held risk 250; new 20 x 10 = 200; +15 = 465 passes. New stop 480 => 400+250+15 fails.
        let ok = intent("ABC", Side::Buy, 20, Some("490"));
        assert!(g02(&c, &ok, &cls(&ok, &s), &s, d("500")).passed());
        let wide = intent("ABC", Side::Buy, 20, Some("480"));
        assert!(msg_ok(
            &g02(&c, &wide, &cls(&wide, &s), &s, d("500")),
            "G02"
        ));
    }

    #[test]
    fn g02_rejects_unknowns_and_bad_stops() {
        let c = ctx();
        let i = intent("ABC", Side::Buy, 10, Some("495"));
        let unknown = PortfolioSnapshot::unknown();
        assert!(!g02(&c, &i, &cls(&i, &unknown), &unknown, d("500")).passed());
        let mut s = snap(vec![]);
        s.charges_buffer = Tri::Unknown;
        assert!(!g02(&c, &i, &cls(&i, &s), &s, d("500")).passed());
        let s = snap(vec![pos("ABC", Side::Buy, 5, Tri::Unknown)]);
        assert!(!g02(&c, &i, &cls(&i, &s), &s, d("500")).passed());
        let s = snap(vec![]);
        for stop in [None, Some("505")] {
            let b = intent("ABC", Side::Buy, 10, stop);
            assert!(!g02(&c, &b, &cls(&b, &s), &s, d("500")).passed());
        }
    }

    #[test]
    fn g02_short_entry_and_exit_pass() {
        let c = ctx();
        let s = snap(vec![]);
        let short = intent("ABC", Side::Sell, 50, Some("505"));
        assert!(g02(&c, &short, &cls(&short, &s), &s, d("500")).passed());
        let s = snap(vec![pos("ABC", Side::Buy, 50, Tri::Unknown)]);
        let exit = intent("ABC", Side::Sell, 50, None);
        assert!(g02(&c, &exit, &cls(&exit, &s), &s, d("500")).passed());
    }

    #[test]
    fn g04a_boundaries_and_adds() {
        let c = ctx();
        let one = snap(vec![pos("AAA", Side::Buy, 1, Tri::Known(d("490")))]);
        let two = snap(vec![
            pos("AAA", Side::Buy, 1, Tri::Known(d("490"))),
            pos("BBB", Side::Buy, 1, Tri::Known(d("490"))),
        ]);
        let new = intent("CCC", Side::Buy, 1, Some("490"));
        assert!(g04a(&c, &new, &cls(&new, &one), &one).passed());
        let r = g04a(&c, &new, &cls(&new, &two), &two);
        assert!(msg_ok(&r, "G04a") && r.message().contains("3 open"));
        let add = intent("AAA", Side::Buy, 1, Some("490"));
        assert!(g04a(&c, &add, &cls(&add, &two), &two).passed());
        let flip = intent("AAA", Side::Sell, 5, Some("510"));
        assert!(g04a(&c, &flip, &cls(&flip, &two), &two).passed());
        let unknown = PortfolioSnapshot::unknown();
        assert!(!g04a(&c, &new, &cls(&new, &unknown), &unknown).passed());
    }

    #[test]
    fn g04b_boundaries_and_unknown_stop() {
        let c = ctx();
        // Held 100 x (500-490) = 1,000; new 100 x 10 = 1,000 => exactly 2,000 passes.
        let s = snap(vec![pos("AAA", Side::Buy, 100, Tri::Known(d("490")))]);
        for (qty, ok) in [(99, true), (100, true), (101, false)] {
            let i = intent("BBB", Side::Buy, qty, Some("490"));
            let r = g04b(&c, &i, &cls(&i, &s), &s, d("500"));
            assert_eq!(r.passed(), ok, "qty {qty}");
            assert!(ok || msg_ok(&r, "G04b"));
        }
        let i = intent("BBB", Side::Buy, 1, Some("490"));
        let bad = snap(vec![pos("AAA", Side::Buy, 1, Tri::Unknown)]);
        assert!(!g04b(&c, &i, &cls(&i, &bad), &bad, d("500")).passed());
    }

    #[test]
    fn g04b_flip_drops_closed_position_and_stop_in_profit_is_zero() {
        let c = ctx();
        let s = snap(vec![pos("AAA", Side::Buy, 400, Tri::Known(d("400")))]);
        let flip = intent("AAA", Side::Sell, 410, Some("510"));
        // Closed long drops out; new 10 x 10 = 100 passes.
        assert!(g04b(&c, &flip, &cls(&flip, &s), &s, d("500")).passed());
        let profit = snap(vec![pos("AAA", Side::Buy, 400, Tri::Known(d("520")))]);
        let i = intent("BBB", Side::Buy, 100, Some("490"));
        assert!(g04b(&c, &i, &cls(&i, &profit), &profit, d("500")).passed());
    }

    #[test]
    fn g14_boundaries_and_combined_position() {
        let c = ctx();
        let s = snap(vec![]);
        for (qty, ok) in [(39, true), (40, true), (41, false)] {
            let i = intent("ABC", Side::Buy, qty, Some("490"));
            let r = g14(&c, &i, &cls(&i, &s), &s, d("500"));
            assert_eq!(r.passed(), ok, "qty {qty}");
            assert!(ok || msg_ok(&r, "G14"));
        }
        // Held 30 at mark 500 = 15,000; adding 11 => 20,500 fails, 10 => 20,000 passes.
        let held = snap(vec![pos("ABC", Side::Buy, 30, Tri::Known(d("490")))]);
        let ten = intent("ABC", Side::Buy, 10, Some("490"));
        assert!(g14(&c, &ten, &cls(&ten, &held), &held, d("500")).passed());
        let eleven = intent("ABC", Side::Buy, 11, Some("490"));
        assert!(!g14(&c, &eleven, &cls(&eleven, &held), &held, d("500")).passed());
        // A flip counts only the new excess.
        let flip = intent("ABC", Side::Sell, 70, Some("510"));
        assert!(g14(&c, &flip, &cls(&flip, &held), &held, d("500")).passed());
    }

    #[test]
    fn g14_rejects_unknowns_and_exits_pass() {
        let c = ctx();
        let i = intent("ABC", Side::Buy, 1, Some("490"));
        let held = snap(vec![pos("ABC", Side::Buy, 30, Tri::Known(d("490")))]);
        let mut nomark = held.clone();
        nomark.marks = Tri::Known(vec![]);
        assert!(!g14(&c, &i, &cls(&i, &nomark), &nomark, d("500")).passed());
        nomark.marks = Tri::Unknown;
        assert!(!g14(&c, &i, &cls(&i, &nomark), &nomark, d("500")).passed());
        assert!(!g14(&c, &i, &cls(&i, &held), &held, Decimal::ZERO).passed());
        let unknown = PortfolioSnapshot::unknown();
        assert!(!g14(&c, &i, &cls(&i, &unknown), &unknown, d("500")).passed());
        let exit = intent("ABC", Side::Sell, 30, None);
        assert!(g14(&c, &exit, &cls(&exit, &held), &held, d("500")).passed());
    }
}
