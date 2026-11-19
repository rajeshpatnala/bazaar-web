//! Loss limits: G03 (daily loss) and G19 (weekly loss). Hard rows: a breach locks entries only
//! (`on_daily_loss = lockout_entries`); exits and stop moves always pass so a position can close.
//!
//! The P&L comes from the snapshot (realised plus unrealised, net of charges, built by the
//! journal); the base is `sizing_equity`, the figure `size` uses. A loss AT the limit blocks:
//! the limit is the most the owner accepts to lose, so reaching it ends the period. Compared
//! exactly (`loss x 100 >= pct x equity`). Unknown P&L, equity, time or week start rejects with
//! `RuleResult::input_missing` (never overridable). No price is needed here, so there is no
//! reference price. G19 also rejects a weekly figure whose `week_start` is not the IST Monday of
//! `as_of`: a stale week must not carry over, and a new week starts from zero on Monday.

use crate::classify::{Action, Classification};
use crate::config::RuleCtx;
use crate::money_fmt::{pct, rupees};
use crate::sizing::sizing_equity;
use crate::snapshot::PortfolioSnapshot;
use crate::types::Tri;
use crate::verdict::{RuleId, RuleResult};
use chrono::{Datelike, Duration, NaiveDate};
use rust_decimal::Decimal;

fn is_entry(cls: &Classification) -> bool {
    cls.action == Action::Entry && cls.entry_qty > 0
}

fn missing(rule: RuleId, limit: &str, why: &str) -> RuleResult {
    RuleResult::input_missing(
        rule,
        "cannot confirm",
        limit,
        &format!(
            "Cannot confirm {why}. When unsure, the app says no. To pass: refresh the account figures."
        ),
    )
}

/// Shared check: `pnl` against `cap_pct` of equity. `period` is "today" or "this week".
fn check(
    rule: RuleId,
    ctx: &RuleCtx,
    snap: &PortfolioSnapshot,
    pnl: Tri<Decimal>,
    cap_pct: Decimal,
    period: &str,
) -> RuleResult {
    let limit = pct(cap_pct);
    let (Tri::Known(pnl), Ok(equity)) = (pnl, sizing_equity(snap, &ctx.limits)) else {
        return missing(
            rule,
            &limit,
            &format!("{period}'s profit and loss or your account value"),
        );
    };
    let Some(cap) = cap_pct
        .checked_mul(equity)
        .and_then(|v| v.checked_div(Decimal::ONE_HUNDRED))
    else {
        return missing(rule, &limit, "the loss limit (the figures overflowed)");
    };
    let Some(loss) = Decimal::ZERO.checked_sub(pnl) else {
        return missing(rule, &limit, "the loss figure (it overflowed)");
    };
    let hit = loss
        .checked_mul(Decimal::ONE_HUNDRED)
        .zip(cap_pct.checked_mul(equity))
        .map(|(a, b)| a >= b);
    match hit {
        Some(false) => RuleResult::pass(rule, rupees(pnl), format!("loss {limit}")),
        Some(true) => RuleResult::fail(
            rule,
            rupees(pnl),
            format!("loss {limit}"),
            &format!(
                "You are down {} {period}, at or past your {} loss limit ({limit} of your account). New entries are stopped; exits are still allowed.",
                rupees(loss),
                rupees(cap)
            ),
        ),
        None => missing(rule, &limit, "the loss figures (they overflowed)"),
    }
}

/// G03: today's loss has reached `max_daily_loss_pct` of equity: no new entries.
#[must_use]
pub fn g03(ctx: &RuleCtx, cls: &Classification, snap: &PortfolioSnapshot) -> RuleResult {
    if !is_entry(cls) {
        return RuleResult::pass(
            RuleId::G03,
            "no new exposure",
            pct(ctx.limits.max_daily_loss_pct),
        );
    }
    check(
        RuleId::G03,
        ctx,
        snap,
        snap.day_pnl,
        ctx.limits.max_daily_loss_pct,
        "today",
    )
}

/// The Monday on or before `date`.
fn monday_of(date: NaiveDate) -> Option<NaiveDate> {
    date.checked_sub_signed(Duration::days(i64::from(
        date.weekday().num_days_from_monday(),
    )))
}

/// G19: this week's loss has reached `max_weekly_loss_pct` of equity: no new entries until the
/// week resets on Monday. The snapshot's `week_start` must be this IST week's Monday.
#[must_use]
pub fn g19(ctx: &RuleCtx, cls: &Classification, snap: &PortfolioSnapshot) -> RuleResult {
    let cap = ctx.limits.max_weekly_loss_pct;
    if !is_entry(cls) {
        return RuleResult::pass(RuleId::G19, "no new exposure", pct(cap));
    }
    let monday = snap.as_of.known().and_then(|t| monday_of(t.date_naive()));
    match (monday, snap.week_start) {
        (Some(m), Tri::Known(start)) if m == start => {}
        _ => {
            return missing(
                RuleId::G19,
                &pct(cap),
                "this week's start (it must be this week's Monday in IST)",
            );
        }
    }
    check(RuleId::G19, ctx, snap, snap.week_pnl, cap, "this week")
}
