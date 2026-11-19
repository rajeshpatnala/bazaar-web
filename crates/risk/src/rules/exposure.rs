//! G05 sector and correlation exposure (Soft override). Pure function on supplied inputs: a
//! sector map, mark prices and daily return series. No data fetch, no I/O, no clock.
//!
//! Idea: two stocks that move together are one bet. Entry: runs only for the part of an order
//! that opens or adds exposure. Fails when the sector would exceed `max_sector_pct` of equity,
//! or when the new symbol plus the held symbols with return correlation at or above
//! `correlation_threshold` would exceed `max_correlated_positions`. Held positions of either
//! side count (conservative: a hedge is treated as a correlated bet). Only positive correlation
//! counts. Exits and stop moves always pass.
//!
//! Fail closed: missing inputs, an unknown position list, a symbol with no sector-map entry or
//! no mark price, a return series shorter than `correlation_window_days`, series whose dates do
//! not match, or zero variance all Reject with what would make the rule pass.
//!
//! Correlation is compared as `cov^2 >= threshold^2 * var_a * var_b` with `cov > 0`, in
//! checked `Decimal` maths (no square root, no `f64`); any overflow rejects.

use crate::classify::{Action, Classification};
use crate::config::RuleCtx;
use crate::snapshot::{PortfolioSnapshot, Position};
use crate::types::OrderIntent;
use crate::verdict::{RuleId, RuleResult};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use std::collections::BTreeMap;

/// One daily return observation.
pub type ReturnPoint = (NaiveDate, Decimal);

/// Inputs G05 needs beyond the snapshot. Supplied by the caller; F3/F10 wire real data later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExposureInputs {
    /// Symbol to sector name.
    pub sectors: BTreeMap<String, String>,
    /// Current mark price per held symbol.
    pub marks: BTreeMap<String, Decimal>,
    /// Daily returns per symbol (intent symbol and every held symbol), oldest first.
    pub returns: BTreeMap<String, Vec<ReturnPoint>>,
    /// Equity the sector percentage is measured against.
    pub equity: Decimal,
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

/// Evaluate G05 for one intent. `class` is the gate's own classification.
#[must_use]
pub fn evaluate(
    ctx: &RuleCtx,
    intent: &OrderIntent,
    class: &Classification,
    snapshot: &PortfolioSnapshot,
    inputs: Option<&ExposureInputs>,
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
            "supply the sector map, marks and returns",
        );
    };
    let Some(positions) = snapshot.positions.known() else {
        return reject(
            &limit,
            "open positions are unknown",
            "refresh the portfolio snapshot",
        );
    };
    if inp.equity <= Decimal::ZERO {
        return reject(
            &limit,
            "equity is not positive",
            "supply a positive equity figure",
        );
    }
    let Some(sector) = inp.sectors.get(&intent.symbol) else {
        return reject(
            &limit,
            &format!("{} has no sector entry", intent.symbol),
            "add the symbol to the sector map",
        );
    };

    // Sector value: held value in the same sector plus this entry.
    let (sector_value, held_symbols) =
        match sector_value(inp, intent, class.entry_qty, sector, positions) {
            Ok(v) => v,
            Err((why, fix)) => return reject(&limit, &why, fix),
        };
    let Some(pct) = sector_value
        .checked_mul(Decimal::ONE_HUNDRED)
        .and_then(|v| v.checked_div(inp.equity))
    else {
        return reject(
            &limit,
            "sector percentage overflowed",
            "check equity and marks",
        );
    };
    let pct = pct.round_dp(2);
    if pct > l.max_sector_pct {
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

    // Correlation against each other held symbol.
    let correlated = match correlated_held(ctx, inp, &intent.symbol, &held_symbols) {
        Ok(c) => c,
        Err((why, fix)) => return reject(&limit, &why, fix),
    };
    let group = correlated.len().saturating_add(1);
    if group > l.max_correlated_positions as usize {
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

/// Why a check cannot be confirmed, and what would fix it.
type Unconfirmed = (String, &'static str);

/// Value in the intent's sector (held plus this entry) and the other held symbols.
fn sector_value<'a>(
    inp: &ExposureInputs,
    intent: &OrderIntent,
    entry_qty: u32,
    sector: &str,
    positions: &'a [Position],
) -> Result<(Decimal, Vec<&'a str>), Unconfirmed> {
    let mut value = intent
        .price
        .checked_mul(Decimal::from(entry_qty))
        .ok_or_else(|| {
            (
                "order value overflowed".to_string(),
                "check the order price and quantity",
            )
        })?;
    let mut held: Vec<&'a str> = Vec::new();
    for p in positions.iter().filter(|p| p.qty > 0) {
        let held_sector = inp.sectors.get(&p.symbol).ok_or_else(|| {
            (
                format!("held {} has no sector entry", p.symbol),
                "add it to the sector map",
            )
        })?;
        let mark = inp
            .marks
            .get(&p.symbol)
            .filter(|m| **m > Decimal::ZERO)
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
        if p.symbol != intent.symbol && !held.contains(&p.symbol.as_str()) {
            held.push(p.symbol.as_str());
        }
    }
    Ok((value, held))
}

/// The held symbols whose returns correlate with `symbol` at or above the threshold.
fn correlated_held<'a>(
    ctx: &RuleCtx,
    inp: &ExposureInputs,
    symbol: &str,
    held: &[&'a str],
) -> Result<Vec<&'a str>, Unconfirmed> {
    let mut out = Vec::new();
    if held.is_empty() {
        return Ok(out);
    }
    let l = &ctx.limits;
    let window = l.correlation_window_days as usize;
    let short = |s: &str| {
        (
            format!("{s} has fewer than {window} daily returns"),
            "supply a full return window",
        )
    };
    let new_series = window_of(inp.returns.get(symbol), window).ok_or_else(|| short(symbol))?;
    for h in held {
        let held_series = window_of(inp.returns.get(*h), window).ok_or_else(|| short(h))?;
        match correlated_at_or_above(new_series, held_series, l.correlation_threshold) {
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

/// The last `window` points, or `None` when absent, `window < 2`, or too short.
fn window_of(series: Option<&Vec<ReturnPoint>>, window: usize) -> Option<&[ReturnPoint]> {
    if window < 2 {
        return None;
    }
    let s = series?;
    s.get(s.len().checked_sub(window)?..)
}

/// True when the series correlate positively at or above `threshold`. `Err` = cannot tell.
fn correlated_at_or_above(
    a: &[ReturnPoint],
    b: &[ReturnPoint],
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
    if cov <= Decimal::ZERO {
        return Ok(false);
    }
    let lhs = cov.checked_mul(cov).ok_or(OVF)?;
    let rhs = threshold
        .checked_mul(threshold)
        .and_then(|t| t.checked_mul(va))
        .and_then(|t| t.checked_mul(vb))
        .ok_or(OVF)?;
    Ok(lhs >= rhs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Baseline, GateConfig};
    use crate::snapshot::Position;
    use crate::types::{Side, Tri};
    use bazaar_core::money::Qty;
    use chrono::{Duration, TimeZone};
    use chrono_tz::Asia::Kolkata;

    const TEXT: &str = include_str!("../../../../fixtures/config/risk.toml");

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn ctx() -> RuleCtx {
        let now = Kolkata.with_ymd_and_hms(2026, 11, 19, 10, 0, 0).unwrap();
        GateConfig::from_toml_str(TEXT)
            .unwrap()
            .rule_ctx(&Baseline::none(), now)
    }

    fn intent(sym: &str, qty: u32) -> OrderIntent {
        OrderIntent {
            symbol: sym.into(),
            side: Side::Buy,
            qty: Qty(qty),
            price: d("100"),
            stop: Some(d("95")),
            new_stop: None,
            lot_size: 1,
            tick_size: d("0.05"),
        }
    }

    fn entry(qty: u32) -> Classification {
        Classification {
            action: Action::Entry,
            entry_qty: qty,
            exit_qty: 0,
            stop_direction: None,
        }
    }

    fn snap(held: &[(&str, u32)]) -> PortfolioSnapshot {
        let mut s = PortfolioSnapshot::unknown();
        s.positions = Tri::Known(
            held.iter()
                .map(|(sym, q)| Position {
                    symbol: (*sym).into(),
                    side: Side::Buy,
                    qty: *q,
                    stop: Tri::Unknown,
                })
                .collect(),
        );
        s
    }

    fn series(vals: &[String]) -> Vec<ReturnPoint> {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        vals.iter()
            .enumerate()
            .map(|(i, v)| (start + Duration::days(i64::try_from(i).unwrap()), d(v)))
            .collect()
    }

    /// A window-long series built from a repeating pattern, optionally negated.
    fn long(pattern: &[&str], neg: bool) -> Vec<ReturnPoint> {
        let w = ctx().limits.correlation_window_days as usize;
        let vals: Vec<String> = (0..w)
            .map(|i| {
                let v = d(pattern[i % pattern.len()]);
                (if neg { -v } else { v }).to_string()
            })
            .collect();
        series(&vals)
    }

    const P: [&str; 5] = ["0.01", "-0.02", "0.015", "0.005", "-0.01"];
    const Q: [&str; 5] = ["0.02", "0.01", "-0.03", "0.0", "0.01"];

    fn inputs() -> ExposureInputs {
        let sectors = [("NEW", "IT"), ("HELD", "BANK"), ("HELD2", "BANK")]
            .iter()
            .map(|(a, b)| ((*a).to_string(), (*b).to_string()))
            .collect();
        let marks = [("HELD", d("100")), ("HELD2", d("100"))]
            .iter()
            .map(|(a, b)| ((*a).to_string(), *b))
            .collect();
        let returns = [
            ("NEW", long(&P, false)),
            ("HELD", long(&Q, false)),
            ("HELD2", long(&P, false)),
        ]
        .into_iter()
        .map(|(a, b)| (a.to_string(), b))
        .collect();
        ExposureInputs {
            sectors,
            marks,
            returns,
            equity: d("100000"),
        }
    }

    fn run(i: &OrderIntent, s: &PortfolioSnapshot, inp: Option<&ExposureInputs>) -> RuleResult {
        evaluate(&ctx(), i, &entry(i.qty.0), s, inp)
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
        assert!(evaluate(&ctx(), &i, &exit, &snap(&[]), None).passed());
        let mv = Classification {
            action: Action::StopMove,
            entry_qty: 0,
            exit_qty: 0,
            stop_direction: None,
        };
        assert!(evaluate(&ctx(), &i, &mv, &PortfolioSnapshot::unknown(), None).passed());
    }

    #[test]
    fn small_uncorrelated_entry_passes() {
        let r = run(&intent("NEW", 10), &snap(&[("HELD", 10)]), Some(&inputs()));
        assert!(r.passed(), "{}", r.message());
        assert_eq!(r.rule(), RuleId::G05);
    }

    #[test]
    fn missing_inputs_unknown_positions_and_bad_equity_reject() {
        let i = intent("NEW", 10);
        assert!(!run(&i, &snap(&[]), None).passed());
        assert!(!run(&i, &PortfolioSnapshot::unknown(), Some(&inputs())).passed());
        let mut inp = inputs();
        inp.equity = Decimal::ZERO;
        assert!(!run(&i, &snap(&[]), Some(&inp)).passed());
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
    fn missing_or_zero_mark_rejects() {
        let mut inp = inputs();
        inp.marks.remove("HELD");
        assert!(!run(&intent("NEW", 10), &snap(&[("HELD", 1)]), Some(&inp)).passed());
        let mut inp = inputs();
        inp.marks.insert("HELD".into(), Decimal::ZERO);
        assert!(!run(&intent("NEW", 10), &snap(&[("HELD", 1)]), Some(&inp)).passed());
    }

    #[test]
    fn sector_cap_boundary() {
        let mut inp = inputs();
        inp.sectors.insert("NEW".into(), "BANK".into());
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
    fn other_sector_value_does_not_count() {
        let inp = inputs(); // NEW is IT, HELD is BANK
        assert!(run(&intent("NEW", 250), &snap(&[("HELD", 500)]), Some(&inp)).passed());
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
            inp.marks.insert(k.into(), d("100"));
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
    fn negative_correlation_does_not_count() {
        let mut inp = inputs();
        inp.returns.insert("HELD".into(), long(&P, true));
        inp.returns.insert("HELD2".into(), long(&P, true));
        let r = run(
            &intent("NEW", 10),
            &snap(&[("HELD", 10), ("HELD2", 10)]),
            Some(&inp),
        );
        assert!(r.passed(), "{}", r.message());
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
        assert!(!run(&intent("NEW", 10), &snap(&[("HELD", 10)]), Some(&inp)).passed());
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
        let mut i = intent("NEW", 10);
        i.price = Decimal::MAX;
        assert!(!run(&i, &snap(&[]), Some(&inputs())).passed());
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
