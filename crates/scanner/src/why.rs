//! Renders a [`Recommendation`]'s structured Why factors as beginner text (plan D12).
//!
//! Pure and deterministic: no clock, no I/O. Rupees go through `risk::money_fmt` (never `f64`);
//! free text (event notes, names, regime labels) is escaped before it is shown; the 0-100
//! confidence is always called a "score", never a chance. Order: Risk, Reward, Confluence,
//! `RegimeFit`, Evidence, Liquidity, Invalidation, Ban, Events, gate line, footer.
//!
//! The money lines (Risk, Reward, Invalidation) and the Evidence line are built from the
//! validated `Recommendation` fields, never from the factor copies; the factors only add what the
//! fields lack (the per-trade limit, the hold period, the notes). Ban, Events and Evidence are
//! never silent: a missing or unchecked factor renders a fixed "not checked" line, and the weakest
//! reading wins when factors of one kind disagree. For other kinds the first factor wins.
//! The direction words follow `rec.action()`.

use bazaar_risk::money_fmt::rupees;
use bazaar_risk::verdict::RuleId;
use rust_decimal::{Decimal, RoundingStrategy};

use crate::types::{Action, GateSummary, Recommendation, Warning, WhyFactor};

/// Longest free-text field shown (characters).
const MAX_TEXT_CHARS: usize = 120;
/// Most notes or names shown on one line; the rest is "... and N more".
const MAX_ITEMS: usize = 5;
/// One crore in rupees, for the liquidity line.
const CRORE: i64 = 10_000_000;

const FOOTER: &str =
    "Educational tool, not investment advice. Scores are not probabilities and can be wrong.";
const BAN_UNCHECKED: &str =
    "The ban list was not checked, so confirm this name is not banned before acting.";
const EVENTS_UNCHECKED: &str = "Results or policy dates were not checked.";
const NO_TESTING: &str = "No testing result is shown for this method. Treat this as an idea to study, not a signal to trust.";
/// Pending-only idea (RISK-165-2): the one gap is the missing approval. Never the passed wording.
const GATE_PENDING: &str = "Needs your approval. No order exists yet. The other safety checks pass right now and run again when you approve.";
const GATE_PASSED: &str = "The risk-rule checks passed when scanned (a pass does not make the trade safe); they run again before any order.";

/// Risk, reward and invalidation lines, the checks that stop a trade, the gate line and the
/// footer; a factor kind that is not a stop-or-money line is skipped when absent.
#[must_use]
pub fn render(rec: &Recommendation) -> Vec<String> {
    let mut out = vec![risk_line(rec)];
    out.extend(reward_line(rec));
    out.extend(find(rec, |f| match f {
        WhyFactor::Confluence {
            families_agreeing,
            families_total,
            names,
        } => Some(confluence_line(
            rec,
            *families_agreeing,
            *families_total,
            names,
        )),
        _ => None,
    }));
    out.extend(find(rec, |f| match f {
        WhyFactor::RegimeFit {
            market_trend,
            stock_trend,
            vol,
            penalised,
        } => Some(regime_line(market_trend, stock_trend, vol, *penalised)),
        _ => None,
    }));
    out.push(evidence_line(rec));
    out.extend(find(rec, |f| match f {
        WhyFactor::Liquidity {
            median_traded_value,
        } => Some(liquidity_line(*median_traded_value)),
        _ => None,
    }));
    out.push(invalidation_line(rec));
    out.push(ban_line(rec));
    out.push(events_line(rec));
    out.push(gate_line(rec.gate()));
    out.push(FOOTER.to_owned());
    out
}

/// The first factor that `pick` maps to a line.
fn find(rec: &Recommendation, pick: impl Fn(&WhyFactor) -> Option<String>) -> Option<String> {
    rec.why().iter().find_map(pick)
}

/// (word for the stop side, word for where the stop sits against entry). `Sell` has the stop
/// above entry; `NoTrade` cannot be built, so it gets neutral words rather than a panic.
const fn sides(a: Action) -> (&'static str, &'static str) {
    match a {
        Action::Buy => ("below", "below"),
        Action::Sell => ("above", "above"),
        _ => ("through", "away from"),
    }
}

fn risk_line(rec: &Recommendation) -> String {
    let (lo, hi) = rec.entry_zone();
    let qty = rec.size().qty;
    let shares = if qty == 1 { "share" } else { "shares" };
    let limit = rec
        .why()
        .iter()
        .find_map(|f| match f {
            WhyFactor::Risk { limit_rupees, .. } => Some(*limit_rupees),
            _ => None,
        })
        .map_or_else(String::new, |l| {
            format!(" (your limit per trade is {})", rupees(l))
        });
    format!(
        "If this goes wrong and your stop is hit, you lose about {} or more, as a gap or slippage can cost extra{limit}. Quantity {qty} {shares}, entry {} to {}, stop {} ({} your entry).",
        rupees(rec.risk_rupees()),
        rupees(lo),
        rupees(hi),
        rupees(rec.stop_loss()),
        sides(rec.action()).1,
    )
}

fn reward_line(rec: &Recommendation) -> Option<String> {
    let target = rec.targets().first()?;
    let r = ratio(rec.reward_risk());
    Some(format!(
        "Target {}. If the target is reached, for every Re 1 you risk you would make about Re {r} (reward to risk {r} : 1) on the worst fill, before charges and slippage. A target is a hope, not a promise.",
        rupees(*target),
    ))
}

/// A reward:risk ratio cut (not rounded up) to 2 dp so it is never overstated.
fn ratio(r: Decimal) -> String {
    let cut = r.round_dp_with_strategy(2, RoundingStrategy::ToZero);
    format!("{cut:.2}")
}

fn confluence_line(rec: &Recommendation, agree: u32, total: u32, names: &[String]) -> String {
    let names: Vec<String> = names.iter().map(|n| clean(n)).collect();
    format!(
        "{agree} of {total} method families agree: {}. Score {} out of 100 (a rule-fit score, not a chance of winning).",
        join_capped(&names, ", "),
        rec.confidence(),
    )
}

fn regime_line(market: &str, stock: &str, vol: &str, penalised: bool) -> String {
    let fit = if penalised {
        "This kind of market is allowed for this method but is rougher than ideal, so the score is cut."
    } else {
        "This method is meant for that kind of market."
    };
    format!(
        "The market is {} and this stock is {}, {} volatility. {fit}",
        clean(market),
        clean(stock),
        clean(vol),
    )
}

/// Built from `rec.backtest()`; an Evidence factor can only weaken it (fewer trades, a flag off,
/// a lower or unavailable profit factor). No backtest, or zero trades, is the no-testing line.
fn evidence_line(rec: &Recommendation) -> String {
    let Some(bt) = rec.backtest() else {
        return NO_TESTING.to_owned();
    };
    let mut trades = bt.trades;
    let mut costs = bt.costs_applied;
    let mut oos = bt.out_of_sample;
    let mut pf = finite(bt.profit_factor);
    let mut seen = false;
    for f in rec.why() {
        if let WhyFactor::Evidence {
            trades: t,
            profit_factor,
            costs_applied,
            out_of_sample,
        } = f
        {
            seen = true;
            trades = trades.min(*t);
            costs &= *costs_applied;
            oos &= *out_of_sample;
            pf = match (pf, profit_factor.and_then(finite)) {
                (Some(a), Some(b)) => Some(a.min(b)),
                _ => None,
            };
        }
    }
    // No Evidence factor means the Why carries no testing claim: say so rather than stay silent.
    if !seen || trades == 0 {
        return NO_TESTING.to_owned();
    }
    describe_evidence(rec, trades, costs, oos, pf)
}

fn finite(p: f64) -> Option<f64> {
    p.is_finite().then_some(p)
}

fn describe_evidence(
    rec: &Recommendation,
    trades: u32,
    costs: bool,
    oos: bool,
    pf: Option<f64>,
) -> String {
    let pf = match pf.and_then(|p| Decimal::try_from(p).ok()) {
        Some(d) => {
            let cut = d
                .round_dp(6)
                .round_dp_with_strategy(2, RoundingStrategy::ToZero);
            let loss = if cut < Decimal::ONE {
                " (below 1.00 means it lost money)"
            } else {
                ""
            };
            format!("{cut:.2} profit factor{loss}")
        }
        None => "profit factor not available".to_owned(),
    };
    let costs = if costs {
        "after costs"
    } else {
        "before costs, so it looks better than it is"
    };
    let sample = if oos {
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

fn liquidity_line(median_traded_value: Decimal) -> String {
    let crore = Decimal::from(CRORE);
    if median_traded_value >= crore {
        let c = (median_traded_value / crore).round_dp_with_strategy(2, RoundingStrategy::ToZero);
        format!(
            "Typical daily turnover of this share is about {} crore.",
            rupees(c),
        )
    } else {
        format!(
            "Only about {} of this share trades on a typical day, so getting in and out may be hard.",
            rupees(median_traded_value),
        )
    }
}

fn invalidation_line(rec: &Recommendation) -> String {
    let hold = rec
        .why()
        .iter()
        .find_map(|f| match f {
            WhyFactor::Invalidation { max_hold_bars, .. } => Some(*max_hold_bars),
            _ => None,
        })
        // A daily bar is a trading day (the only horizon today).
        .map_or_else(String::new, |n| {
            format!(", or if it has not worked after {n} trading days")
        });
    format!(
        "The idea is wrong if the price closes {} {}{hold}.",
        sides(rec.action()).0,
        rupees(rec.stop_loss()),
    )
}

/// Missing or any unchecked Ban factor is the unchecked line (the weakest reading wins).
fn ban_line(rec: &Recommendation) -> String {
    let mut date: Option<Option<&str>> = None;
    for f in rec.why() {
        if let WhyFactor::Ban { checked, list_date } = f {
            if !*checked {
                return BAN_UNCHECKED.to_owned();
            }
            date.get_or_insert(list_date.as_deref());
        }
    }
    match date {
        None => BAN_UNCHECKED.to_owned(),
        Some(Some(d)) => format!(
            "Not on the ban list dated {}. Check today's list before acting.",
            clean(d)
        ),
        Some(None) => {
            "Not on the ban list (list date unknown). Check today's list before acting.".to_owned()
        }
    }
}

/// Missing, unchecked, or a carried `EventsNotChecked` warning is the unchecked line; otherwise
/// every note of every Events factor is shown.
fn events_line(rec: &Recommendation) -> String {
    let mut seen = false;
    let mut all: Vec<String> = Vec::new();
    for f in rec.why() {
        if let WhyFactor::Events { checked, notes } = f {
            if !*checked {
                return EVENTS_UNCHECKED.to_owned();
            }
            seen = true;
            all.extend(notes.iter().map(|n| clean(n)));
        }
    }
    if !seen || rec.warnings().contains(&Warning::EventsNotChecked) {
        EVENTS_UNCHECKED.to_owned()
    } else if all.is_empty()
        && rec
            .warnings()
            .iter()
            .any(|w| matches!(w, Warning::EventNearby(_)))
    {
        "A date to know is near, but its details are missing: check the results and policy calendar before acting.".to_owned()
    } else if all.is_empty() {
        "No results or policy dates were found in the checked window.".to_owned()
    } else {
        format!("Dates to know: {}.", join_capped(&all, "; "))
    }
}

/// The gate line, over `GateSummary` only. The failure lists decide, never the `passed` flag: any
/// listed failure is blocked, a `paper_only` id the overridable list does not carry counts as
/// not overridable, and a blocked summary with no ids says the reason is unavailable (RISK-166-11).
fn gate_line(g: &GateSummary) -> String {
    let ids = |v: &[RuleId]| {
        v.iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    };
    // The lists decide too: the flag counts only with exactly G16 failing and nothing else.
    if g.approval_pending
        && !g.passed
        && g.hard_failures == [RuleId::G16]
        && g.overridable_failures.is_empty()
        && g.paper_only.is_empty()
    {
        return GATE_PENDING.to_owned();
    }
    let mut hard = g.hard_failures.clone();
    let mut paper = Vec::new();
    for r in &g.paper_only {
        if g.overridable_failures.contains(r) {
            if !hard.contains(r) {
                paper.push(*r);
            }
        } else if !hard.contains(r) {
            hard.push(*r);
        }
    }
    let soft: Vec<RuleId> = g
        .overridable_failures
        .iter()
        .copied()
        .filter(|r| !g.paper_only.contains(r) && !hard.contains(r))
        .collect();
    let mut parts = Vec::new();
    if !hard.is_empty() {
        parts.push(format!(
            "failed checks that cannot be overridden ({})",
            ids(&hard)
        ));
    }
    if !soft.is_empty() {
        parts.push(format!(
            "failed checks you can override with a written reason ({})",
            ids(&soft)
        ));
    }
    if !paper.is_empty() {
        parts.push(format!(
            "failed checks that can be overridden in paper mode only ({})",
            ids(&paper)
        ));
    }
    if parts.is_empty() {
        return if g.passed {
            GATE_PASSED.to_owned()
        } else {
            "Blocked by the safety checks, but the reason is not available: do not act on this."
                .to_owned()
        };
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
            '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}'
            | '\u{E0000}'..='\u{E007F}' => None,
            c => Some(c),
        })
        .collect();
    let collapsed = mapped.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > MAX_TEXT_CHARS {
        let cut: String = collapsed.chars().take(MAX_TEXT_CHARS).collect();
        format!("{cut}...")
    } else {
        collapsed
    }
}

/// Join at most [`MAX_ITEMS`] items, then "... and N more", so a line stays bounded.
fn join_capped(items: &[String], sep: &str) -> String {
    let shown = items.iter().take(MAX_ITEMS).cloned().collect::<Vec<_>>();
    let mut s = shown.join(sep);
    if items.len() > MAX_ITEMS {
        s.push_str(sep);
        s.push_str("... and ");
        s.push_str(&(items.len() - MAX_ITEMS).to_string());
        s.push_str(" more");
    }
    s
}

#[cfg(test)]
mod tests;
