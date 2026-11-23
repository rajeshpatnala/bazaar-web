//! Renders a [`Recommendation`]'s structured Why factors as beginner text (plan D12).
//!
//! Pure and deterministic: no clock, no I/O. Rupees go through `risk::money_fmt` (never `f64`);
//! free text (event notes, names, regime labels) is escaped before it is shown; the 0-100
//! confidence is always called a "score", never a chance. Order: Risk, Reward, Confluence,
//! `RegimeFit`, Evidence, Liquidity, Invalidation, Ban, Events, gate line, footer. A factor the
//! recommendation does not carry is skipped; the gate line and footer are always present.

use bazaar_risk::money_fmt::rupees;
use bazaar_risk::verdict::RuleId;
use rust_decimal::{Decimal, RoundingStrategy};

use crate::types::{GateSummary, Horizon, Recommendation, WhyFactor};

/// Longest free-text field shown (characters).
const MAX_TEXT_CHARS: usize = 120;
/// One crore in rupees, for the liquidity line.
const CRORE: i64 = 10_000_000;

const FOOTER: &str =
    "Educational tool, not investment advice. Scores are not probabilities and can be wrong.";

/// One line per factor present, then the gate line and the footer.
#[must_use]
pub fn render(rec: &Recommendation) -> Vec<String> {
    let unit = time_unit(rec.horizon());
    let mut out = Vec::new();
    for want in 0..=8 {
        if let Some(f) = rec.why().iter().find(|f| rank(f) == want) {
            out.push(factor_line(rec, f, unit));
        }
    }
    out.push(gate_line(rec.gate()));
    out.push(FOOTER.to_owned());
    out
}

const fn rank(f: &WhyFactor) -> u8 {
    match f {
        WhyFactor::Risk { .. } => 0,
        WhyFactor::Reward { .. } => 1,
        WhyFactor::Confluence { .. } => 2,
        WhyFactor::RegimeFit { .. } => 3,
        WhyFactor::Evidence { .. } => 4,
        WhyFactor::Liquidity { .. } => 5,
        WhyFactor::Invalidation { .. } => 6,
        WhyFactor::Ban { .. } => 7,
        WhyFactor::Events { .. } => 8,
    }
}

/// "days" only when a bar is a day (the only horizon today); any other horizon would say "bars".
const fn time_unit(h: Horizon) -> &'static str {
    match h {
        Horizon::Swing => "days",
    }
}

fn factor_line(rec: &Recommendation, f: &WhyFactor, unit: &str) -> String {
    match f {
        WhyFactor::Risk {
            risk_rupees,
            limit_rupees,
            qty,
        } => {
            let (lo, hi) = rec.entry_zone();
            let shares = if *qty == 1 { "share" } else { "shares" };
            format!(
                "If this goes wrong and your stop is hit, you lose about {} (your limit per trade is {}). Quantity {qty} {shares}, entry {} to {}, stop {}.",
                rupees(*risk_rupees),
                rupees(*limit_rupees),
                rupees(lo),
                rupees(hi),
                rupees(rec.stop_loss()),
            )
        }
        WhyFactor::Reward {
            target,
            reward_risk,
        } => format!(
            "Target {}. On the worst fill you would make about {} for every Re 1 you risk (reward to risk). A target is a hope, not a promise.",
            rupees(*target),
            ratio(*reward_risk),
        ),
        WhyFactor::Confluence {
            families_agreeing,
            families_total,
            names,
        } => {
            let names: Vec<String> = names.iter().map(|n| clean(n)).collect();
            format!(
                "{families_agreeing} of {families_total} method families agree: {}. Score {} out of 100 (a rule-fit score, not a chance of winning).",
                names.join(", "),
                rec.confidence(),
            )
        }
        WhyFactor::RegimeFit {
            market_trend,
            stock_trend,
            vol,
            penalised,
        } => {
            let fit = if *penalised {
                "This kind of market is allowed for this method but is rougher than ideal, so the score is cut."
            } else {
                "This method is meant for that kind of market."
            };
            format!(
                "The market is {} and this stock is {}, {} volatility. {fit}",
                clean(market_trend),
                clean(stock_trend),
                clean(vol),
            )
        }
        WhyFactor::Evidence {
            trades,
            profit_factor,
            costs_applied,
            out_of_sample,
        } => evidence_line(rec, *trades, *profit_factor, *costs_applied, *out_of_sample),
        WhyFactor::Liquidity {
            median_traded_value,
        } => liquidity_line(*median_traded_value),
        WhyFactor::Invalidation {
            stop,
            max_hold_bars,
        } => format!(
            "The idea is wrong if the price closes below {}, or if it has not worked after {max_hold_bars} {unit}.",
            rupees(*stop),
        ),
        WhyFactor::Ban { checked, list_date } => match (checked, list_date) {
            (false, _) => {
                "The ban list was not checked, so confirm this name is not banned before acting."
                    .to_owned()
            }
            (true, Some(d)) => format!("No ban today (list dated {}).", clean(d)),
            (true, None) => "No ban today.".to_owned(),
        },
        WhyFactor::Events { checked, notes } => events_line(*checked, notes),
    }
}

/// A reward:risk ratio as "1.33 : 1", cut (not rounded up) to 2 dp so it is never overstated.
fn ratio(r: Decimal) -> String {
    let cut = r.round_dp_with_strategy(2, RoundingStrategy::ToZero);
    format!("{cut:.2} : 1")
}

fn liquidity_line(median_traded_value: Decimal) -> String {
    let crore = Decimal::from(CRORE);
    if median_traded_value >= crore {
        format!(
            "About {} crore of this share trades on a typical day, so you can get in and out.",
            rupees(median_traded_value / crore),
        )
    } else {
        format!(
            "Only about {} of this share trades on a typical day, so getting in and out may be hard.",
            rupees(median_traded_value),
        )
    }
}

fn events_line(checked: bool, notes: &[String]) -> String {
    if !checked {
        "Results or policy dates were not checked.".to_owned()
    } else if notes.is_empty() {
        "No results or policy dates were found in the checked window.".to_owned()
    } else {
        let notes: Vec<String> = notes.iter().map(|n| clean(n)).collect();
        format!("Dates to know: {}.", notes.join("; "))
    }
}

fn evidence_line(
    rec: &Recommendation,
    trades: u32,
    profit_factor: Option<f64>,
    costs_applied: bool,
    out_of_sample: bool,
) -> String {
    if trades == 0 || rec.backtest().is_none() {
        return "No past testing exists for this method yet. Treat this as an idea to study, not a signal to trust.".to_owned();
    }
    let pf = match profit_factor {
        Some(p) if p.is_finite() => format!("{p:.2} profit factor"),
        _ => "profit factor not available".to_owned(),
    };
    let costs = if costs_applied {
        "after costs"
    } else {
        "before costs, so it looks better than it is"
    };
    let sample = if out_of_sample {
        "on data the method had not seen"
    } else {
        "on data the method was fitted to, so it may flatter"
    };
    let low = if rec.low_evidence() {
        " This is low evidence, so the score is held down."
    } else {
        ""
    };
    format!("Tested on {trades} past trades {costs}, {sample}: {pf}.{low}")
}

/// The gate line, over `GateSummary` only (kept small so the gate wiring can change it alone).
fn gate_line(g: &GateSummary) -> String {
    let ids = |v: &[RuleId]| {
        v.iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    };
    if g.passed {
        return "The safety checks passed when this was scanned; they run again before any order."
            .to_owned();
    }
    let mut parts = Vec::new();
    if !g.hard_failures.is_empty() {
        parts.push(format!(
            "failed checks that cannot be overridden ({})",
            ids(&g.hard_failures)
        ));
    }
    let soft: Vec<RuleId> = g
        .overridable_failures
        .iter()
        .copied()
        .filter(|r| !g.paper_only.contains(r))
        .collect();
    if !soft.is_empty() {
        parts.push(format!(
            "failed checks you can override with a written reason ({})",
            ids(&soft)
        ));
    }
    if !g.paper_only.is_empty() {
        parts.push(format!(
            "failed checks that can be overridden in paper mode only ({})",
            ids(&g.paper_only)
        ));
    }
    format!("Blocked by the safety checks: {}.", parts.join("; "))
}

/// Escape free text for display: control and bidi characters dropped, markup characters and all
/// whitespace (newlines included) turned into single spaces, capped at [`MAX_TEXT_CHARS`].
fn clean(s: &str) -> String {
    let mapped: String = s
        .chars()
        .filter_map(|c| match c {
            '<' | '>' | '&' | '"' | '\'' => Some(' '),
            c if c.is_whitespace() => Some(' '),
            c if c.is_control() => None,
            '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}' => None,
            c => Some(c),
        })
        .collect();
    let collapsed = mapped.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(MAX_TEXT_CHARS).collect()
}

#[cfg(test)]
mod tests;
