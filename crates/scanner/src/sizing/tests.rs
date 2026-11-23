use bazaar_core::instrument::{Exchange, InstrumentType};
use bazaar_risk::config::{Baseline, GateConfig};
use bazaar_risk::sizing::SizingError;

use super::*;

const RISK: &str = include_str!("../../../../fixtures/config/risk.toml");

fn d(n: i64) -> Decimal {
    Decimal::from(n)
}

fn px(s: &str) -> Decimal {
    s.parse().unwrap()
}

/// 1% risk, 25% position cap, Rs 60 floor.
fn limits() -> EffectiveLimits {
    let mut l = GateConfig::from_toml_str(RISK)
        .unwrap()
        .effective(&Baseline::none());
    l.max_risk_per_trade_pct = d(1);
    l.max_position_value_pct = d(25);
    l.min_charges_slippage_buffer = d(60);
    l.cap_equity_at_starting_cash = true;
    l
}

fn snap() -> PortfolioSnapshot {
    let mut s = PortfolioSnapshot::unknown();
    s.starting_cash = Tri::Known(d(100_000));
    s.realised_pnl = Tri::Known(d(0));
    s.equity_mtm = Tri::Known(d(100_000));
    s
}

fn inst(tick: &str, lot: u32) -> Instrument {
    Instrument {
        instrument_token: 1,
        exchange_token: 1,
        tradingsymbol: "ABC".into(),
        name: "ABC".into(),
        exchange: Exchange::parse("NSE"),
        instrument_type: InstrumentType::parse("EQ"),
        segment: "NSE".into(),
        expiry: None,
        strike: None,
        tick_size: px(tick),
        lot_size: lot,
    }
}

fn levels() -> Levels {
    Levels {
        entry_low: px("499.98"),
        entry_high: px("500.02"),
        stop: px("489.97"),
        targets: vec![px("520.03"), d(530)],
    }
}

fn flat(c: i64) -> impl Fn(u32) -> Tri<Decimal> {
    move |_| Tri::Known(d(c))
}

fn tick_collapse(r: &Result<Levels, BlockReason>, needle: &str) {
    assert!(
        matches!(r, Err(BlockReason::TickCollapse(m)) if m.contains(needle)),
        "expected TickCollapse({needle}), got {r:?}"
    );
}

#[test]
fn rounding_goes_against_the_trader() {
    let r = round_levels(&levels(), px("0.05")).unwrap();
    assert_eq!(r.entry_low, d(500));
    assert_eq!(r.entry_high, px("500.05"));
    assert_eq!(r.stop, px("489.95"));
    assert_eq!(r.targets, vec![d(520), d(530)]);
}

#[test]
fn stop_96_0333_rounds_down_to_96_00() {
    let l = Levels {
        entry_low: d(100),
        entry_high: px("100.02"),
        stop: px("96.0333"),
        targets: vec![d(110)],
    };
    let r = round_levels(&l, px("0.05")).unwrap();
    assert_eq!(r.stop, d(96));
    assert_eq!(r.entry_high, px("100.05"));
}

#[test]
fn zone_narrower_than_a_tick_collapses() {
    let l = Levels {
        entry_low: px("100.01"),
        entry_high: px("100.02"),
        stop: d(95),
        targets: vec![d(110)],
    };
    tick_collapse(&round_levels(&l, px("0.05")), "narrower than one tick");
}

#[test]
fn a_target_rounding_into_the_zone_collapses() {
    let mut l = levels();
    l.entry_low = d(100);
    l.entry_high = d(100);
    l.stop = d(95);
    l.targets = vec![px("100.03")];
    tick_collapse(&round_levels(&l, px("0.05")), "target is not above");
}

#[test]
fn targets_that_merge_are_not_strictly_rising() {
    let mut l = levels();
    l.targets = vec![px("520.01"), px("520.04")];
    tick_collapse(&round_levels(&l, px("0.05")), "strictly rising");
}

#[test]
fn stop_rounding_to_zero_collapses() {
    let mut l = levels();
    l.stop = px("0.03");
    tick_collapse(&round_levels(&l, px("0.05")), "zero or below");
}

#[test]
fn no_targets_collapses() {
    let mut l = levels();
    l.targets = vec![];
    tick_collapse(&round_levels(&l, px("0.05")), "no target");
}

#[test]
fn price_overflow_while_rounding_is_a_block_not_a_panic() {
    let l = Levels {
        entry_low: px("50000000000000000000000000000"),
        entry_high: px("50000000000000000000000000000"),
        stop: px("40000000000000000000000000000"),
        targets: vec![px("60000000000000000000000000000")],
    };
    tick_collapse(&round_levels(&l, px("0.05")), "overflowed");
}

#[test]
fn invalid_input_levels_and_tick_are_sizing_blocks() {
    let mut l = levels();
    l.stop = l.entry_low; // stop equals entry
    assert!(matches!(
        round_levels(&l, px("0.05")),
        Err(BlockReason::Sizing(m)) if m.contains("invalid before rounding")
    ));
    assert!(matches!(
        round_levels(&levels(), Decimal::ZERO),
        Err(BlockReason::Sizing(m)) if m.contains("tick size")
    ));
}

#[test]
fn sizes_field_by_field_with_position_cap_and_floor_allowance() {
    // per unit 500.05 - 489.95 = 10.10; risk allows 93, the 25% cap (25,000 / 500.05) allows 49.
    let s = size_idea(&inst("0.05", 1), &levels(), &snap(), &limits(), &flat(10)).unwrap();
    assert_eq!((s.size.qty, s.size.lots), (49, 49));
    assert_eq!(s.risk_at_stop, px("494.90"));
    assert_eq!(s.allowance, d(60), "floor beats a Rs 10 cost");
    assert_eq!(s.risk_rupees, px("554.90"));
    assert_eq!(s.budget, d(1000));
    assert_eq!(s.notional, px("24502.45"));
    assert!(s.capped_by_position_cap);
    assert_eq!(s.reward_risk, px("19.95") / px("10.10"));
    assert_eq!(s.levels.entry_high, px("500.05"));
    assert_eq!(s.levels.stop, px("489.95"));
}

#[test]
fn cost_above_the_floor_is_the_allowance() {
    let cost = |q: u32| Tri::Known(Decimal::from(q) * d(3));
    let s = size_idea(&inst("0.05", 1), &levels(), &snap(), &limits(), &cost).unwrap();
    assert_eq!(s.size.qty, 49);
    assert_eq!(s.allowance, d(147));
    assert_eq!(s.risk_rupees, px("641.90"));
    assert!(s.risk_rupees <= s.budget);
}

#[test]
fn budget_below_one_lot_is_blocked_not_one_lot() {
    // One lot of 100 risks 100 x 10.10 = 1,010 + 60 > the Rs 1,000 budget.
    let r = size_idea(&inst("0.05", 100), &levels(), &snap(), &limits(), &flat(10));
    assert_eq!(
        r,
        Err(BlockReason::Sizing(
            SizingError::BudgetBelowOneLot.to_string()
        ))
    );
}

#[test]
fn lots_come_from_the_instrument() {
    let mut l = limits();
    l.max_position_value_pct = d(100);
    let s = size_idea(&inst("0.05", 10), &levels(), &snap(), &l, &flat(10)).unwrap();
    assert_eq!((s.size.qty, s.size.lots), (90, 9));
}

#[test]
fn zero_lot_unknown_equity_and_bad_cost_name_the_check() {
    let sz = |e: SizingError| Err(BlockReason::Sizing(e.to_string()));
    let i = inst("0.05", 1);
    assert_eq!(
        size_idea(&inst("0.05", 0), &levels(), &snap(), &limits(), &flat(10)),
        sz(SizingError::ZeroLot)
    );
    assert_eq!(
        size_idea(
            &i,
            &levels(),
            &PortfolioSnapshot::unknown(),
            &limits(),
            &flat(10)
        ),
        sz(SizingError::InputUnknown("your starting cash"))
    );
    assert_eq!(
        size_idea(&i, &levels(), &snap(), &limits(), &|_| Tri::Unknown),
        sz(SizingError::InputUnknown(
            "the charges and slippage allowance"
        ))
    );
    assert_eq!(
        size_idea(&i, &levels(), &snap(), &limits(), &flat(-5)),
        sz(SizingError::NegativeCost)
    );
}

#[test]
fn tick_collapse_blocks_before_sizing() {
    let mut l = levels();
    l.entry_low = px("100.01");
    l.entry_high = px("100.02");
    l.stop = d(95);
    l.targets = vec![d(110)];
    assert!(matches!(
        size_idea(&inst("0.05", 1), &l, &snap(), &limits(), &flat(10)),
        Err(BlockReason::TickCollapse(m)) if m.contains("narrower")
    ));
}

#[test]
fn charged_risk_refuses_above_budget_and_allows_equal() {
    let f = d(60);
    assert_eq!(
        charged_risk(d(900), d(101), f, d(1000)),
        Err(BlockReason::RiskAboveBudget)
    );
    assert_eq!(
        charged_risk(d(900), d(100), f, d(1000)),
        Ok((d(100), d(1000)))
    );
    // floor beats a small cost
    assert_eq!(charged_risk(d(900), d(10), f, d(1000)), Ok((d(60), d(960))));
}

#[test]
fn charged_risk_overflow_is_a_sizing_block() {
    let r = charged_risk(Decimal::MAX, d(60), d(60), Decimal::MAX);
    assert!(matches!(r, Err(BlockReason::Sizing(m)) if m.contains("overflowed")));
}

#[test]
fn a_pure_cost_never_reaches_the_final_guard() {
    for c in [0, 10, 60, 200, 400] {
        let s = size_idea(&inst("0.05", 1), &levels(), &snap(), &limits(), &flat(c)).unwrap();
        assert!(s.risk_rupees <= s.budget);
    }
}

#[test]
fn cost_priced_on_the_rounded_stop_sets_the_allowance() {
    let r = round_levels(&levels(), px("0.05")).unwrap();
    let rate = px("0.01");
    let on = |stop: Decimal| move |q: u32| Tri::Known(Decimal::from(q) * stop * rate);
    let s = size_idea(&inst("0.05", 1), &levels(), &snap(), &limits(), &on(r.stop)).unwrap();
    assert_eq!(s.size.qty, 49);
    assert_eq!(s.allowance, px("240.0755")); // 49 x 489.95 x 0.01
    assert_eq!(s.risk_rupees, px("734.9755"));
    assert_ne!(s.allowance, Decimal::from(49) * levels().stop * rate);
}

#[test]
fn a_single_price_zone_stays_valid() {
    let mut l = levels();
    l.entry_low = d(500);
    l.entry_high = d(500);
    let r = round_levels(&l, px("0.05")).unwrap();
    assert_eq!((r.entry_low, r.entry_high), (d(500), d(500)));
}

#[test]
fn rounding_holds_at_other_ticks_and_leaves_grid_levels_alone() {
    for t in ["0.01", "0.05", "0.10", "0.50", "5"] {
        let tick = px(t);
        let l = Levels {
            entry_low: px("1000.03"),
            entry_high: px("1013.07"),
            stop: px("951.93"),
            targets: vec![px("1100.01"), px("1200.09")],
        };
        let r = round_levels(&l, tick).unwrap();
        let on_grid = |x: Decimal| x % tick == Decimal::ZERO;
        assert!(
            on_grid(r.entry_low) && on_grid(r.entry_high) && on_grid(r.stop),
            "{t}"
        );
        assert!(r.targets.iter().all(|x| on_grid(*x)), "{t}");
        assert!(
            r.entry_low >= l.entry_low && r.entry_high >= l.entry_high,
            "{t}"
        );
        assert!(r.stop <= l.stop, "{t}");
        assert!(r.targets.iter().zip(&l.targets).all(|(a, b)| a <= b), "{t}");
        assert!(
            r.entry_high - l.entry_high < tick && l.stop - r.stop < tick,
            "{t}"
        );
        assert_eq!(round_levels(&r, tick).unwrap(), r, "idempotent at {t}");
    }
}
