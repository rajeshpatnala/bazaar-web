//! G05 sector and correlation exposure (Soft override). Pure function on supplied inputs: a
//! sector map, daily return series and one reference price. No data fetch, no I/O, no clock.
//!
//! Idea: two stocks that move together are one bet. Entry: runs only for the part of an order
//! that opens or adds exposure (`entry_qty`). Fails when the sector would exceed
//! `max_sector_pct` of equity, or when the new symbol plus the held symbols that move with the
//! new exposure would exceed `max_correlated_positions`. Exits and stop moves always pass.
//!
//! Guard state comes from the snapshot only: equity is `sizing_equity(snapshot, limits)` (the
//! same figure G14 and `size` use) and marks are `snapshot.marks`. The caller supplies only the
//! sector map and returns. The new entry is valued at `ref_price`, the worst case
//! `max(intent.price, live price)` computed once by the gate. Held positions of either side
//! count at their marks toward the sector value (gross, conservative).
//!
//! Correlation is side-aware. A held position counts toward the group when its returns move
//! WITH the exposure the entry adds: `s * cov > 0` and `cov^2 >= t^2 * var_a * var_b`, where
//! `s = +1` when the held position and the intent have the same side and `-1` when opposite.
//! So long+long and short+short count positive correlation; a long entry against a held short
//! counts NEGATIVE correlation (both legs lose together); a positively correlated opposite-side
//! pair is a hedge and is not counted (owner to confirm: hedges are not counted).
//!
//! Fail closed: missing inputs, unknown positions, unknown equity or marks, a symbol with no
//! sector-map entry or mark, a return series shorter than `correlation_window_days`, series
//! whose dates do not match, zero variance, or returns too small to compare all Reject with
//! what would make the rule pass.
//!
//! Maths is checked `Decimal` (no square root, no `f64`); any overflow rejects. The sector cap
//! is compared exactly (cross-multiplied); rounding is for the message only.

use crate::classify::{Action, Classification};
use crate::config::RuleCtx;
use crate::sizing::sizing_equity;
use crate::snapshot::{PortfolioSnapshot, Position};
use crate::types::{OrderIntent, Side};
use crate::verdict::{RuleId, RuleResult};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use std::collections::BTreeMap;

/// One daily return observation.
pub type ReturnPoint = (NaiveDate, Decimal);

/// Inputs G05 needs beyond the snapshot. Supplied by the caller; F3/F10 wire real data later.
/// Equity and marks are deliberately absent: they come from the snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExposureInputs {
    /// Symbol to sector name.
    pub sectors: BTreeMap<String, String>,
    /// Daily returns per symbol (intent symbol and every held symbol), oldest first.
    pub returns: BTreeMap<String, Vec<ReturnPoint>>,
}

fn reject(limit: &str, why: &str, fix: &str) -> RuleResult {
    RuleResult::fail(
        RuleId::G05,
        "cannot confirm",
        limit,
        &format!(
            "Cannot confirm sector and correlation exposure: {why}. When unsure, the app says no. To pass: {fix}."
        ),
    )
}

/// Evaluate G05 for one intent. `class` is the gate's own classification; `ref_price` is the
/// notional price for the entry, `max(intent.price, live price)`, computed once by the gate.
#[must_use]
pub fn evaluate(
    ctx: &RuleCtx,
    intent: &OrderIntent,
    class: &Classification,
    snapshot: &PortfolioSnapshot,
    inputs: Option<&ExposureInputs>,
    ref_price: Decimal,
) -> RuleResult {
    let l = &ctx.limits;
    let limit = format!(
        "sector {}% / {} correlated",
        l.max_sector_pct, l.max_correlated_positions
    );
    if class.action != Action::Entry || class.entry_qty == 0 {
        return RuleResult::pass(RuleId::G05, "no new exposure", limit);
    }
    let Some(inp) = inputs else {
        return reject(
            &limit,
            "sector map and return series were not supplied",
            "supply the sector map and returns",
        );
    };
    let Some(positions) = snapshot.positions.known() else {
        return reject(
            &limit,
            "open positions are unknown",
            "refresh the portfolio snapshot",
        );
    };
    let equity = match sizing_equity(snapshot, l) {
        Ok(e) => e,
        Err(e) => {
            return reject(
                &limit,
                &e.to_string(),
                "refresh the account figures in the snapshot",
            );
        }
    };
    if ref_price <= Decimal::ZERO {
        return reject(
            &limit,
            "the reference price is not positive",
            "supply a positive price",
        );
    }
    let Some(sector) = inp.sectors.get(&intent.symbol) else {
        return reject(
            &limit,
            &format!("{} has no sector entry", intent.symbol),
            "add the symbol to the sector map",
        );
    };
    let (sector_value, held) = match sector_value(
        snapshot,
        inp,
        intent,
        class.entry_qty,
        ref_price,
        sector,
        positions,
    ) {
        Ok(v) => v,
        Err((why, fix)) => return reject(&limit, &why, fix),
    };
    let pct = match check_sector_cap(l.max_sector_pct, sector_value, equity) {
        Ok(Ok(pct)) => pct,
        Ok(Err(pct)) => {
            return RuleResult::fail(
                RuleId::G05,
                format!("{sector} {pct}%"),
                format!("{}%", l.max_sector_pct),
                &format!(
                    "This trade would put {pct}% of your account in the {sector} sector, above your {}% cap. Several stocks in one sector act like one big bet.",
                    l.max_sector_pct
                ),
            );
        }
        Err(()) => return reject(&limit, "the sector check overflowed", "check equity"),
    };

    // Correlation against each other held symbol.
    let correlated = match correlated_held(ctx, inp, intent, &held) {
        Ok(c) => c,
        Err((why, fix)) => return reject(&limit, &why, fix),
    };
    let group = correlated.len().saturating_add(1);
    let max_group = usize::try_from(l.max_correlated_positions).unwrap_or(usize::MAX);
    if group > max_group {
        let first = correlated.first().copied().unwrap_or("a held stock");
        return RuleResult::fail(
            RuleId::G05,
            format!("{group} correlated"),
            format!("{} correlated", l.max_correlated_positions),
            &format!(
                "This stock moves much like one you already hold ({first}). Two similar bets act like one big bet."
            ),
        );
    }
    RuleResult::pass(
        RuleId::G05,
        format!("{sector} {pct}%, {group} correlated"),
        limit,
    )
}

/// Exact cap check: `value x 100 > cap x equity` fails. `Ok(Ok(pct))` within the cap,
/// `Ok(Err(pct))` over it (pct rounded for the message only), `Err(())` on overflow.
fn check_sector_cap(
    cap: Decimal,
    value: Decimal,
    equity: Decimal,
) -> Result<Result<Decimal, Decimal>, ()> {
    let (Some(scaled), Some(allowed)) = (
        value.checked_mul(Decimal::ONE_HUNDRED),
        cap.checked_mul(equity),
    ) else {
        return Err(());
    };
    let pct = scaled
        .checked_div(equity)
        .map_or(Decimal::ZERO, |p| p.round_dp(2));
    Ok(if scaled > allowed { Err(pct) } else { Ok(pct) })
}

/// Why a check cannot be confirmed, and what would fix it.
type Unconfirmed = (String, &'static str);

/// Other held symbols with the side that opened them.
type Held<'a> = Vec<(&'a str, Side)>;

/// Value in the intent's sector (held plus this entry) and the other held symbols with sides.
fn sector_value<'a>(
    snapshot: &PortfolioSnapshot,
    inp: &ExposureInputs,
    intent: &OrderIntent,
    entry_qty: u32,
    ref_price: Decimal,
    sector: &str,
    positions: &'a [Position],
) -> Result<(Decimal, Held<'a>), Unconfirmed> {
    let marks = snapshot.marks.known().ok_or_else(|| {
        (
            "mark prices are unknown".to_string(),
            "refresh the portfolio snapshot",
        )
    })?;
    let mut value = ref_price
        .checked_mul(Decimal::from(entry_qty))
        .ok_or_else(|| {
            (
                "order value overflowed".to_string(),
                "check the order price and quantity",
            )
        })?;
    let mut held: Held<'a> = Vec::new();
    for p in positions.iter().filter(|p| p.qty > 0) {
        let held_sector = inp.sectors.get(&p.symbol).ok_or_else(|| {
            (
                format!("held {} has no sector entry", p.symbol),
                "add it to the sector map",
            )
        })?;
        let mark = marks
            .iter()
            .find(|m| m.symbol == p.symbol)
            .map(|m| m.price)
            .filter(|m| *m > Decimal::ZERO)
            .ok_or_else(|| {
                (
                    format!("held {} has no mark price", p.symbol),
                    "supply a positive mark price",
                )
            })?;
        if held_sector == sector {
            value = mark
                .checked_mul(Decimal::from(p.qty))
                .and_then(|v| value.checked_add(v))
                .ok_or_else(|| {
                    (
                        "sector value overflowed".to_string(),
                        "check marks and quantities",
                    )
                })?;
        }
        if p.symbol != intent.symbol && !held.iter().any(|(s, _)| *s == p.symbol) {
            held.push((p.symbol.as_str(), p.side));
        }
    }
    Ok((value, held))
}

/// The held symbols that move with the exposure `intent` adds (see the module doc).
fn correlated_held<'a>(
    ctx: &RuleCtx,
    inp: &ExposureInputs,
    intent: &OrderIntent,
    held: &[(&'a str, Side)],
) -> Result<Vec<&'a str>, Unconfirmed> {
    let mut out = Vec::new();
    if held.is_empty() {
        return Ok(out);
    }
    let l = &ctx.limits;
    let window = usize::try_from(l.correlation_window_days).unwrap_or(usize::MAX);
    if window < 2 {
        return Err((
            format!(
                "the correlation window is {} day(s); it needs at least 2",
                l.correlation_window_days
            ),
            "set a valid correlation_window_days in risk.toml",
        ));
    }
    let short = |s: &str| {
        (
            format!("{s} has fewer than {window} daily returns"),
            "supply a full return window",
        )
    };
    let symbol = intent.symbol.as_str();
    let new_series = window_of(inp.returns.get(symbol), window).ok_or_else(|| short(symbol))?;
    for (h, side) in held {
        let held_series = window_of(inp.returns.get(*h), window).ok_or_else(|| short(h))?;
        match moves_with(
            new_series,
            held_series,
            *side == intent.side,
            l.correlation_threshold,
        ) {
            Ok(true) => out.push(*h),
            Ok(false) => {}
            Err(why) => {
                return Err((
                    format!("{symbol} vs {h}: {why}"),
                    "supply clean, aligned return series",
                ));
            }
        }
    }
    Ok(out)
}

/// The last `window` points, or `None` when absent or too short.
fn window_of(series: Option<&Vec<ReturnPoint>>, window: usize) -> Option<&[ReturnPoint]> {
    let s = series?;
    s.get(s.len().checked_sub(window)?..)
}

/// True when `s * cov > 0` and `cov^2 >= threshold^2 * var_a * var_b`, `s` = +1 for the same
/// side and -1 for opposite sides. `Err` = cannot tell.
fn moves_with(
    a: &[ReturnPoint],
    b: &[ReturnPoint],
    same_side: bool,
    threshold: Decimal,
) -> Result<bool, &'static str> {
    const OVF: &str = "arithmetic overflow";
    if a.len() != b.len() || a.len() < 2 {
        return Err("series lengths differ");
    }
    if a.iter().zip(b).any(|(x, y)| x.0 != y.0) {
        return Err("series dates do not match");
    }
    if a.windows(2).any(|w| match w {
        [x, y] => x.0 >= y.0,
        _ => true,
    }) {
        return Err("series dates are not strictly increasing");
    }
    let n = Decimal::from(a.len());
    let mean = |s: &[ReturnPoint]| -> Option<Decimal> {
        let mut t = Decimal::ZERO;
        for p in s {
            t = t.checked_add(p.1)?;
        }
        t.checked_div(n)
    };
    let (ma, mb) = (mean(a).ok_or(OVF)?, mean(b).ok_or(OVF)?);
    let step = |acc: Decimal, p: Decimal, q: Decimal| -> Option<Decimal> {
        acc.checked_add(p.checked_mul(q)?)
    };
    let (mut cov, mut va, mut vb) = (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO);
    for (x, y) in a.iter().zip(b) {
        let dx = x.1.checked_sub(ma).ok_or(OVF)?;
        let dy = y.1.checked_sub(mb).ok_or(OVF)?;
        cov = step(cov, dx, dy).ok_or(OVF)?;
        va = step(va, dx, dx).ok_or(OVF)?;
        vb = step(vb, dy, dy).ok_or(OVF)?;
    }
    if va.is_zero() || vb.is_zero() {
        return Err("a series has zero variance");
    }
    let with = if same_side {
        cov > Decimal::ZERO
    } else {
        cov < Decimal::ZERO
    };
    if !with {
        return Ok(false);
    }
    let lhs = cov.checked_mul(cov).ok_or(OVF)?;
    let rhs = threshold
        .checked_mul(threshold)
        .and_then(|t| t.checked_mul(va))
        .and_then(|t| t.checked_mul(vb))
        .ok_or(OVF)?;
    // Squares of tiny returns can round to 0 at 28 dp; "0 >= 0" would be a spurious answer.
    if lhs.is_zero() || rhs.is_zero() {
        return Err("returns are too small to compare");
    }
    Ok(lhs >= rhs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Baseline, GateConfig};
    use crate::snapshot::{Mark, Position};
    use crate::types::Tri;
    use bazaar_core::money::Qty;
    use chrono::{Duration, TimeZone};
    use chrono_tz::Asia::Kolkata;

    const TEXT: &str = include_str!("../../../../fixtures/config/risk.toml");
    const REAL: &str = include_str!("../../../../config/risk.toml");

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn ctx_of(text: &str) -> RuleCtx {
        let now = Kolkata.with_ymd_and_hms(2026, 11, 19, 10, 0, 0).unwrap();
        GateConfig::from_toml_str(text)
            .unwrap()
            .rule_ctx(&Baseline::none(), now)
    }

    fn ctx() -> RuleCtx {
        ctx_of(TEXT)
    }

    fn intent_side(sym: &str, qty: u32, side: Side) -> OrderIntent {
        OrderIntent {
            symbol: sym.into(),
            side,
            qty: Qty(qty),
            price: d("100"),
            stop: Some(d("95")),
            new_stop: None,
            lot_size: 1,
            tick_size: d("0.05"),
        }
    }

    fn intent(sym: &str, qty: u32) -> OrderIntent {
        intent_side(sym, qty, Side::Buy)
    }

    fn entry(qty: u32) -> Classification {
        Classification {
            action: Action::Entry,
            entry_qty: qty,
            exit_qty: 0,
            stop_direction: None,
        }
    }

    /// Snapshot with equity 100,000 (start, realised 0, MTM) and a mark of 100 per held symbol.
    fn snap_sides(held: &[(&str, Side, u32)]) -> PortfolioSnapshot {
        let mut s = PortfolioSnapshot::unknown();
        s.positions = Tri::Known(
            held.iter()
                .map(|(sym, side, q)| Position {
                    symbol: (*sym).into(),
                    side: *side,
                    qty: *q,
                    stop: Tri::Unknown,
                    risk_price: Tri::Unknown,
                    avg_cost: Tri::Unknown,
                })
                .collect(),
        );
        s.marks = Tri::Known(
            held.iter()
                .map(|(sym, _, _)| Mark {
                    symbol: (*sym).into(),
                    price: d("100"),
                })
                .collect(),
        );
        s.starting_cash = Tri::Known(d("100000"));
        s.realised_pnl = Tri::Known(Decimal::ZERO);
        s.equity_mtm = Tri::Known(d("100000"));
        s
    }

    fn snap(held: &[(&str, u32)]) -> PortfolioSnapshot {
        let v: Vec<(&str, Side, u32)> = held.iter().map(|(a, q)| (*a, Side::Buy, *q)).collect();
        snap_sides(&v)
    }

    fn set_mark(s: &mut PortfolioSnapshot, sym: &str, price: &str) {
        if let Tri::Known(m) = &mut s.marks {
            for x in m.iter_mut().filter(|x| x.symbol == sym) {
                x.price = d(price);
            }
        }
    }

    fn series(vals: &[String]) -> Vec<ReturnPoint> {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        vals.iter()
            .enumerate()
            .map(|(i, v)| (start + Duration::days(i64::try_from(i).unwrap()), d(v)))
            .collect()
    }

    fn strs(v: &[&str]) -> Vec<ReturnPoint> {
        series(&v.iter().map(|s| (*s).to_string()).collect::<Vec<_>>())
    }

    /// A `w`-long series built from a repeating pattern, optionally negated.
    fn long_w(pattern: &[&str], neg: bool, w: usize) -> Vec<ReturnPoint> {
        let vals: Vec<String> = (0..w)
            .map(|i| {
                let v = d(pattern[i % pattern.len()]);
                (if neg { -v } else { v }).to_string()
            })
            .collect();
        series(&vals)
    }

    fn long(pattern: &[&str], neg: bool) -> Vec<ReturnPoint> {
        long_w(pattern, neg, ctx().limits.correlation_window_days as usize)
    }

    const P: [&str; 5] = ["0.01", "-0.02", "0.015", "0.005", "-0.01"];
    const Q: [&str; 5] = ["0.02", "0.01", "-0.03", "0.0", "0.01"];

    fn inputs_w(w: usize) -> ExposureInputs {
        let sectors = [("NEW", "IT"), ("HELD", "BANK"), ("HELD2", "BANK")]
            .iter()
            .map(|(a, b)| ((*a).to_string(), (*b).to_string()))
            .collect();
        let returns = [
            ("NEW", long_w(&P, false, w)),
            ("HELD", long_w(&Q, false, w)),
            ("HELD2", long_w(&P, false, w)),
        ]
        .into_iter()
        .map(|(a, b)| (a.to_string(), b))
        .collect();
        ExposureInputs { sectors, returns }
    }

    fn inputs() -> ExposureInputs {
        inputs_w(ctx().limits.correlation_window_days as usize)
    }

    fn run_ctx(
        c: &RuleCtx,
        i: &OrderIntent,
        s: &PortfolioSnapshot,
        inp: Option<&ExposureInputs>,
    ) -> RuleResult {
        evaluate(c, i, &entry(i.qty.0), s, inp, d("100"))
    }

    fn run(i: &OrderIntent, s: &PortfolioSnapshot, inp: Option<&ExposureInputs>) -> RuleResult {
        run_ctx(&ctx(), i, s, inp)
    }

    fn bank(inp: &mut ExposureInputs) {
        inp.sectors.insert("NEW".into(), "BANK".into());
    }

    #[test]
    fn exit_and_stop_move_pass_without_inputs() {
        let i = intent("NEW", 1);
        let exit = Classification {
            action: Action::Exit,
            entry_qty: 0,
            exit_qty: 1,
            stop_direction: None,
        };
        assert!(evaluate(&ctx(), &i, &exit, &snap(&[]), None, d("100")).passed());
        let mv = Classification {
            action: Action::StopMove,
            entry_qty: 0,
            exit_qty: 0,
            stop_direction: None,
        };
        let unknown = PortfolioSnapshot::unknown();
        assert!(evaluate(&ctx(), &i, &mv, &unknown, None, d("100")).passed());
    }

    #[test]
    fn small_uncorrelated_entry_passes() {
        let r = run(&intent("NEW", 10), &snap(&[("HELD", 10)]), Some(&inputs()));
        assert!(r.passed(), "{}", r.message());
        assert_eq!(r.rule(), RuleId::G05);
    }

    #[test]
    fn missing_inputs_unknown_positions_and_bad_figures_reject() {
        let i = intent("NEW", 10);
        assert!(!run(&i, &snap(&[]), None).passed());
        assert!(!run(&i, &PortfolioSnapshot::unknown(), Some(&inputs())).passed());
        let mut s = snap(&[]);
        s.equity_mtm = Tri::Unknown;
        assert!(!run(&i, &s, Some(&inputs())).passed());
        let mut s = snap(&[]);
        s.equity_mtm = Tri::Known(Decimal::ZERO);
        assert!(!run(&i, &s, Some(&inputs())).passed());
        let r = evaluate(
            &ctx(),
            &i,
            &entry(10),
            &snap(&[]),
            Some(&inputs()),
            Decimal::ZERO,
        );
        assert!(!r.passed() && r.message().ends_with("(G05)"));
    }

    #[test]
    fn missing_sector_map_entry_rejects() {
        let mut inp = inputs();
        inp.sectors.remove("NEW");
        let r = run(&intent("NEW", 10), &snap(&[]), Some(&inp));
        assert!(!r.passed() && r.message().ends_with("(G05)"));
        let mut inp = inputs();
        inp.sectors.remove("HELD");
        assert!(!run(&intent("NEW", 10), &snap(&[("HELD", 1)]), Some(&inp)).passed());
    }

    #[test]
    fn missing_zero_or_unknown_marks_reject() {
        let i = intent("NEW", 10);
        let mut s = snap(&[("HELD", 1)]);
        s.marks = Tri::Known(vec![]);
        assert!(!run(&i, &s, Some(&inputs())).passed());
        let mut s = snap(&[("HELD", 1)]);
        set_mark(&mut s, "HELD", "0");
        assert!(!run(&i, &s, Some(&inputs())).passed());
        let mut s = snap(&[("HELD", 1)]);
        s.marks = Tri::Unknown;
        let r = run(&i, &s, Some(&inputs()));
        assert!(!r.passed() && r.message().contains("mark"));
    }

    #[test]
    fn sector_cap_boundary() {
        let mut inp = inputs();
        bank(&mut inp);
        let held = snap(&[("HELD", 100)]); // 10,000 held
        let cap = ctx().limits.max_sector_pct;
        assert_eq!(cap, d("25"), "fixture cap changed; adjust test");
        // 10,000 + 150 x 100 = 25,000 = exactly 25% of 100,000
        assert!(
            run(&intent("NEW", 150), &held, Some(&inp)).passed(),
            "exactly 25%"
        );
        let r = run(&intent("NEW", 151), &held, Some(&inp));
        assert!(!r.passed());
        assert!(r.message().contains("BANK") && r.message().ends_with("(G05)"));
    }

    #[test]
    fn cap_is_compared_unrounded() {
        let mut inp = inputs();
        bank(&mut inp);
        // 10,004 held + 15,000 entry = 25,004 = 25.004%, which rounds to 25.00 but is over.
        let mut held = snap(&[("HELD", 1)]);
        set_mark(&mut held, "HELD", "10004");
        let r = run(&intent("NEW", 150), &held, Some(&inp));
        assert!(!r.passed(), "25.004% must fail a 25% cap: {}", r.message());
        // 10,000 + 15,000 = exactly 25%: passes.
        let mut held = snap(&[("HELD", 1)]);
        set_mark(&mut held, "HELD", "10000");
        assert!(run(&intent("NEW", 150), &held, Some(&inp)).passed());
    }

    #[test]
    fn lower_equity_gives_a_lower_cap() {
        let mut inp = inputs();
        bank(&mut inp);
        let held = snap(&[("HELD", 100)]); // 10,000 held
        assert!(run(&intent("NEW", 100), &held, Some(&inp)).passed());
        // Account at 80,000 MTM: cap is 20,000, so 20,000 passes and one more share fails.
        let mut low = snap(&[("HELD", 100)]);
        low.equity_mtm = Tri::Known(d("80000"));
        assert!(run(&intent("NEW", 100), &low, Some(&inp)).passed());
        assert!(!run(&intent("NEW", 101), &low, Some(&inp)).passed());
        assert!(run(&intent("NEW", 101), &held, Some(&inp)).passed());
        // Equity above starting cash is capped at starting cash (fixture: capped).
        let mut high = snap(&[("HELD", 100)]);
        high.equity_mtm = Tri::Known(d("200000"));
        assert!(!run(&intent("NEW", 151), &high, Some(&inp)).passed());
    }

    #[test]
    fn reference_price_is_the_notional() {
        let mut inp = inputs();
        bank(&mut inp);
        let held = snap(&[("HELD", 100)]);
        let i = intent("NEW", 150); // intent.price 100
        let at_intent = evaluate(&ctx(), &i, &entry(150), &held, Some(&inp), d("100"));
        assert!(at_intent.passed(), "{}", at_intent.message());
        // The live price is higher: 150 x 100.1 + 10,000 = 25,015 is over 25%.
        let at_live = evaluate(&ctx(), &i, &entry(150), &held, Some(&inp), d("100.1"));
        assert!(!at_live.passed());
        let ovf = evaluate(&ctx(), &i, &entry(150), &held, Some(&inp), Decimal::MAX);
        assert!(!ovf.passed());
    }

    #[test]
    fn other_sector_value_does_not_count() {
        let inp = inputs(); // NEW is IT, HELD is BANK
        assert!(run(&intent("NEW", 250), &snap(&[("HELD", 500)]), Some(&inp)).passed());
    }

    #[test]
    fn flip_counts_only_the_entry_quantity() {
        let mut inp = inputs();
        bank(&mut inp);
        let held = snap(&[("HELD", 100)]); // 10,000
        // Order of 200 shares whose entry part is 50 (the rest closes a position).
        let i = intent("NEW", 200);
        let part = evaluate(&ctx(), &i, &entry(50), &held, Some(&inp), d("100"));
        assert!(part.passed(), "{}", part.message());
        let full = evaluate(&ctx(), &i, &entry(200), &held, Some(&inp), d("100"));
        assert!(!full.passed());
        let none = evaluate(&ctx(), &i, &entry(0), &held, None, d("100"));
        assert!(none.passed());
    }

    #[test]
    fn adding_to_a_held_symbol_counts_its_value_but_not_a_self_correlation() {
        let mut inp = inputs();
        bank(&mut inp);
        let held = snap(&[("NEW", 100)]); // 10,000 already in NEW
        assert!(run(&intent("NEW", 150), &held, Some(&inp)).passed());
        assert!(!run(&intent("NEW", 151), &held, Some(&inp)).passed());
        // Not correlated with itself: with a group limit of 1 the add still passes.
        let mut c = ctx();
        c.limits.max_correlated_positions = 1;
        assert!(run_ctx(&c, &intent("NEW", 10), &held, Some(&inp)).passed());
    }

    #[test]
    fn flat_held_position_is_ignored() {
        let held = snap(&[("GONE", 0), ("HELD", 10)]); // GONE has no sector or returns
        let r = run(&intent("NEW", 10), &held, Some(&inputs()));
        assert!(r.passed(), "{}", r.message());
    }

    #[test]
    fn short_positions_count_gross_toward_the_sector() {
        let mut inp = inputs();
        bank(&mut inp);
        let held = snap_sides(&[("HELD", Side::Sell, 100)]);
        assert!(run(&intent("NEW", 150), &held, Some(&inp)).passed());
        assert!(!run(&intent("NEW", 151), &held, Some(&inp)).passed());
    }

    #[test]
    fn correlated_group_over_limit_rejects() {
        let c = ctx();
        assert_eq!(
            c.limits.max_correlated_positions, 3,
            "fixture changed; adjust test"
        );
        let mut inp = inputs();
        for (k, s) in [("HELD", "S1"), ("HELD2", "S2"), ("HELD3", "S3")] {
            inp.sectors.insert(k.into(), s.into());
            inp.returns.insert(k.into(), long(&P, false));
        }
        // two correlated held: group 3 <= 3 passes
        let two = snap(&[("HELD", 10), ("HELD2", 10)]);
        assert!(run(&intent("NEW", 10), &two, Some(&inp)).passed());
        // three correlated held: group 4 > 3 rejects
        let r = run(
            &intent("NEW", 10),
            &snap(&[("HELD", 10), ("HELD2", 10), ("HELD3", 10)]),
            Some(&inp),
        );
        assert!(!r.passed());
        assert!(r.message().contains("moves much like one you already hold"));
    }

    #[test]
    fn real_risk_toml_values() {
        let c = ctx_of(REAL);
        let l = &c.limits;
        assert_eq!(
            (
                l.max_sector_pct,
                l.max_correlated_positions,
                l.correlation_threshold,
                l.correlation_window_days
            ),
            (d("30"), 2, d("0.7"), 60),
            "config/risk.toml changed; adjust test"
        );
        // Sector boundary at 30%: 10,000 + 200 x 100 = 30,000.
        let mut inp = inputs_w(60);
        bank(&mut inp);
        let held = snap(&[("HELD", 100)]);
        assert!(run_ctx(&c, &intent("NEW", 200), &held, Some(&inp)).passed());
        assert!(!run_ctx(&c, &intent("NEW", 201), &held, Some(&inp)).passed());
        // Correlation, group limit 2: one correlated held passes, two reject.
        let inp = inputs_w(60);
        let one = snap(&[("HELD", 10), ("HELD2", 10)]); // only HELD2 matches NEW
        assert!(run_ctx(&c, &intent("NEW", 10), &one, Some(&inp)).passed());
        let mut inp = inputs_w(60);
        inp.returns.insert("HELD".into(), long_w(&P, false, 60));
        let r = run_ctx(&c, &intent("NEW", 10), &one, Some(&inp));
        assert!(!r.passed(), "group of 3 over a limit of 2");
    }

    #[test]
    fn correlation_is_side_aware() {
        let mut c = ctx();
        c.limits.max_correlated_positions = 1; // one correlated held symbol rejects
        // (intent side, held side, held series negated, counts as correlated)
        let cases = [
            (Side::Buy, Side::Buy, false, true),
            (Side::Buy, Side::Buy, true, false),
            (Side::Buy, Side::Sell, false, false), // hedge
            (Side::Buy, Side::Sell, true, true),   // both legs lose together
            (Side::Sell, Side::Sell, false, true),
            (Side::Sell, Side::Sell, true, false),
            (Side::Sell, Side::Buy, false, false), // hedge
            (Side::Sell, Side::Buy, true, true),
        ];
        for (is, hs, neg, counts) in cases {
            let mut inp = inputs();
            inp.returns.insert("NEW".into(), long(&P, false));
            inp.returns.insert("HELD".into(), long(&P, neg));
            let s = snap_sides(&[("HELD", hs, 10)]);
            let r = run_ctx(&c, &intent_side("NEW", 10, is), &s, Some(&inp));
            assert_eq!(
                !r.passed(),
                counts,
                "intent {is:?}, held {hs:?}, negated {neg}: {}",
                r.message()
            );
        }
    }

    #[test]
    fn threshold_boundary_counts_at_equality() {
        // Correlation of exactly 0.8: b = 0.8 a + 0.6 c with c orthogonal to a.
        let a = strs(&["0.01", "-0.01", "0.01", "-0.01"]);
        let b = strs(&["0.014", "-0.002", "0.002", "-0.014"]);
        let mut inp = inputs();
        inp.returns.insert("NEW".into(), a);
        inp.returns.insert("HELD".into(), b);
        let s = snap(&[("HELD", 10)]);
        let mut c = ctx();
        c.limits.correlation_window_days = 4;
        c.limits.max_correlated_positions = 1;
        c.limits.correlation_threshold = d("0.8");
        assert!(
            !run_ctx(&c, &intent("NEW", 10), &s, Some(&inp)).passed(),
            "at the threshold counts"
        );
        c.limits.correlation_threshold = d("0.81");
        assert!(
            run_ctx(&c, &intent("NEW", 10), &s, Some(&inp)).passed(),
            "just below the threshold does not"
        );
    }

    #[test]
    fn tiny_returns_reject_instead_of_reading_zero_as_correlated() {
        let t = ["0.00000001", "-0.00000001", "0.00000001", "-0.00000001"];
        let mut inp = inputs();
        inp.returns.insert("NEW".into(), strs(&t));
        inp.returns.insert("HELD".into(), strs(&t));
        let mut c = ctx();
        c.limits.correlation_window_days = 4;
        let r = run_ctx(&c, &intent("NEW", 10), &snap(&[("HELD", 10)]), Some(&inp));
        assert!(!r.passed() && r.message().contains("too small"));
    }

    #[test]
    fn bad_window_config_has_its_own_message() {
        let mut c = ctx();
        c.limits.correlation_window_days = 1;
        let r = run_ctx(
            &c,
            &intent("NEW", 10),
            &snap(&[("HELD", 10)]),
            Some(&inputs()),
        );
        assert!(!r.passed() && r.message().contains("correlation window"));
        // No held symbols: the window is not needed.
        assert!(run_ctx(&c, &intent("NEW", 10), &snap(&[]), Some(&inputs())).passed());
    }

    #[test]
    fn zero_variance_rejects() {
        let mut inp = inputs();
        inp.returns.insert("HELD".into(), long(&["0.01"], false));
        let r = run(&intent("NEW", 10), &snap(&[("HELD", 10)]), Some(&inp));
        assert!(!r.passed() && r.message().contains("zero variance"));
        let mut inp = inputs();
        inp.returns.insert("NEW".into(), long(&["0.01"], false));
        assert!(!run(&intent("NEW", 10), &snap(&[("HELD", 10)]), Some(&inp)).passed());
    }

    #[test]
    fn short_or_missing_series_rejects() {
        let mut inp = inputs();
        let mut s = long(&Q, false);
        s.pop();
        inp.returns.insert("HELD".into(), s);
        let r = run(&intent("NEW", 10), &snap(&[("HELD", 10)]), Some(&inp));
        assert!(!r.passed() && r.message().contains("fewer than"));
        let mut inp = inputs();
        inp.returns.remove("NEW");
        assert!(!run(&intent("NEW", 10), &snap(&[("HELD", 10)]), Some(&inp)).passed());
        let mut inp = inputs();
        inp.returns.remove("HELD");
        assert!(!run(&intent("NEW", 10), &snap(&[("HELD", 10)]), Some(&inp)).passed());
    }

    #[test]
    fn mismatched_dates_reject() {
        let mut inp = inputs();
        let mut s = long(&Q, false);
        if let Some(p) = s.last_mut() {
            p.0 += Duration::days(30);
        }
        inp.returns.insert("HELD".into(), s);
        let r = run(&intent("NEW", 10), &snap(&[("HELD", 10)]), Some(&inp));
        assert!(!r.passed() && r.message().contains("dates do not match"));
    }

    #[test]
    fn non_increasing_dates_reject() {
        let mut inp = inputs();
        for k in ["NEW", "HELD"] {
            let mut s = long(&Q, false);
            let (a, b) = (s[3].0, s[4].0);
            s[3].0 = b;
            s[4].0 = a;
            inp.returns.insert(k.into(), s);
        }
        let r = run(&intent("NEW", 10), &snap(&[("HELD", 10)]), Some(&inp));
        assert!(!r.passed() && r.message().contains("strictly increasing"));
    }

    #[test]
    fn overflow_rejects_instead_of_panicking() {
        let r = evaluate(
            &ctx(),
            &intent("NEW", 10),
            &entry(10),
            &snap(&[]),
            Some(&inputs()),
            Decimal::MAX,
        );
        assert!(!r.passed());
        let mut inp = inputs();
        let w = ctx().limits.correlation_window_days;
        let huge: Vec<String> = (0..w)
            .map(|k| {
                if k % 2 == 0 {
                    "50000000000000000"
                } else {
                    "-50000000000000000"
                }
            })
            .map(String::from)
            .collect();
        inp.returns.insert("HELD".into(), series(&huge));
        inp.returns.insert("NEW".into(), series(&huge));
        let r = run(&intent("NEW", 10), &snap(&[("HELD", 10)]), Some(&inp));
        assert!(!r.passed() && r.message().contains("overflow"));
    }

    #[test]
    fn only_the_last_window_is_compared() {
        let mut inp = inputs();
        let mut s = long(&Q, false);
        // Older, differently dated point before the window must be ignored.
        s.insert(0, (NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(), d("9")));
        inp.returns.insert("HELD".into(), s);
        assert!(run(&intent("NEW", 10), &snap(&[("HELD", 10)]), Some(&inp)).passed());
    }
}
