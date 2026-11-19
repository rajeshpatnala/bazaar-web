//! Input rules: G01 (evidence bar), G06 (F&O ban list), G07 (event days), G15 (data freshness),
//! X01 (order session open) and X03 (cash or margin). The data comes from F3/F9/F10 through
//! [`DataInputs`]; until they supply it every field is `Unknown` and the rule rejects with what
//! would make it pass. Thresholds come from `RuleCtx.limits`; override levels live in `verdict.rs`.
//!
//! Entry-only: G01, G06, G07, X03 and the quote age of G15. A stale snapshot (G15) and a closed
//! session (X01) block every order: a stale snapshot can misclassify an exit, and the broker
//! rejects orders outside the session.

use crate::classify::Classification;
use crate::config::RuleCtx;
use crate::money_fmt::rupees;
use crate::snapshot::PortfolioSnapshot;
use crate::types::Tri;
use crate::verdict::{RuleId, RuleResult};
use bazaar_core::mode::ExecutionMode;
use chrono::DateTime;
use chrono_tz::Tz;
use rust_decimal::Decimal;

/// Cost-adjusted out-of-sample evidence for the strategy on this instrument and segment (F9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvidenceInputs {
    pub oos_trades: Tri<u32>,
    pub profit_factor: Tri<Decimal>,
    pub expectancy_r: Tri<Decimal>,
    pub walk_forward_pass: Tri<bool>,
}

/// Today's F&O ban list as fetched from the exchange (F3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BanList {
    pub banned_underlyings: Vec<String>,
    pub fetched_at: DateTime<Tz>,
}

/// Event-calendar answers for this order's underlying today (F3 events file).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventFlags {
    /// Results day or the session before it.
    pub results_window: Tri<bool>,
    /// RBI policy day or Budget day.
    pub policy_day: Tri<bool>,
    /// The order is an option on its own expiry day.
    pub option_on_own_expiry: Tri<bool>,
}

/// Cash or margin for the order (F9/F10): what it needs and what is available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarginInputs {
    /// Margin or cash this order needs, for the quantity the gate sized.
    pub required: Tri<Decimal>,
    pub available: Tri<Decimal>,
}

/// Everything these rules read that is not in the snapshot. No `Default`: build every field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataInputs {
    pub evidence: EvidenceInputs,
    pub ban_list: Tri<BanList>,
    /// `Known(Some(underlying))` for a stock F&O order; `Known(None)` for an index or non-F&O
    /// order (the ban list does not apply); `Unknown` rejects.
    pub stock_fno_underlying: Tri<Option<String>>,
    pub events: EventFlags,
    /// Time of the quote the order price rests on.
    pub quote_at: Tri<DateTime<Tz>>,
    /// The exchange is in an order-accepting session now (F3 calendar).
    pub session_open: Tri<bool>,
    pub margin: MarginInputs,
}

fn cannot(rule: RuleId, observed: &str, limit: &str, what: &str, to_pass: &str) -> RuleResult {
    RuleResult::fail(
        rule,
        observed,
        limit,
        &format!("Cannot confirm {what}. To pass, {to_pass}. When unsure, the app says no."),
    )
}

/// Whole seconds from `then` to `now`; `None` when `then` is in the future (never trusted).
fn age_secs(now: DateTime<Tz>, then: DateTime<Tz>) -> Option<i64> {
    let s = now.signed_duration_since(then).num_seconds();
    (s >= 0).then_some(s)
}

fn done(rule: RuleId, observed: String, limit: String, why: &[String]) -> RuleResult {
    if why.is_empty() {
        RuleResult::pass(rule, observed, limit)
    } else {
        RuleResult::fail(rule, observed, limit, &why.join(" "))
    }
}

fn no_exposure(rule: RuleId) -> RuleResult {
    RuleResult::pass(rule, "no new exposure", "entries only")
}

/// G01: enough out-of-sample, cost-adjusted evidence. Entries only. Override level Paper: the
/// override engine allows it in `paper` mode only; here the message says so.
#[must_use]
pub fn g01(ctx: &RuleCtx, cls: &Classification, inputs: &DataInputs) -> RuleResult {
    let rule = RuleId::G01;
    if cls.entry_qty == 0 {
        return no_exposure(rule);
    }
    let (lim, ev) = (&ctx.limits, &inputs.evidence);
    let limit = format!(
        "{}+ trades, PF {}, {} R, walk-forward {}",
        lim.min_oos_trades,
        lim.min_profit_factor.normalize(),
        lim.min_expectancy_r.normalize(),
        if lim.require_walk_forward_pass {
            "pass"
        } else {
            "not needed"
        }
    );
    let (Tri::Known(n), Tri::Known(pf), Tri::Known(ex)) =
        (ev.oos_trades, ev.profit_factor, ev.expectancy_r)
    else {
        return cannot(
            rule,
            "unknown",
            &limit,
            "the evidence for this strategy",
            "supply the out-of-sample trade count, profit factor and expectancy after costs",
        );
    };
    let wf = ev.walk_forward_pass;
    if lim.require_walk_forward_pass && wf.is_unknown() {
        return cannot(
            rule,
            "unknown",
            &limit,
            "the walk-forward result",
            "supply the walk-forward test result",
        );
    }
    let mut why = Vec::new();
    if n < lim.min_oos_trades {
        why.push(format!(
            "Only {n} out-of-sample trades; at least {} are needed.",
            lim.min_oos_trades
        ));
    }
    if pf < lim.min_profit_factor {
        why.push(format!(
            "Profit factor {} is below {}.",
            pf.normalize(),
            lim.min_profit_factor.normalize()
        ));
    }
    if ex < lim.min_expectancy_r {
        why.push(format!(
            "Expectancy {} R is below {} R.",
            ex.normalize(),
            lim.min_expectancy_r.normalize()
        ));
    }
    if lim.require_walk_forward_pass && wf != Tri::Known(true) {
        why.push("The walk-forward test did not pass.".into());
    }
    if !why.is_empty() && ctx.mode == ExecutionMode::Paper {
        why.push("In paper mode you may continue, tagged exploratory.".into());
    }
    done(
        rule,
        format!("{n} trades, PF {}, {} R", pf.normalize(), ex.normalize()),
        limit,
        &why,
    )
}

/// G06: no new stock F&O position in a banned underlying; a list older than
/// `ban_list_max_age_hours` (or dated in the future) is not trusted. Entries only.
#[must_use]
pub fn g06(ctx: &RuleCtx, cls: &Classification, inputs: &DataInputs) -> RuleResult {
    let rule = RuleId::G06;
    let lim = &ctx.limits;
    let limit = format!(
        "not banned, list under {} h old",
        lim.ban_list_max_age_hours
    );
    if cls.entry_qty == 0 {
        return no_exposure(rule);
    }
    if !lim.block_ban_list {
        return RuleResult::pass(rule, "ban check off", limit);
    }
    let Tri::Known(scope) = &inputs.stock_fno_underlying else {
        return cannot(
            rule,
            "unknown",
            &limit,
            "whether this is a stock F&O order",
            "supply the instrument's underlying",
        );
    };
    let Some(under) = scope else {
        return RuleResult::pass(rule, "index or non-F&O", limit);
    };
    let Tri::Known(list) = &inputs.ban_list else {
        return cannot(
            rule,
            "no ban list",
            &limit,
            "today's F&O ban list",
            "load the exchange ban list",
        );
    };
    let max_age = i64::from(lim.ban_list_max_age_hours).saturating_mul(3600);
    if !matches!(age_secs(ctx.now, list.fetched_at), Some(a) if a <= max_age) {
        return cannot(
            rule,
            "old ban list",
            &limit,
            "the F&O ban list, which is out of date",
            "refresh the exchange ban list",
        );
    }
    if list
        .banned_underlyings
        .iter()
        .any(|b| b.eq_ignore_ascii_case(under))
    {
        return RuleResult::fail(
            rule,
            format!("{under} is banned"),
            limit,
            &format!(
                "{under} is on today's F&O ban list, so no new position is allowed. Exits are still allowed."
            ),
        );
    }
    RuleResult::pass(rule, format!("{under} not banned"), limit)
}

/// G07: results day (and the session before), policy day, and new option orders on their own
/// expiry day after `expiry_day_no_new_after_ist`. Entries only; a needed flag that is
/// `Unknown` rejects.
#[must_use]
pub fn g07(ctx: &RuleCtx, cls: &Classification, inputs: &DataInputs) -> RuleResult {
    let rule = RuleId::G07;
    let lim = &ctx.limits;
    let cutoff = lim.expiry_day_no_new_after_ist;
    let limit = format!(
        "no event day; no new option orders on expiry day after {}",
        cutoff.format("%H:%M")
    );
    if cls.entry_qty == 0 {
        return no_exposure(rule);
    }
    let ev = &inputs.events;
    let needed = [
        (
            lim.block_results_days,
            ev.results_window,
            "the results calendar",
        ),
        (
            lim.block_policy_days,
            ev.policy_day,
            "the policy-day calendar",
        ),
        (
            true,
            ev.option_on_own_expiry,
            "whether this option expires today",
        ),
    ];
    let unknown: Vec<&str> = needed
        .iter()
        .filter(|(on, f, _)| *on && f.is_unknown())
        .map(|(_, _, n)| *n)
        .collect();
    if !unknown.is_empty() {
        return cannot(
            rule,
            "unknown",
            &limit,
            &unknown.join(", "),
            "load the events calendar",
        );
    }
    let hit = |on: bool, f: Tri<bool>| on && f == Tri::Known(true);
    let mut why = Vec::new();
    if hit(lim.block_results_days, ev.results_window) {
        why.push("Results are due for this stock today or the next session.".to_owned());
    }
    if hit(lim.block_policy_days, ev.policy_day) {
        why.push("Today is an RBI policy or Budget day.".to_owned());
    }
    if hit(true, ev.option_on_own_expiry) && ctx.now.time() >= cutoff {
        why.push(format!(
            "New option orders on their own expiry day stop at {}.",
            cutoff.format("%H:%M")
        ));
    }
    done(
        rule,
        if why.is_empty() {
            "no event".into()
        } else {
            "event day".into()
        },
        limit,
        &why,
    )
}

/// G15: the quote (entries) and the snapshot (every order) are within the freshness limits.
#[must_use]
pub fn g15(
    ctx: &RuleCtx,
    cls: &Classification,
    snap: &PortfolioSnapshot,
    inputs: &DataInputs,
) -> RuleResult {
    let rule = RuleId::G15;
    let lim = &ctx.limits;
    let limit = format!(
        "quote {} s, snapshot {} s",
        lim.max_quote_age_secs, lim.max_snapshot_age_secs
    );
    let mut why = Vec::new();
    let mut seen = Vec::new();
    match snap.as_of {
        Tri::Known(t) => match age_secs(ctx.now, t) {
            Some(a) if a <= i64::from(lim.max_snapshot_age_secs) => {
                seen.push(format!("snapshot {a} s"));
            }
            _ => {
                why.push("Your account figures are out of date. To pass, refresh them.".to_owned());
            }
        },
        Tri::Unknown => why.push(
            "Cannot confirm when your account figures were taken. When unsure, the app says no."
                .into(),
        ),
    }
    if cls.entry_qty > 0 {
        match inputs.quote_at {
            Tri::Known(t) => match age_secs(ctx.now, t) {
                Some(a) if a <= i64::from(lim.max_quote_age_secs) => seen.push(format!("quote {a} s")),
                _ => why.push(format!("The price quote is older than {} seconds. To pass, wait for a fresh quote.", lim.max_quote_age_secs)),
            },
            Tri::Unknown => why.push("Cannot confirm the age of the price quote. To pass, supply the quote time. When unsure, the app says no.".into()),
        }
    }
    done(rule, seen.join(", "), limit, &why)
}

/// X01: the exchange is accepting orders now. Every order.
#[must_use]
pub fn x01(inputs: &DataInputs) -> RuleResult {
    let rule = RuleId::X01;
    match inputs.session_open {
        Tri::Known(true) => RuleResult::pass(rule, "session open", "order session"),
        Tri::Known(false) => RuleResult::fail(
            rule,
            "session closed",
            "order session",
            "The market is not accepting orders now. To pass, place the order in the trading session.",
        ),
        Tri::Unknown => cannot(
            rule,
            "unknown",
            "order session",
            "that the market is open",
            "supply the exchange calendar state",
        ),
    }
}

/// X03: the order's cash or margin need is within what is available. Entries only. The need
/// must be computed for the quantity the gate sized, so a sized order is judged as sized.
#[must_use]
pub fn x03(cls: &Classification, inputs: &DataInputs) -> RuleResult {
    let rule = RuleId::X03;
    if cls.entry_qty == 0 {
        return no_exposure(rule);
    }
    let (Tri::Known(req), Tri::Known(avail)) = (inputs.margin.required, inputs.margin.available)
    else {
        return cannot(
            rule,
            "unknown",
            "available cash",
            "your cash or margin for this order",
            "supply the margin needed and the cash available",
        );
    };
    if req < Decimal::ZERO || avail < Decimal::ZERO {
        return cannot(
            rule,
            &rupees(req),
            &rupees(avail),
            "the margin figures, which are negative",
            "recompute them",
        );
    }
    if req <= avail {
        RuleResult::pass(rule, rupees(req), rupees(avail))
    } else {
        RuleResult::fail(
            rule,
            rupees(req),
            rupees(avail),
            &format!(
                "This order needs {} but only {} is available. To pass, reduce the size or add cash.",
                rupees(req),
                rupees(avail)
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::Action;
    use crate::config::{Baseline, GateConfig};
    use crate::sizing::{SizingInput, size};
    use crate::types::Side;
    use chrono::{Duration, TimeZone};
    use chrono_tz::Asia::Kolkata;

    const TEXT: &str = include_str!("../../../../fixtures/config/risk.toml");

    fn at(h: u32, m: u32) -> DateTime<Tz> {
        Kolkata.with_ymd_and_hms(2026, 11, 19, h, m, 0).unwrap()
    }

    fn ctx(mode: ExecutionMode, now: DateTime<Tz>) -> RuleCtx {
        let mut c = GateConfig::from_toml_str(TEXT)
            .unwrap()
            .rule_ctx(&Baseline::none(), now);
        c.mode = mode;
        c.limits.min_oos_trades = 30;
        c.limits.min_profit_factor = d("1.3");
        c.limits.min_expectancy_r = d("0.1");
        c.limits.require_walk_forward_pass = true;
        c.limits.expiry_day_no_new_after_ist = chrono::NaiveTime::from_hms_opt(13, 30, 0).unwrap();
        c
    }

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn cls(entry_qty: u32) -> Classification {
        Classification {
            action: if entry_qty > 0 {
                Action::Entry
            } else {
                Action::Exit
            },
            entry_qty,
            exit_qty: u32::from(entry_qty == 0),
            stop_direction: None,
        }
    }

    fn good(now: DateTime<Tz>) -> DataInputs {
        DataInputs {
            evidence: EvidenceInputs {
                oos_trades: Tri::Known(30),
                profit_factor: Tri::Known(d("1.3")),
                expectancy_r: Tri::Known(d("0.1")),
                walk_forward_pass: Tri::Known(true),
            },
            ban_list: Tri::Known(BanList {
                banned_underlyings: vec!["XYZ".into()],
                fetched_at: now,
            }),
            stock_fno_underlying: Tri::Known(Some("ABC".into())),
            events: EventFlags {
                results_window: Tri::Known(false),
                policy_day: Tri::Known(false),
                option_on_own_expiry: Tri::Known(false),
            },
            quote_at: Tri::Known(now),
            session_open: Tri::Known(true),
            margin: MarginInputs {
                required: Tri::Known(d("25000")),
                available: Tri::Known(d("25000")),
            },
        }
    }

    fn snap(now: DateTime<Tz>) -> PortfolioSnapshot {
        let mut s = PortfolioSnapshot::unknown();
        s.as_of = Tri::Known(now);
        s
    }

    fn ends_with_id(r: &RuleResult) {
        assert!(
            r.message().ends_with(&format!("({})", r.rule())),
            "{}",
            r.message()
        );
    }

    #[test]
    fn g01_boundaries_and_each_failing_input() {
        let (now, c) = (at(11, 0), ctx(ExecutionMode::SemiAuto, at(11, 0)));
        assert!(g01(&c, &cls(5), &good(now)).passed());
        let cases: [fn(&mut EvidenceInputs); 4] = [
            |e| e.oos_trades = Tri::Known(29),
            |e| e.profit_factor = Tri::Known(d("1.29")),
            |e| e.expectancy_r = Tri::Known(d("0.09")),
            |e| e.walk_forward_pass = Tri::Known(false),
        ];
        for f in cases {
            let mut i = good(now);
            f(&mut i.evidence);
            let r = g01(&c, &cls(5), &i);
            assert!(!r.passed());
            ends_with_id(&r);
        }
    }

    #[test]
    fn g01_unknown_evidence_rejects_and_exits_pass() {
        let (now, c) = (at(11, 0), ctx(ExecutionMode::SemiAuto, at(11, 0)));
        for f in [
            (|e: &mut EvidenceInputs| e.oos_trades = Tri::Unknown) as fn(&mut EvidenceInputs),
            |e| e.profit_factor = Tri::Unknown,
            |e| e.expectancy_r = Tri::Unknown,
            |e| e.walk_forward_pass = Tri::Unknown,
        ] {
            let mut i = good(now);
            f(&mut i.evidence);
            let r = g01(&c, &cls(5), &i);
            assert!(
                !r.passed() && r.message().contains("To pass"),
                "{}",
                r.message()
            );
        }
        assert!(g01(&c, &cls(0), &good(now)).passed());
    }

    #[test]
    fn g01_mentions_exploratory_only_in_paper() {
        let now = at(11, 0);
        let mut i = good(now);
        i.evidence.oos_trades = Tri::Known(1);
        let paper = g01(&ctx(ExecutionMode::Paper, now), &cls(5), &i);
        let semi = g01(&ctx(ExecutionMode::SemiAuto, now), &cls(5), &i);
        assert!(!paper.passed() && paper.message().contains("exploratory"));
        assert!(!semi.passed() && !semi.message().contains("exploratory"));
    }

    #[test]
    fn g06_banned_unbanned_index_and_unknowns() {
        let (now, c) = (at(11, 0), ctx(ExecutionMode::Paper, at(11, 0)));
        assert!(g06(&c, &cls(5), &good(now)).passed());
        let mut i = good(now);
        i.stock_fno_underlying = Tri::Known(Some("xyz".into()));
        let r = g06(&c, &cls(5), &i);
        assert!(!r.passed() && r.message().contains("Exits are still allowed"));
        ends_with_id(&r);
        assert!(g06(&c, &cls(0), &i).passed());
        i.stock_fno_underlying = Tri::Known(None);
        assert!(g06(&c, &cls(5), &i).passed());
        i.stock_fno_underlying = Tri::Unknown;
        assert!(!g06(&c, &cls(5), &i).passed());
        i = good(now);
        i.ban_list = Tri::Unknown;
        assert!(!g06(&c, &cls(5), &i).passed());
    }

    #[test]
    fn g06_list_age_boundary_and_future_date() {
        let (now, c) = (at(11, 0), ctx(ExecutionMode::Paper, at(11, 0)));
        let max = Duration::hours(i64::from(c.limits.ban_list_max_age_hours));
        let with = |t: DateTime<Tz>| {
            let mut i = good(now);
            i.ban_list = Tri::Known(BanList {
                banned_underlyings: vec![],
                fetched_at: t,
            });
            g06(&c, &cls(5), &i)
        };
        assert!(with(now - max).passed());
        assert!(!with(now - max - Duration::seconds(1)).passed());
        assert!(!with(now + Duration::seconds(1)).passed());
    }

    #[test]
    fn g07_event_days_expiry_cutoff_and_unknowns() {
        let c = ctx(ExecutionMode::Paper, at(13, 29));
        let now = at(13, 29);
        assert!(g07(&c, &cls(5), &good(now)).passed());
        let mut i = good(now);
        i.events.results_window = Tri::Known(true);
        assert!(!g07(&c, &cls(5), &i).passed());
        assert!(g07(&c, &cls(0), &i).passed());
        i = good(now);
        i.events.policy_day = Tri::Known(true);
        assert!(!g07(&c, &cls(5), &i).passed());
        i = good(now);
        i.events.option_on_own_expiry = Tri::Known(true);
        assert!(g07(&c, &cls(5), &i).passed(), "13:29 is before the cutoff");
        let late = ctx(ExecutionMode::Paper, at(13, 30));
        let r = g07(&late, &cls(5), &i);
        assert!(!r.passed());
        ends_with_id(&r);
        for f in [
            (|e: &mut EventFlags| e.results_window = Tri::Unknown) as fn(&mut EventFlags),
            |e| e.policy_day = Tri::Unknown,
            |e| e.option_on_own_expiry = Tri::Unknown,
        ] {
            let mut i = good(now);
            f(&mut i.events);
            assert!(!g07(&c, &cls(5), &i).passed());
        }
    }

    #[test]
    fn g07_calendar_flags_not_needed_when_the_block_is_off() {
        let now = at(11, 0);
        let mut c = ctx(ExecutionMode::Paper, now);
        c.limits.block_results_days = false;
        c.limits.block_policy_days = false;
        let mut i = good(now);
        i.events.results_window = Tri::Unknown;
        i.events.policy_day = Tri::Known(true);
        assert!(g07(&c, &cls(5), &i).passed());
    }

    #[test]
    fn g15_quote_and_snapshot_boundaries() {
        let now = at(11, 0);
        let c = ctx(ExecutionMode::Paper, now);
        let q = i64::from(c.limits.max_quote_age_secs);
        let s = i64::from(c.limits.max_snapshot_age_secs);
        let run = |qa: i64, sa: i64, entry: u32| {
            let mut i = good(now);
            i.quote_at = Tri::Known(now - Duration::seconds(qa));
            g15(&c, &cls(entry), &snap(now - Duration::seconds(sa)), &i)
        };
        assert!(run(q, s, 5).passed());
        assert!(!run(q + 1, s, 5).passed());
        assert!(!run(q, s + 1, 5).passed());
        assert!(
            run(q + 100, s, 0).passed(),
            "a stale quote never traps an exit"
        );
        assert!(
            !run(0, s + 1, 0).passed(),
            "a stale snapshot blocks exits too"
        );
        assert!(
            !run(-1, 0, 5).passed(),
            "a quote from the future is not trusted"
        );
        ends_with_id(&run(q + 1, s, 5));
    }

    #[test]
    fn g15_unknown_times_reject() {
        let now = at(11, 0);
        let c = ctx(ExecutionMode::Paper, now);
        let mut i = good(now);
        i.quote_at = Tri::Unknown;
        assert!(!g15(&c, &cls(5), &snap(now), &i).passed());
        assert!(g15(&c, &cls(0), &snap(now), &i).passed());
        assert!(!g15(&c, &cls(0), &PortfolioSnapshot::unknown(), &good(now)).passed());
    }

    #[test]
    fn x01_session_states() {
        let mut i = good(at(11, 0));
        assert!(x01(&i).passed());
        i.session_open = Tri::Known(false);
        let r = x01(&i);
        assert!(!r.passed());
        ends_with_id(&r);
        i.session_open = Tri::Unknown;
        assert!(!x01(&i).passed());
    }

    #[test]
    fn x03_boundary_unknown_negative_and_exit() {
        let mut i = good(at(11, 0));
        assert!(x03(&cls(5), &i).passed());
        i.margin.required = Tri::Known(d("25000.01"));
        let r = x03(&cls(5), &i);
        assert!(
            !r.passed() && r.message().starts_with("This order needs Rs 25,000.01"),
            "{}",
            r.message()
        );
        assert!(x03(&cls(0), &i).passed());
        i.margin.required = Tri::Unknown;
        assert!(!x03(&cls(5), &i).passed());
        i.margin = MarginInputs {
            required: Tri::Known(d("1")),
            available: Tri::Unknown,
        };
        assert!(!x03(&cls(5), &i).passed());
        i.margin = MarginInputs {
            required: Tri::Known(d("-1")),
            available: Tri::Known(d("5")),
        };
        assert!(!x03(&cls(5), &i).passed());
    }

    #[test]
    fn a_sized_order_passes_x03_and_one_share_more_does_not() {
        let c = ctx(ExecutionMode::Paper, at(11, 0));
        let s = size(
            &c.limits,
            &SizingInput {
                equity: d("100000"),
                side: Side::Buy,
                entry: d("500"),
                stop: d("490"),
                lot_size: 1,
                tick_size: d("0.05"),
                buffer: Tri::Known(d("60")),
                overridden: false,
            },
        )
        .unwrap();
        let mut i = good(at(11, 0));
        i.margin = MarginInputs {
            required: Tri::Known(s.notional),
            available: Tri::Known(d("100000")),
        };
        assert!(x03(&cls(s.qty), &i).passed());
        i.margin.required = Tri::Known(s.notional + d("100000"));
        assert!(!x03(&cls(s.qty), &i).passed());
    }
}
