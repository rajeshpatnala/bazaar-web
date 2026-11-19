//! Loss limits: G03 (daily loss) and G19 (weekly loss). Hard rows: a breach locks entries only
//! (`on_daily_loss = lockout_entries`); exits and stop moves always pass so a position can close.
//!
//! The P&L comes from the snapshot (realised plus unrealised, net of charges, built by the
//! journal); the base is `sizing_equity`, the figure `size` uses. A loss AT the limit blocks:
//! the limit is the most the owner accepts to lose, so reaching it ends the period. Compared
//! exactly (`loss x 100 >= pct x equity`). Unknown P&L, equity, time or week start rejects with
//! `RuleResult::input_missing` (never overridable). No price is needed here, so there is no
//! reference price. G19 also rejects a weekly figure whose `week_start` is not the IST Monday of
//! `as_of` (converted to IST first, whatever zone it carries): a stale week must not carry
//! over, and a new week starts from zero on Monday. Freshness of the snapshot itself is not
//! checked here: these rules rely on G15 (`max_snapshot_age_secs`) for that.
//!
//! Base is the post-loss equity (owner decision, RISK-59-3): the snapshot's equity already
//! includes today's loss, so the cap shrinks as the loss grows. This is stricter than the
//! nominal percentage: at the shipped 3% / 6% limits and Rs 1,00,000 the real triggers are about
//! -2.91% (daily) and -5.65% (weekly) of starting cash, never looser.

use crate::classify::{Action, Classification};
use crate::config::RuleCtx;
use crate::money_fmt::{pct, rupees};
use crate::sizing::{SizingError, sizing_equity};
use crate::snapshot::PortfolioSnapshot;
use crate::types::Tri;
use crate::verdict::{RuleId, RuleResult};
use chrono::{Datelike, Duration, NaiveDate};
use chrono_tz::Asia::Kolkata;
use rust_decimal::Decimal;

const OVERFLOW_FIX: &str = "check the account figures, they are out of range";

fn is_entry(cls: &Classification) -> bool {
    cls.action == Action::Entry && cls.entry_qty > 0
}

fn missing(rule: RuleId, limit: &str, why: &str, remedy: &str) -> RuleResult {
    RuleResult::input_missing(
        rule,
        "cannot confirm",
        limit,
        &format!("Cannot confirm {why}. When unsure, the app says no. To pass: {remedy}."),
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
    let Tri::Known(pnl) = pnl else {
        return missing(
            rule,
            &limit,
            &format!("{period}'s profit and loss"),
            "refresh the profit and loss figures from the journal",
        );
    };
    let equity = match sizing_equity(snap, &ctx.limits) {
        Ok(e) => e,
        Err(SizingError::ZeroEquity) => {
            return missing(
                rule,
                &limit,
                "your account value (it is zero or negative)",
                "add capital; no new entries are possible with no capital left",
            );
        }
        Err(SizingError::InputUnknown(what)) => {
            return missing(
                rule,
                &limit,
                &format!("your account value ({what} is unknown)"),
                "refresh the account value (cash and open-position marks)",
            );
        }
        Err(_) => {
            return missing(
                rule,
                &limit,
                "your account value (the figures overflowed)",
                OVERFLOW_FIX,
            );
        }
    };
    let Some(cap) = cap_pct
        .checked_mul(equity)
        .and_then(|v| v.checked_div(Decimal::ONE_HUNDRED))
    else {
        return missing(
            rule,
            &limit,
            "the loss limit (the figures overflowed)",
            OVERFLOW_FIX,
        );
    };
    let Some(loss) = Decimal::ZERO.checked_sub(pnl) else {
        return missing(
            rule,
            &limit,
            "the loss figure (it overflowed)",
            OVERFLOW_FIX,
        );
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
        None => missing(
            rule,
            &limit,
            "the loss figures (they overflowed)",
            OVERFLOW_FIX,
        ),
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
    let monday = snap
        .as_of
        .known()
        .and_then(|t| monday_of(t.with_timezone(&Kolkata).date_naive()));
    match (monday, snap.week_start) {
        (Some(m), Tri::Known(start)) if m == start => {}
        _ => {
            return missing(
                RuleId::G19,
                &pct(cap),
                "this week's start (it must be this week's Monday in IST)",
                "refresh the snapshot so it carries the current time and this week's Monday",
            );
        }
    }
    check(RuleId::G19, ctx, snap, snap.week_pnl, cap, "this week")
}
