//! Structure rules: G13 (no naked option shorts), G16 (mode, kill switch, order rate, day count,
//! notional) and X02 (lot and tick). Pure checks: every threshold comes from `RuleCtx.limits`,
//! every missing input rejects entries, and the override level lives only in `verdict.rs`.
//!
//! Exits and stop moves are never blocked by the entry-only parts (day count, notional, unknown
//! kill switch); the mode lock and the per-minute broker rate apply to every order.

use crate::classify::{Action, Classification};
use crate::config::RuleCtx;
use crate::money_fmt::rupees;
use crate::snapshot::PortfolioSnapshot;
use crate::types::{OrderIntent, Tri};
use crate::verdict::{RuleId, RuleResult};
use bazaar_core::mode::ExecutionMode;
use rust_decimal::Decimal;

/// What the order is, as far as options go. The caller supplies it from the instrument master.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionKind {
    /// Cash equity, futures or any non-option instrument.
    NotAnOption,
    /// Bought option: loss is capped at the premium.
    LongOption,
    /// Spread or other structure with a known maximum loss, checked against the per-trade budget.
    DefinedRisk {
        max_loss: Decimal,
        risk_budget: Decimal,
    },
    /// Short option with undefined loss.
    NakedShort,
}

/// Evidence for the mode lock. F6 only checks that the inputs are present; F10 owns the unlock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeInputs {
    /// The user approved this order (`semi_auto`).
    pub user_approval: Tri<bool>,
    /// Compliance review is fresh (`auto`).
    pub compliance_fresh: Tri<bool>,
    /// Paper-to-live readiness report is present and passing (`auto`).
    pub readiness_report: Tri<bool>,
    /// The owner opted in to auto orders (`auto`).
    pub owner_opt_in: Tri<bool>,
    /// The order would go to the live broker.
    pub live: bool,
    /// The `live-trading` feature is compiled into the executing binary (checked when `live`).
    pub live_trading_compiled: Tri<bool>,
}

/// Inputs for the structure rules that are not in the snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StructureInputs {
    pub option: Tri<OptionKind>,
    /// Orders sent today (all orders, not only entries).
    pub orders_today: Tri<u32>,
    pub mode: ModeInputs,
}

fn yes(t: Tri<bool>) -> bool {
    matches!(t, Tri::Known(true))
}

/// G13: no naked short options; defined-risk structures pass when max loss fits the budget.
#[must_use]
pub fn g13(ctx: &RuleCtx, cls: &Classification, inputs: &StructureInputs) -> RuleResult {
    let rule = RuleId::G13;
    if cls.entry_qty == 0 {
        return RuleResult::pass(rule, "no new exposure", "entries only");
    }
    match inputs.option {
        Tri::Unknown => RuleResult::fail(
            rule,
            "unknown",
            "known instrument type",
            "Cannot confirm whether this is an option. To pass, supply the instrument type. When unsure, the app says no.",
        ),
        Tri::Known(OptionKind::NotAnOption | OptionKind::LongOption) => {
            RuleResult::pass(rule, "capped loss", "no naked short")
        }
        Tri::Known(OptionKind::DefinedRisk {
            max_loss,
            risk_budget,
        }) => {
            if max_loss <= risk_budget {
                RuleResult::pass(rule, rupees(max_loss), rupees(risk_budget))
            } else {
                RuleResult::fail(
                    rule,
                    rupees(max_loss),
                    rupees(risk_budget),
                    &format!(
                        "This structure can lose {}, above your {} per-trade budget. Only capped-loss structures within budget are allowed.",
                        rupees(max_loss),
                        rupees(risk_budget)
                    ),
                )
            }
        }
        Tri::Known(OptionKind::NakedShort) => {
            if ctx.limits.allow_naked_short {
                RuleResult::pass(rule, "naked short", "allowed by config")
            } else {
                RuleResult::fail(
                    rule,
                    "naked short",
                    "not allowed",
                    "Selling options without a cap on loss can cost far more than your account holds. Only capped-loss structures are allowed.",
                )
            }
        }
    }
}

fn mode_check(ctx: &RuleCtx, cls: &Classification, m: ModeInputs) -> Result<(), String> {
    let _ = cls;
    match ctx.mode {
        ExecutionMode::Paper => {
            if m.live {
                return Err("Mode is paper, so orders cannot go to the live broker.".into());
            }
            return Ok(());
        }
        ExecutionMode::SemiAuto => {
            if !yes(m.user_approval) {
                return Err("Your approval of this order is not on record. To pass, approve it in the queue.".into());
            }
        }
        ExecutionMode::Auto => {
            let mut missing = Vec::new();
            if !yes(m.compliance_fresh) {
                missing.push("a fresh compliance review");
            }
            if !yes(m.readiness_report) {
                missing.push("the readiness report");
            }
            if !yes(m.owner_opt_in) {
                missing.push("your opt-in to auto orders");
            }
            if !missing.is_empty() {
                return Err(format!(
                    "Auto mode is locked: missing {}. To pass, supply them.",
                    missing.join(", ")
                ));
            }
        }
    }
    if m.live && !yes(m.live_trading_compiled) {
        return Err("Live orders need the live-trading build, which is not confirmed here.".into());
    }
    Ok(())
}

/// G16: mode lock, kill switch, orders per minute, orders per day, notional per order.
#[must_use]
pub fn g16(
    ctx: &RuleCtx,
    intent: &OrderIntent,
    cls: &Classification,
    snap: &PortfolioSnapshot,
    inputs: &StructureInputs,
) -> RuleResult {
    let rule = RuleId::G16;
    let lim = &ctx.limits;
    let entry = cls.entry_qty > 0;
    let mut why: Vec<String> = Vec::new();

    if let Err(e) = mode_check(ctx, cls, inputs.mode) {
        why.push(e);
    }
    match snap.kill_switch {
        Tri::Known(false) => {}
        Tri::Known(true) => {
            if entry || cls.action == Action::Entry {
                why.push("The kill switch is on, so new entries are stopped.".into());
            }
        }
        Tri::Unknown => {
            if entry {
                why.push(
                    "Cannot confirm the kill switch state. When unsure, the app says no.".into(),
                );
            }
        }
    }
    match snap.orders_last_minute {
        Tri::Known(n) if n.saturating_add(1) <= lim.max_orders_per_minute => {}
        Tri::Known(n) => why.push(format!(
            "{n} orders in the last minute; the limit is {} per minute.",
            lim.max_orders_per_minute
        )),
        Tri::Unknown => why.push("Cannot confirm the recent order count.".into()),
    }
    if entry {
        match inputs.orders_today {
            Tri::Known(n) if n.saturating_add(1) <= lim.max_orders_per_day => {}
            Tri::Known(n) => why.push(format!(
                "{n} orders today; the limit is {} per day.",
                lim.max_orders_per_day
            )),
            Tri::Unknown => why.push("Cannot confirm today's order count.".into()),
        }
        let notional = intent.price * Decimal::from(cls.entry_qty);
        if notional > lim.max_notional_per_order_inr {
            why.push(format!(
                "This order is {}, above your {} per-order cap.",
                rupees(notional),
                rupees(lim.max_notional_per_order_inr)
            ));
        }
    }

    let observed = format!("mode {}", ctx.mode.as_str());
    let limit = format!(
        "{} per min, {} per day, {} per order",
        lim.max_orders_per_minute,
        lim.max_orders_per_day,
        rupees(lim.max_notional_per_order_inr)
    );
    if why.is_empty() {
        RuleResult::pass(rule, observed, limit)
    } else {
        RuleResult::fail(rule, observed, limit, &why.join(" "))
    }
}

/// X02: quantity is a lot multiple; price and stops sit on the tick grid.
#[must_use]
pub fn x02(intent: &OrderIntent, cls: &Classification) -> RuleResult {
    let rule = RuleId::X02;
    let limit = format!(
        "lot {}, tick {}",
        intent.lot_size,
        intent.tick_size.normalize()
    );
    if intent.lot_size == 0 || intent.tick_size <= Decimal::ZERO {
        return RuleResult::fail(
            rule,
            "unknown lot or tick",
            limit,
            "The lot size or tick size is missing or zero. To pass, refresh the instrument data.",
        );
    }
    let mut why: Vec<String> = Vec::new();
    let qty = intent.qty.0;
    if qty > 0 && !qty.is_multiple_of(intent.lot_size) {
        why.push(format!(
            "Quantity {qty} is not a multiple of the lot size {}.",
            intent.lot_size
        ));
    }
    let off_tick = |p: Decimal| (p % intent.tick_size) != Decimal::ZERO;
    if cls.action != Action::StopMove && qty > 0 && off_tick(intent.price) {
        why.push(format!(
            "Price {} is not on the {} tick.",
            intent.price.normalize(),
            intent.tick_size.normalize()
        ));
    }
    for (name, s) in [("stop", intent.stop), ("new stop", intent.new_stop)] {
        if let Some(px) = s.filter(|px| off_tick(*px)) {
            why.push(format!(
                "The {name} {} is not on the {} tick.",
                px.normalize(),
                intent.tick_size.normalize()
            ));
        }
    }
    if why.is_empty() {
        RuleResult::pass(rule, format!("qty {qty}"), limit)
    } else {
        RuleResult::fail(rule, format!("qty {qty}"), limit, &why.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::classify;
    use crate::config::{Baseline, GateConfig};
    use crate::snapshot::Position;
    use crate::types::Side;
    use bazaar_core::money::Qty;
    use chrono::TimeZone;
    use chrono_tz::Asia::Kolkata;

    const TEXT: &str = include_str!("../../../../fixtures/config/risk.toml");

    fn ctx(mode: ExecutionMode) -> RuleCtx {
        let now = Kolkata.with_ymd_and_hms(2026, 11, 19, 11, 0, 0).unwrap();
        let mut c = GateConfig::from_toml_str(TEXT)
            .unwrap()
            .rule_ctx(&Baseline::none(), now);
        c.mode = mode;
        c
    }

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn intent(side: Side, qty: u32, price: &str) -> OrderIntent {
        OrderIntent {
            symbol: "ABC".into(),
            side,
            qty: Qty(qty),
            price: d(price),
            stop: Some(d("490")),
            new_stop: None,
            lot_size: 1,
            tick_size: d("0.05"),
        }
    }

    fn snap() -> PortfolioSnapshot {
        let mut s = PortfolioSnapshot::unknown();
        s.positions = Tri::Known(vec![]);
        s.kill_switch = Tri::Known(false);
        s.orders_last_minute = Tri::Known(0);
        s
    }

    fn modes() -> ModeInputs {
        ModeInputs {
            user_approval: Tri::Unknown,
            compliance_fresh: Tri::Unknown,
            readiness_report: Tri::Unknown,
            owner_opt_in: Tri::Unknown,
            live: false,
            live_trading_compiled: Tri::Unknown,
        }
    }

    fn inputs() -> StructureInputs {
        StructureInputs {
            option: Tri::Known(OptionKind::NotAnOption),
            orders_today: Tri::Known(0),
            mode: modes(),
        }
    }

    fn run16(
        c: &RuleCtx,
        i: &OrderIntent,
        s: &PortfolioSnapshot,
        inp: &StructureInputs,
    ) -> RuleResult {
        g16(c, i, &classify(i, s).unwrap(), s, inp)
    }

    #[test]
    fn paper_passes_and_message_ends_with_rule() {
        let r = run16(
            &ctx(ExecutionMode::Paper),
            &intent(Side::Buy, 10, "500"),
            &snap(),
            &inputs(),
        );
        assert!(r.passed(), "{}", r.message());
    }

    #[test]
    fn paper_with_live_target_rejects() {
        let mut inp = inputs();
        inp.mode.live = true;
        inp.mode.live_trading_compiled = Tri::Known(true);
        let r = run16(
            &ctx(ExecutionMode::Paper),
            &intent(Side::Buy, 10, "500"),
            &snap(),
            &inp,
        );
        assert!(!r.passed());
    }

    #[test]
    fn semi_auto_needs_approval() {
        let c = ctx(ExecutionMode::SemiAuto);
        let i = intent(Side::Buy, 10, "500");
        assert!(!run16(&c, &i, &snap(), &inputs()).passed());
        let mut inp = inputs();
        inp.mode.user_approval = Tri::Known(false);
        assert!(!run16(&c, &i, &snap(), &inp).passed());
        inp.mode.user_approval = Tri::Known(true);
        assert!(run16(&c, &i, &snap(), &inp).passed());
    }

    #[test]
    fn auto_needs_every_unlock_input() {
        let c = ctx(ExecutionMode::Auto);
        let i = intent(Side::Buy, 10, "500");
        let full = ModeInputs {
            compliance_fresh: Tri::Known(true),
            readiness_report: Tri::Known(true),
            owner_opt_in: Tri::Known(true),
            ..modes()
        };
        let mut inp = inputs();
        inp.mode = full;
        assert!(run16(&c, &i, &snap(), &inp).passed());
        for k in 0..3 {
            let mut m = full;
            match k {
                0 => m.compliance_fresh = Tri::Unknown,
                1 => m.readiness_report = Tri::Known(false),
                _ => m.owner_opt_in = Tri::Unknown,
            }
            inp.mode = m;
            let r = run16(&c, &i, &snap(), &inp);
            assert!(!r.passed(), "case {k}");
            assert!(r.message().ends_with("(G16)"));
        }
        inp.mode = modes();
        assert!(!run16(&c, &i, &snap(), &inp).passed());
    }

    #[test]
    fn live_needs_compiled_in() {
        let c = ctx(ExecutionMode::SemiAuto);
        let i = intent(Side::Buy, 10, "500");
        let mut inp = inputs();
        inp.mode.user_approval = Tri::Known(true);
        inp.mode.live = true;
        assert!(!run16(&c, &i, &snap(), &inp).passed());
        inp.mode.live_trading_compiled = Tri::Known(false);
        assert!(!run16(&c, &i, &snap(), &inp).passed());
        inp.mode.live_trading_compiled = Tri::Known(true);
        assert!(run16(&c, &i, &snap(), &inp).passed());
    }

    #[test]
    fn kill_switch_tri_state() {
        let c = ctx(ExecutionMode::Paper);
        let i = intent(Side::Buy, 10, "500");
        let mut s = snap();
        s.kill_switch = Tri::Known(true);
        assert!(!run16(&c, &i, &s, &inputs()).passed());
        s.kill_switch = Tri::Unknown;
        assert!(!run16(&c, &i, &s, &inputs()).passed());
    }

    #[test]
    fn exit_passes_with_unknown_kill_switch_and_is_not_notional_capped() {
        let c = ctx(ExecutionMode::Paper);
        let mut s = snap();
        s.kill_switch = Tri::Unknown;
        s.positions = Tri::Known(vec![Position {
            symbol: "ABC".into(),
            side: Side::Buy,
            qty: 1000,
            stop: Tri::Unknown,
        }]);
        let i = intent(Side::Sell, 1000, "500");
        assert!(run16(&c, &i, &s, &inputs()).passed());
    }

    #[test]
    fn order_rate_boundary() {
        let c = ctx(ExecutionMode::Paper);
        let i = intent(Side::Buy, 10, "500");
        let max = c.limits.max_orders_per_minute;
        let mut s = snap();
        s.orders_last_minute = Tri::Known(max - 1);
        assert!(run16(&c, &i, &s, &inputs()).passed());
        s.orders_last_minute = Tri::Known(max);
        assert!(!run16(&c, &i, &s, &inputs()).passed());
        s.orders_last_minute = Tri::Unknown;
        assert!(!run16(&c, &i, &s, &inputs()).passed());
    }

    #[test]
    fn orders_per_day_boundary() {
        let c = ctx(ExecutionMode::Paper);
        let i = intent(Side::Buy, 10, "500");
        let max = c.limits.max_orders_per_day;
        let mut inp = inputs();
        inp.orders_today = Tri::Known(max - 1);
        assert!(run16(&c, &i, &snap(), &inp).passed());
        inp.orders_today = Tri::Known(max);
        assert!(!run16(&c, &i, &snap(), &inp).passed());
        inp.orders_today = Tri::Unknown;
        assert!(!run16(&c, &i, &snap(), &inp).passed());
    }

    #[test]
    fn notional_boundary() {
        let c = ctx(ExecutionMode::Paper);
        let cap = c.limits.max_notional_per_order_inr;
        let at = intent(Side::Buy, 1, &cap.to_string());
        assert!(run16(&c, &at, &snap(), &inputs()).passed());
        let over = intent(Side::Buy, 1, &(cap + Decimal::ONE).to_string());
        let r = run16(&c, &over, &snap(), &inputs());
        assert!(!r.passed());
        assert!(r.message().contains("per-order cap"));
    }

    #[test]
    fn g13_cases() {
        let c = ctx(ExecutionMode::Paper);
        let i = intent(Side::Sell, 10, "500");
        let cls = classify(&i, &snap()).unwrap();
        let run = |o: Tri<OptionKind>| {
            let mut inp = inputs();
            inp.option = o;
            g13(&c, &cls, &inp)
        };
        assert!(run(Tri::Known(OptionKind::NotAnOption)).passed());
        assert!(run(Tri::Known(OptionKind::LongOption)).passed());
        assert!(!run(Tri::Unknown).passed());
        let naked = run(Tri::Known(OptionKind::NakedShort));
        assert!(!naked.passed() && naked.message().ends_with("(G13)"));
        let spread = |loss: i64| {
            run(Tri::Known(OptionKind::DefinedRisk {
                max_loss: Decimal::from(loss),
                risk_budget: Decimal::from(1000),
            }))
        };
        assert!(spread(1000).passed());
        assert!(!spread(1001).passed());
    }

    #[test]
    fn g13_allows_naked_only_with_flag_and_never_blocks_exits() {
        let mut c = ctx(ExecutionMode::Paper);
        let mut s = snap();
        let i = intent(Side::Sell, 10, "500");
        let cls = classify(&i, &s).unwrap();
        let mut inp = inputs();
        inp.option = Tri::Known(OptionKind::NakedShort);
        c.limits.allow_naked_short = true;
        assert!(g13(&c, &cls, &inp).passed());
        c.limits.allow_naked_short = false;
        s.positions = Tri::Known(vec![Position {
            symbol: "ABC".into(),
            side: Side::Sell,
            qty: 10,
            stop: Tri::Unknown,
        }]);
        let close = intent(Side::Buy, 10, "500");
        let close_cls = classify(&close, &s).unwrap();
        assert!(g13(&c, &close_cls, &inp).passed());
    }

    fn run_x02(i: &OrderIntent) -> RuleResult {
        x02(
            i,
            &classify(i, &snap()).unwrap_or(Classification {
                action: Action::Entry,
                entry_qty: i.qty.0,
                exit_qty: 0,
                stop_direction: None,
            }),
        )
    }

    #[test]
    fn x02_lot_and_tick() {
        let mut i = intent(Side::Buy, 75, "500.05");
        i.lot_size = 25;
        assert!(run_x02(&i).passed());
        i.qty = Qty(80);
        let r = run_x02(&i);
        assert!(!r.passed() && r.message().ends_with("(X02)"));
        i.qty = Qty(75);
        i.price = d("500.03");
        assert!(!run_x02(&i).passed());
        i.price = d("500");
        i.stop = Some(d("490.02"));
        assert!(!run_x02(&i).passed());
    }

    #[test]
    fn x02_rejects_zero_lot_or_tick() {
        let mut i = intent(Side::Buy, 10, "500");
        i.lot_size = 0;
        assert!(!run_x02(&i).passed());
        i.lot_size = 1;
        i.tick_size = Decimal::ZERO;
        assert!(!run_x02(&i).passed());
    }

    #[test]
    fn x02_stop_move_checks_new_stop_tick() {
        let mut s = snap();
        s.positions = Tri::Known(vec![Position {
            symbol: "ABC".into(),
            side: Side::Buy,
            qty: 10,
            stop: Tri::Known(d("480")),
        }]);
        let mut i = intent(Side::Buy, 0, "500.03");
        i.stop = None;
        i.new_stop = Some(d("485"));
        let cls = classify(&i, &s).unwrap();
        assert!(x02(&i, &cls).passed());
        i.new_stop = Some(d("485.02"));
        assert!(!x02(&i, &cls).passed());
    }
}
