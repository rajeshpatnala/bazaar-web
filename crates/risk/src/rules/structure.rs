//! Structure rules: G13 (no naked option shorts), G16 (mode, kill switch, order rate, day count,
//! notional) and X02 (lot and tick). Pure checks: every threshold comes from `RuleCtx.limits`,
//! every missing input rejects entries, and the override level lives only in `verdict.rs`.
//!
//! Exits and stop moves are never blocked by the entry-only parts (day count, notional, kill
//! switch, lot multiple, unknown instrument data); the mode lock and the per-minute broker rate
//! apply to every order. Option entries are never sized by `size` (see [`is_sizable`]).

use crate::classify::{Action, Classification};
use crate::config::RuleCtx;
use crate::money_fmt::rupees;
use crate::sizing::sizing_equity;
use crate::snapshot::PortfolioSnapshot;
use crate::types::{OrderIntent, Side, Tri};
use crate::verdict::{RuleId, RuleResult};
use bazaar_core::instrument::{Instrument, InstrumentType};
use bazaar_core::mode::ExecutionMode;
use rust_decimal::Decimal;

/// What the order is, as far as options go. Built from the instrument master with
/// [`OptionKind::from_instrument`]; the gate also checks it against the order side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionKind {
    /// Cash equity or futures: not an option in the instrument master.
    NotAnOption,
    /// Bought option: loss is capped at the premium. A sell-side entry labelled this is a naked
    /// short and rejects.
    LongOption,
    /// Spread or other structure with a known maximum loss. `max_loss` is the whole-order worst
    /// case in rupees from the options payoff engine; the budget is derived in G13.
    DefinedRisk { max_loss: Decimal },
    /// Short option with undefined loss. Always rejects.
    NakedShort,
}

impl OptionKind {
    /// Classify an instrument-master row. `defined_max_loss` is the payoff engine's whole-order
    /// maximum loss when the order is a capped-loss structure, else `None`. An option can never
    /// come back as `NotAnOption`; a type the app does not know is `Unknown` (rejects).
    #[must_use]
    pub fn from_instrument(inst: &Instrument, defined_max_loss: Option<Decimal>) -> Tri<Self> {
        match &inst.instrument_type {
            InstrumentType::Equity | InstrumentType::Future => Tri::Known(Self::NotAnOption),
            InstrumentType::Call | InstrumentType::Put => Tri::Known(match defined_max_loss {
                Some(max_loss) => Self::DefinedRisk { max_loss },
                None => Self::LongOption,
            }),
            InstrumentType::Other(_) => Tri::Unknown,
        }
    }
}

/// True only for linear instruments `size` can handle. Option entries (long, spread or short)
/// are not sizable until options sizing exists; the gate rejects them rather than call `size`.
#[must_use]
pub const fn is_sizable(option: &Tri<OptionKind>) -> bool {
    matches!(option, Tri::Known(OptionKind::NotAnOption))
}

/// Evidence for the mode lock. F6 only checks that the inputs are present; F10 owns the unlock
/// (and must source `user_approval` bound to the intent digest, `live_trading_compiled` from
/// build info and the auto unlock inputs from the journal, never from request data).
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
    /// The order would go to the live broker; `Unknown` rejects.
    pub live: Tri<bool>,
    /// The `live-trading` feature is compiled into the executing binary (checked when `live`).
    pub live_trading_compiled: Tri<bool>,
}

/// Inputs for the structure rules that are not in the snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StructureInputs {
    pub option: Tri<OptionKind>,
    pub mode: ModeInputs,
}

fn yes(t: Tri<bool>) -> bool {
    matches!(t, Tri::Known(true))
}

fn naked_short_fail(rule: RuleId) -> RuleResult {
    RuleResult::fail(
        rule,
        "naked short",
        "not allowed",
        "Selling options without a cap on loss can cost far more than your account holds. Only capped-loss structures are allowed.",
    )
}

/// G13: no naked short options; defined-risk structures pass when max loss plus the charges
/// floor fits the per-trade budget (`max_risk_per_trade_pct` x sizing equity, as `size` uses).
/// Applies to entries, including the excess of a flip. `allow_naked_short` is never read.
#[must_use]
pub fn g13(
    ctx: &RuleCtx,
    intent: &OrderIntent,
    cls: &Classification,
    snap: &PortfolioSnapshot,
    inputs: &StructureInputs,
) -> RuleResult {
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
        Tri::Known(OptionKind::NotAnOption) => {
            RuleResult::pass(rule, "capped loss", "no naked short")
        }
        Tri::Known(OptionKind::LongOption) => {
            if intent.side == Side::Sell {
                naked_short_fail(rule)
            } else {
                RuleResult::pass(rule, "capped loss", "no naked short")
            }
        }
        Tri::Known(OptionKind::DefinedRisk { max_loss }) => g13_defined(ctx, snap, max_loss),
        Tri::Known(OptionKind::NakedShort) => naked_short_fail(rule),
    }
}

fn g13_defined(ctx: &RuleCtx, snap: &PortfolioSnapshot, max_loss: Decimal) -> RuleResult {
    let rule = RuleId::G13;
    let lim = &ctx.limits;
    if max_loss <= Decimal::ZERO {
        return RuleResult::fail(
            rule,
            rupees(max_loss),
            "max loss above 0",
            "The worst-case loss for this structure is missing or not above zero, so it cannot be checked. To pass, recompute it.",
        );
    }
    let budget = sizing_equity(snap, lim).ok().and_then(|eq| {
        eq.checked_mul(lim.max_risk_per_trade_pct)
            .and_then(|v| v.checked_div(Decimal::ONE_HUNDRED))
    });
    let Some(budget) = budget else {
        return RuleResult::fail(
            rule,
            rupees(max_loss),
            "unknown budget",
            "Cannot work out your per-trade budget from your account value. When unsure, the app says no.",
        );
    };
    match max_loss.checked_add(lim.min_charges_slippage_buffer) {
        Some(n) if n <= budget => RuleResult::pass(rule, rupees(max_loss), rupees(budget)),
        _ => RuleResult::fail(
            rule,
            rupees(max_loss),
            rupees(budget),
            &format!(
                "This structure can lose {} plus charges, above your {} per-trade budget. Only capped-loss structures within budget are allowed.",
                rupees(max_loss),
                rupees(budget)
            ),
        ),
    }
}

/// Why the mode check failed, as a type (never parsed from text). `ApprovalMissing` is produced
/// only in `semi_auto`, only when no approval was supplied (`Unknown`), and only when nothing
/// else in the mode check failed. An approval for another order (`Known(false)`) is `Other`.
enum ModeGap {
    ApprovalMissing(String),
    Other(String),
}

impl ModeGap {
    fn into_text(self) -> String {
        match self {
            Self::ApprovalMissing(t) | Self::Other(t) => t,
        }
    }
}

const APPROVAL_TEXT: &str =
    "Your approval of this order is not on record. To pass, approve it in the queue.";

fn mode_check(ctx: &RuleCtx, m: ModeInputs) -> Result<(), ModeGap> {
    let mut approval_missing = false;
    match ctx.mode {
        ExecutionMode::Paper => {}
        ExecutionMode::SemiAuto => {
            if !yes(m.user_approval) {
                if !matches!(m.user_approval, Tri::Unknown) {
                    return Err(ModeGap::Other(APPROVAL_TEXT.into()));
                }
                approval_missing = true;
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
                return Err(ModeGap::Other(format!(
                    "Auto mode is locked: missing {}. To pass, supply them.",
                    missing.join(", ")
                )));
            }
        }
    }
    let live_gap = match m.live {
        Tri::Unknown => Some("Cannot confirm whether this order goes to the live broker."),
        Tri::Known(true) if ctx.mode == ExecutionMode::Paper => {
            Some("Mode is paper, so orders cannot go to the live broker.")
        }
        Tri::Known(true) if !yes(m.live_trading_compiled) => {
            Some("Live orders need the live-trading build, which is not confirmed here.")
        }
        Tri::Known(_) => None,
    };
    match (approval_missing, live_gap) {
        (false, None) => Ok(()),
        (true, None) => Err(ModeGap::ApprovalMissing(APPROVAL_TEXT.into())),
        // Both gaps: name both reasons, and it is not "only the approval".
        (true, Some(t)) => Err(ModeGap::Other(format!("{APPROVAL_TEXT} {t}"))),
        (false, Some(t)) => Err(ModeGap::Other(t.into())),
    }
}

/// Entry-only checks of G16: orders per day and notional. Pushes the reasons it finds.
fn g16_entry_checks(
    ctx: &RuleCtx,
    intent: &OrderIntent,
    cls: &Classification,
    snap: &PortfolioSnapshot,
    notional_price: Decimal,
    why: &mut Vec<String>,
) {
    let lim = &ctx.limits;
    match snap.orders_today {
        Tri::Known(n) if n.saturating_add(1) <= lim.max_orders_per_day => {}
        Tri::Known(n) => why.push(format!(
            "{n} orders today; the limit is {} per day.",
            lim.max_orders_per_day
        )),
        Tri::Unknown => why.push("Cannot confirm today's order count.".into()),
    }
    if intent.price <= Decimal::ZERO || notional_price <= Decimal::ZERO {
        why.push("The order price must be above 0 to check the per-order cap.".into());
        return;
    }
    // Never below the order's own price: a lower reference would understate a Hard ceiling row.
    let reference = notional_price.max(intent.price);
    match reference.checked_mul(Decimal::from(cls.entry_qty)) {
        Some(n) if n <= lim.max_notional_per_order_inr => {}
        Some(n) => why.push(format!(
            "This order is {}, above your {} per-order cap.",
            rupees(n),
            rupees(lim.max_notional_per_order_inr)
        )),
        None => why.push("The order value is too large to check, above your per-order cap.".into()),
    }
}

/// G16: mode lock, kill switch, orders per minute, orders per day, notional per order.
///
/// `notional_price` is the reference price for exposure, computed once in the gate as
/// `max(intent.price, live price)` and shared with `size`; it is not re-derived here.
#[must_use]
pub fn g16(
    ctx: &RuleCtx,
    intent: &OrderIntent,
    cls: &Classification,
    snap: &PortfolioSnapshot,
    inputs: &StructureInputs,
    notional_price: Decimal,
) -> RuleResult {
    let rule = RuleId::G16;
    let lim = &ctx.limits;
    let entry = cls.entry_qty > 0;
    let mut why: Vec<String> = Vec::new();
    let mut approval_only = false;

    if let Err(gap) = mode_check(ctx, inputs.mode) {
        approval_only = matches!(gap, ModeGap::ApprovalMissing(_));
        why.push(gap.into_text());
    }
    // Exits always pass the kill switch (reduce-only); owner to confirm.
    match snap.kill_switch {
        Tri::Known(false) => {}
        Tri::Known(true) => {
            if entry {
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
        g16_entry_checks(ctx, intent, cls, snap, notional_price, &mut why);
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
    // `why.len() == 1`: the approval text must be the only reason (kill switch, limits, live gap add more).
    } else if approval_only && why.len() == 1 {
        RuleResult::g16_approval_pending(observed, limit, &why.join(" "))
    } else {
        RuleResult::fail(rule, observed, limit, &why.join(" "))
    }
}

/// X02: the entry quantity is a lot multiple; price and stops sit on the tick grid.
///
/// Exits stay closable: the lot multiple applies to `entry_qty` only (an odd-lot residual can be
/// sold in full; the entry part of a flip must be a lot multiple), and an exit with a missing lot
/// or tick passes (the broker validates; stale instrument data never traps a position). An
/// off-tick price on an exit still rejects when the tick is known.
#[must_use]
pub fn x02(intent: &OrderIntent, cls: &Classification) -> RuleResult {
    let rule = RuleId::X02;
    let limit = format!(
        "lot {}, tick {}",
        intent.lot_size,
        intent.tick_size.normalize()
    );
    let entry = cls.entry_qty > 0;
    let tick_known = intent.tick_size > Decimal::ZERO;
    if entry && (intent.lot_size == 0 || !tick_known) {
        return RuleResult::fail(
            rule,
            "unknown lot or tick",
            limit,
            "The lot size or tick size is missing or zero. To pass, refresh the instrument data.",
        );
    }
    let mut why: Vec<String> = Vec::new();
    let qty = intent.qty.0;
    if entry && !cls.entry_qty.is_multiple_of(intent.lot_size) {
        why.push(format!(
            "The new quantity {} is not a multiple of the lot size {}.",
            cls.entry_qty, intent.lot_size
        ));
    }
    if tick_known {
        let off_tick = |p: Decimal| p.checked_rem(intent.tick_size) != Some(Decimal::ZERO);
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
    use crate::sizing::{SizingInput, size};
    use crate::snapshot::Position;
    use bazaar_core::instrument::Exchange;
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

    fn pos(side: Side, qty: u32) -> Position {
        Position {
            symbol: "ABC".into(),
            side,
            qty,
            stop: Tri::Unknown,
            risk_price: Tri::Unknown,
            avg_cost: Tri::Unknown,
        }
    }

    fn snap() -> PortfolioSnapshot {
        let mut s = PortfolioSnapshot::unknown();
        s.positions = Tri::Known(vec![]);
        s.kill_switch = Tri::Known(false);
        s.orders_last_minute = Tri::Known(0);
        s.orders_today = Tri::Known(0);
        s.starting_cash = Tri::Known(Decimal::from(100_000));
        s.realised_pnl = Tri::Known(Decimal::ZERO);
        s.equity_mtm = Tri::Known(Decimal::from(100_000));
        s
    }

    fn modes() -> ModeInputs {
        ModeInputs {
            user_approval: Tri::Unknown,
            compliance_fresh: Tri::Unknown,
            readiness_report: Tri::Unknown,
            owner_opt_in: Tri::Unknown,
            live: Tri::Known(false),
            live_trading_compiled: Tri::Unknown,
        }
    }

    fn inputs() -> StructureInputs {
        StructureInputs {
            option: Tri::Known(OptionKind::NotAnOption),
            mode: modes(),
        }
    }

    fn run16(
        c: &RuleCtx,
        i: &OrderIntent,
        s: &PortfolioSnapshot,
        inp: &StructureInputs,
    ) -> RuleResult {
        g16(c, i, &classify(i, s).unwrap(), s, inp, i.price)
    }

    fn run16_at(
        c: &RuleCtx,
        i: &OrderIntent,
        s: &PortfolioSnapshot,
        notional_price: Decimal,
    ) -> RuleResult {
        g16(c, i, &classify(i, s).unwrap(), s, &inputs(), notional_price)
    }

    #[test]
    fn paper_passes() {
        let r = run16(
            &ctx(ExecutionMode::Paper),
            &intent(Side::Buy, 10, "500"),
            &snap(),
            &inputs(),
        );
        assert!(r.passed(), "{}", r.message());
    }

    #[test]
    fn rejection_message_ends_with_rule_id() {
        let mut s = snap();
        s.kill_switch = Tri::Known(true);
        let r = run16(
            &ctx(ExecutionMode::Paper),
            &intent(Side::Buy, 10, "500"),
            &s,
            &inputs(),
        );
        assert!(!r.passed() && r.message().ends_with("(G16)"));
    }

    #[test]
    fn paper_with_live_target_rejects_and_unknown_live_rejects() {
        let mut inp = inputs();
        inp.mode.live = Tri::Known(true);
        inp.mode.live_trading_compiled = Tri::Known(true);
        let c = ctx(ExecutionMode::Paper);
        let i = intent(Side::Buy, 10, "500");
        assert!(!run16(&c, &i, &snap(), &inp).passed());
        inp.mode.live = Tri::Unknown;
        assert!(!run16(&c, &i, &snap(), &inp).passed());
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

    /// Table: only the `SemiAuto` + approval absent (`Unknown`) + no other reason case is pending.
    #[test]
    fn approval_pending_marker_table() {
        use ExecutionMode::{Auto, Paper, SemiAuto};
        let i = intent(Side::Buy, 10, "500");
        let kill = {
            let mut s = snap();
            s.kill_switch = Tri::Known(true);
            s
        };
        let per_day = {
            let mut s = snap();
            s.orders_today = Tri::Known(10_000);
            s
        };
        let live_unknown = {
            let mut m = inputs();
            m.mode.live = Tri::Unknown;
            m
        };
        let approved_other = {
            let mut m = inputs();
            m.mode.user_approval = Tri::Known(false);
            m
        };
        let approved = {
            let mut m = inputs();
            m.mode.user_approval = Tri::Known(true);
            m
        };
        // (label, mode, snapshot, inputs, expect pending, expect failed)
        let cases = [
            ("sole approval gap", SemiAuto, snap(), inputs(), true, true),
            (
                "approval for another order",
                SemiAuto,
                snap(),
                approved_other,
                false,
                true,
            ),
            ("approved", SemiAuto, snap(), approved, false, false),
            ("kill switch too", SemiAuto, kill, inputs(), false, true),
            (
                "orders per day too",
                SemiAuto,
                per_day,
                inputs(),
                false,
                true,
            ),
            (
                "live unknown too",
                SemiAuto,
                snap(),
                live_unknown,
                false,
                true,
            ),
            ("auto locked", Auto, snap(), inputs(), false, true),
            ("paper", Paper, snap(), inputs(), false, false),
        ];
        for (label, mode, s, inp, pending, failed) in cases {
            let r = run16(&ctx(mode), &i, &s, &inp);
            assert_eq!(r.is_approval_pending(), pending, "{label}");
            assert_eq!(!r.passed(), failed, "{label}");
            if failed {
                assert_eq!(r.rule(), RuleId::G16, "{label}");
            }
        }
    }

    #[test]
    fn missing_approval_and_live_gap_name_both_reasons() {
        let c = ctx(ExecutionMode::SemiAuto);
        let mut m = inputs();
        m.mode.live = Tri::Unknown;
        let r = run16(&c, &intent(Side::Buy, 10, "500"), &snap(), &m);
        assert!(!r.is_approval_pending());
        assert!(
            r.message()
                .contains("approval of this order is not on record"),
            "{}",
            r.message()
        );
        assert!(r.message().contains("live broker"), "{}", r.message());
    }

    #[test]
    fn marker_leaves_message_and_level_alone_and_names_every_reason() {
        let c = ctx(ExecutionMode::SemiAuto);
        let i = intent(Side::Buy, 10, "500");
        let pending = run16(&c, &i, &snap(), &inputs());
        assert!(
            pending
                .message()
                .contains("approval of this order is not on record")
        );
        assert_eq!(pending.level(), crate::verdict::OverrideLevel::HardCeiling);
        let mut s = snap();
        s.kill_switch = Tri::Known(true);
        let both = run16(&c, &i, &s, &inputs());
        assert!(
            both.message()
                .contains("approval of this order is not on record")
        );
        assert!(
            both.message().contains("kill switch is on"),
            "{}",
            both.message()
        );
        assert!(!both.is_approval_pending());
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
        for mode in [ExecutionMode::SemiAuto, ExecutionMode::Auto] {
            let c = ctx(mode);
            let i = intent(Side::Buy, 10, "500");
            let mut inp = inputs();
            inp.mode.user_approval = Tri::Known(true);
            inp.mode.compliance_fresh = Tri::Known(true);
            inp.mode.readiness_report = Tri::Known(true);
            inp.mode.owner_opt_in = Tri::Known(true);
            inp.mode.live = Tri::Known(true);
            assert!(!run16(&c, &i, &snap(), &inp).passed());
            inp.mode.live_trading_compiled = Tri::Known(false);
            assert!(!run16(&c, &i, &snap(), &inp).passed());
            inp.mode.live_trading_compiled = Tri::Known(true);
            assert!(run16(&c, &i, &snap(), &inp).passed());
        }
    }

    #[test]
    fn mode_lock_applies_to_exits() {
        let c = ctx(ExecutionMode::SemiAuto);
        let mut s = snap();
        s.positions = Tri::Known(vec![pos(Side::Buy, 10)]);
        let i = intent(Side::Sell, 10, "500");
        assert!(!run16(&c, &i, &s, &inputs()).passed());
        let mut inp = inputs();
        inp.mode.user_approval = Tri::Known(true);
        assert!(run16(&c, &i, &s, &inp).passed());
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
    fn kill_switch_on_lets_exit_and_stop_move_through_but_stops_a_flip() {
        let c = ctx(ExecutionMode::Paper);
        let mut s = snap();
        s.kill_switch = Tri::Known(true);
        s.positions = Tri::Known(vec![Position {
            stop: Tri::Known(d("480")),
            ..pos(Side::Buy, 10)
        }]);
        let exit = intent(Side::Sell, 10, "500");
        assert!(run16(&c, &exit, &s, &inputs()).passed());
        let mut mv = intent(Side::Buy, 0, "500");
        mv.stop = None;
        mv.new_stop = Some(d("485"));
        assert!(run16(&c, &mv, &s, &inputs()).passed());
        let flip = intent(Side::Sell, 15, "500");
        let r = run16(&c, &flip, &s, &inputs());
        assert!(!r.passed() && r.message().contains("kill switch"));
    }

    #[test]
    fn exit_passes_with_unknown_kill_switch_and_is_not_notional_capped() {
        let c = ctx(ExecutionMode::Paper);
        let mut s = snap();
        s.kill_switch = Tri::Unknown;
        s.positions = Tri::Known(vec![pos(Side::Buy, 1000)]);
        let i = intent(Side::Sell, 1000, "500");
        assert!(run16(&c, &i, &s, &inputs()).passed());
    }

    #[test]
    fn order_rate_boundary_and_applies_to_exits() {
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
        s.positions = Tri::Known(vec![pos(Side::Buy, 10)]);
        let exit = intent(Side::Sell, 10, "500");
        s.orders_last_minute = Tri::Known(max);
        assert!(!run16(&c, &exit, &s, &inputs()).passed());
        s.orders_last_minute = Tri::Known(max - 1);
        assert!(run16(&c, &exit, &s, &inputs()).passed());
    }

    #[test]
    fn orders_per_day_boundary() {
        let c = ctx(ExecutionMode::Paper);
        let i = intent(Side::Buy, 10, "500");
        let max = c.limits.max_orders_per_day;
        let mut s = snap();
        s.orders_today = Tri::Known(max - 1);
        assert!(run16(&c, &i, &s, &inputs()).passed());
        s.orders_today = Tri::Known(max);
        assert!(!run16(&c, &i, &s, &inputs()).passed());
        s.orders_today = Tri::Unknown;
        assert!(!run16(&c, &i, &s, &inputs()).passed());
    }

    #[test]
    fn notional_boundary_on_the_reference_price() {
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
    fn notional_uses_the_higher_live_price_and_not_a_lower_one() {
        let c = ctx(ExecutionMode::Paper);
        let cap = c.limits.max_notional_per_order_inr;
        // 50 shares at a limit of cap/50 sits exactly at the cap.
        let limit_price = cap / Decimal::from(50);
        let i = intent(Side::Buy, 50, &limit_price.to_string());
        assert!(run16_at(&c, &i, &snap(), limit_price).passed());
        // Live is higher: the reference price is the live price and the cap bites.
        let r = run16_at(&c, &i, &snap(), limit_price + d("0.05"));
        assert!(!r.passed() && r.message().contains("per-order cap"));
        // A lower reference than the order's own price is floored at the order price.
        let hi = intent(Side::Buy, 50, &(limit_price + d("0.05")).to_string());
        let r = run16_at(&c, &hi, &snap(), limit_price);
        assert!(!r.passed() && r.message().contains("per-order cap"));
    }

    #[test]
    fn non_positive_price_and_overflow_reject_entries() {
        let c = ctx(ExecutionMode::Paper);
        let i = intent(Side::Buy, 10, "500");
        assert!(!run16_at(&c, &i, &snap(), Decimal::ZERO).passed());
        assert!(!run16_at(&c, &i, &snap(), d("-5")).passed());
        let mut zero = intent(Side::Buy, 10, "0");
        zero.stop = None;
        assert!(!run16_at(&c, &zero, &snap(), d("500")).passed());
        let big = intent(Side::Buy, 10, "500");
        let r = run16_at(&c, &big, &snap(), Decimal::MAX);
        assert!(!r.passed());
    }

    #[test]
    fn flip_entry_part_is_notional_capped() {
        let c = ctx(ExecutionMode::Paper);
        let cap = c.limits.max_notional_per_order_inr;
        let mut s = snap();
        s.positions = Tri::Known(vec![pos(Side::Buy, 10)]);
        // Sell 10 + (cap/price + 1) excess.
        let excess = (cap / d("100")).to_string().parse::<u32>().unwrap() + 1;
        let i = intent(Side::Sell, 10 + excess, "100");
        let cls = classify(&i, &s).unwrap();
        assert_eq!(cls.entry_qty, excess);
        assert!(!g16(&c, &i, &cls, &s, &inputs(), i.price).passed());
        let ok = intent(Side::Sell, 10 + excess - 1, "100");
        let cls = classify(&ok, &s).unwrap();
        assert!(g16(&c, &ok, &cls, &s, &inputs(), ok.price).passed());
    }

    fn run13(
        c: &RuleCtx,
        i: &OrderIntent,
        s: &PortfolioSnapshot,
        o: Tri<OptionKind>,
    ) -> RuleResult {
        let mut inp = inputs();
        inp.option = o;
        g13(c, i, &classify(i, s).unwrap(), s, &inp)
    }

    /// The per-trade budget the way G13 and `size` derive it, from config and the snapshot.
    fn budget(c: &RuleCtx, s: &PortfolioSnapshot) -> Decimal {
        sizing_equity(s, &c.limits).unwrap() * c.limits.max_risk_per_trade_pct
            / Decimal::ONE_HUNDRED
    }

    #[test]
    fn g13_basic_cases() {
        let c = ctx(ExecutionMode::Paper);
        let s = snap();
        let buy = intent(Side::Buy, 10, "500");
        assert!(run13(&c, &buy, &s, Tri::Known(OptionKind::NotAnOption)).passed());
        assert!(run13(&c, &buy, &s, Tri::Known(OptionKind::LongOption)).passed());
        assert!(!run13(&c, &buy, &s, Tri::Unknown).passed());
        let naked = run13(&c, &buy, &s, Tri::Known(OptionKind::NakedShort));
        assert!(!naked.passed() && naked.message().ends_with("(G13)"));
    }

    #[test]
    fn g13_long_option_with_a_sell_entry_is_a_naked_short() {
        let c = ctx(ExecutionMode::Paper);
        let s = snap();
        let sell = intent(Side::Sell, 10, "500");
        let r = run13(&c, &sell, &s, Tri::Known(OptionKind::LongOption));
        assert!(!r.passed() && r.message().ends_with("(G13)"));
    }

    #[test]
    fn g13_flip_of_a_long_option_to_short_rejects_the_excess() {
        let c = ctx(ExecutionMode::Paper);
        let mut s = snap();
        s.positions = Tri::Known(vec![pos(Side::Buy, 10)]);
        let flip = intent(Side::Sell, 15, "500");
        assert!(!run13(&c, &flip, &s, Tri::Known(OptionKind::LongOption)).passed());
        // Closing the long exactly is not an entry.
        let close = intent(Side::Sell, 10, "500");
        assert!(run13(&c, &close, &s, Tri::Known(OptionKind::LongOption)).passed());
    }

    #[test]
    fn g13_naked_short_rejects_even_with_the_flag_forced_true() {
        let mut c = ctx(ExecutionMode::Paper);
        c.limits.allow_naked_short = true;
        let s = snap();
        let sell = intent(Side::Sell, 10, "500");
        assert!(!run13(&c, &sell, &s, Tri::Known(OptionKind::NakedShort)).passed());
    }

    #[test]
    fn g13_exit_of_a_short_is_never_blocked() {
        let c = ctx(ExecutionMode::Paper);
        let mut s = snap();
        s.positions = Tri::Known(vec![pos(Side::Sell, 10)]);
        let close = intent(Side::Buy, 10, "500");
        assert!(run13(&c, &close, &s, Tri::Known(OptionKind::NakedShort)).passed());
    }

    #[test]
    fn g13_defined_risk_boundary_uses_the_config_derived_budget() {
        let c = ctx(ExecutionMode::Paper);
        let s = snap();
        let sell = intent(Side::Sell, 10, "500");
        let b = budget(&c, &s);
        let fit = b - c.limits.min_charges_slippage_buffer;
        let run = |loss: Decimal| {
            run13(
                &c,
                &sell,
                &s,
                Tri::Known(OptionKind::DefinedRisk { max_loss: loss }),
            )
        };
        assert!(run(fit).passed());
        assert!(!run(fit + Decimal::ONE).passed());
        // A caller can no longer choose the budget: a big loss never passes.
        assert!(!run(b * Decimal::from(40)).passed());
    }

    #[test]
    fn g13_defined_risk_rejects_zero_negative_loss_and_unknown_equity() {
        let c = ctx(ExecutionMode::Paper);
        let s = snap();
        let sell = intent(Side::Sell, 10, "500");
        for loss in [Decimal::ZERO, d("-40000")] {
            let r = run13(
                &c,
                &sell,
                &s,
                Tri::Known(OptionKind::DefinedRisk { max_loss: loss }),
            );
            assert!(!r.passed(), "{loss}");
        }
        let mut no_eq = snap();
        no_eq.equity_mtm = Tri::Unknown;
        let r = run13(
            &c,
            &sell,
            &no_eq,
            Tri::Known(OptionKind::DefinedRisk {
                max_loss: Decimal::ONE,
            }),
        );
        assert!(!r.passed());
    }

    fn instrument(t: InstrumentType) -> Instrument {
        Instrument {
            instrument_token: 1,
            exchange_token: 1,
            tradingsymbol: "ABC".into(),
            name: "ABC".into(),
            exchange: Exchange::Nfo,
            instrument_type: t,
            segment: "NFO-OPT".into(),
            expiry: None,
            strike: None,
            tick_size: d("0.05"),
            lot_size: 25,
        }
    }

    #[test]
    fn option_kind_comes_from_the_instrument_master() {
        let k = |t, m| OptionKind::from_instrument(&instrument(t), m);
        assert_eq!(
            k(InstrumentType::Equity, None),
            Tri::Known(OptionKind::NotAnOption)
        );
        assert_eq!(
            k(InstrumentType::Future, None),
            Tri::Known(OptionKind::NotAnOption)
        );
        for t in [InstrumentType::Call, InstrumentType::Put] {
            assert_eq!(k(t.clone(), None), Tri::Known(OptionKind::LongOption));
            assert_eq!(
                k(t, Some(d("900"))),
                Tri::Known(OptionKind::DefinedRisk { max_loss: d("900") })
            );
        }
        assert_eq!(k(InstrumentType::Other("XX".into()), None), Tri::Unknown);
    }

    #[test]
    fn option_entries_are_never_sizable() {
        assert!(is_sizable(&Tri::Known(OptionKind::NotAnOption)));
        assert!(!is_sizable(&Tri::Known(OptionKind::LongOption)));
        assert!(!is_sizable(&Tri::Known(OptionKind::NakedShort)));
        assert!(!is_sizable(&Tri::Known(OptionKind::DefinedRisk {
            max_loss: Decimal::ONE
        })));
        assert!(!is_sizable(&Tri::Unknown));
    }

    fn run_x02(i: &OrderIntent, s: &PortfolioSnapshot) -> RuleResult {
        x02(i, &classify(i, s).unwrap())
    }

    #[test]
    fn x02_lot_and_tick() {
        let s = snap();
        let mut i = intent(Side::Buy, 75, "500.05");
        i.lot_size = 25;
        assert!(run_x02(&i, &s).passed());
        i.qty = Qty(80);
        let r = run_x02(&i, &s);
        assert!(!r.passed() && r.message().ends_with("(X02)"));
        i.qty = Qty(75);
        i.price = d("500.03");
        assert!(!run_x02(&i, &s).passed());
        i.price = d("500");
        i.stop = Some(d("490.02"));
        assert!(!run_x02(&i, &s).passed());
    }

    #[test]
    fn x02_entry_with_zero_lot_or_tick_rejects() {
        let s = snap();
        let mut i = intent(Side::Buy, 10, "500");
        i.lot_size = 0;
        assert!(!run_x02(&i, &s).passed());
        i.lot_size = 1;
        i.tick_size = Decimal::ZERO;
        assert!(!run_x02(&i, &s).passed());
    }

    #[test]
    fn x02_odd_lot_residual_exit_passes_but_flip_with_odd_entry_part_rejects() {
        let mut s = snap();
        s.positions = Tri::Known(vec![pos(Side::Buy, 30)]);
        let mut exit = intent(Side::Sell, 30, "500");
        exit.lot_size = 25;
        assert!(run_x02(&exit, &s).passed());
        // 30 held, sell 55: 25 closes... entry part is 25 (a lot multiple) and passes.
        let mut flip = intent(Side::Sell, 55, "500");
        flip.lot_size = 25;
        assert!(run_x02(&flip, &s).passed());
        // Sell 60: entry part 30 is not a lot multiple.
        flip.qty = Qty(60);
        let r = run_x02(&flip, &s);
        assert!(!r.passed() && r.message().contains("new quantity 30"));
    }

    #[test]
    fn x02_exit_with_unknown_lot_or_tick_passes_but_off_tick_exit_rejects() {
        let mut s = snap();
        s.positions = Tri::Known(vec![pos(Side::Buy, 10)]);
        let mut exit = intent(Side::Sell, 10, "500");
        exit.lot_size = 0;
        assert!(run_x02(&exit, &s).passed());
        exit.tick_size = Decimal::ZERO;
        assert!(run_x02(&exit, &s).passed());
        let mut off = intent(Side::Sell, 10, "500.03");
        off.lot_size = 1;
        let r = run_x02(&off, &s);
        assert!(!r.passed() && r.message().contains("0.05 tick"));
    }

    #[test]
    fn x02_stop_move_checks_new_stop_tick() {
        let mut s = snap();
        s.positions = Tri::Known(vec![Position {
            stop: Tri::Known(d("480")),
            ..pos(Side::Buy, 10)
        }]);
        let mut i = intent(Side::Buy, 0, "500.03");
        i.stop = None;
        i.new_stop = Some(d("485"));
        let cls = classify(&i, &s).unwrap();
        assert!(x02(&i, &cls).passed());
        i.new_stop = Some(d("485.02"));
        assert!(!x02(&i, &cls).passed());
    }

    #[test]
    fn a_sized_order_passes_g16_g13_and_x02() {
        let mut c = ctx(ExecutionMode::Paper);
        c.limits.max_risk_per_trade_pct = Decimal::ONE;
        c.limits.max_position_value_pct = Decimal::from(25);
        c.limits.max_notional_per_order_inr = Decimal::from(25_000);
        c.limits.min_charges_slippage_buffer = Decimal::from(60);
        c.limits.cap_equity_at_starting_cash = true;
        let s = snap();
        let equity = sizing_equity(&s, &c.limits).unwrap();
        assert_eq!(equity, Decimal::from(100_000));
        let sized = size(
            &c.limits,
            &SizingInput {
                equity,
                side: Side::Buy,
                entry: d("500"),
                stop: d("490"),
                lot_size: 1,
                tick_size: d("0.05"),
                buffer: Tri::Known(Decimal::from(60)),
                overridden: false,
            },
        )
        .unwrap();
        assert_eq!((sized.qty, sized.notional), (50, Decimal::from(25_000)));
        let i = intent(Side::Buy, sized.qty, "500");
        let cls = classify(&i, &s).unwrap();
        let r16 = g16(&c, &i, &cls, &s, &inputs(), i.price);
        assert!(r16.passed(), "{}", r16.message());
        assert!(g13(&c, &i, &cls, &s, &inputs()).passed());
        assert!(x02(&i, &cls).passed());
        assert!(is_sizable(&inputs().option));
        // One share more is over the G16 cap the sizer respects.
        let more = intent(Side::Buy, sized.qty + 1, "500");
        let cls = classify(&more, &s).unwrap();
        assert!(!g16(&c, &more, &cls, &s, &inputs(), more.price).passed());
    }
}
