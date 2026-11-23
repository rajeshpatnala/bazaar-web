use bazaar_regime::{Fact, RegimeState};
use bazaar_risk::config::ConfigHash;
use bazaar_risk::verdict::{GateVerdict, RuleId, RuleResult};
use chrono::{DateTime, TimeZone};
use chrono_tz::{Asia::Kolkata, Tz};

use super::*;
use crate::types::{
    Evidence, Horizon, InstrumentRef, PositionSize, RecommendationParts, Warning, reward_risk,
};

/// Words and phrases no line may contain (whole words; phrases by substring). Shared with the UI
/// copy test later (deferred to F12).
pub(crate) const FORBIDDEN: &[&str] = &[
    "guarantee",
    "guarantees",
    "guaranteed",
    "sure",
    "surely",
    "certain",
    "certainly",
    "assured",
    "assure",
    "ensure",
    "ensures",
    "definitely",
    "safe",
    "safely",
    "riskless",
    "risk-free",
    "sure-shot",
    "promise",
    "promised",
    "will",
    "strong buy",
    "no risk",
    "easy money",
    "never lose",
    "cannot lose",
    "can't lose",
    "cannot fail",
    "can't fail",
    "no chance of loss",
    "probability of profit",
    "% chance",
];

/// The only negated uses of "safe" and "promise" the renderer may print.
const EXEMPT: &[&str] = &["does not make the trade safe", "not a promise"];

fn t(h: u32) -> DateTime<Tz> {
    Kolkata.with_ymd_and_hms(2026, 11, 20, h, 0, 0).unwrap()
}

fn d(n: i64) -> Decimal {
    Decimal::from(n)
}

fn gate(failing: &[RuleId]) -> GateSummary {
    let rs: Vec<RuleResult> = RuleId::EVALUATION_ORDER
        .iter()
        .map(|r| {
            if failing.contains(r) {
                RuleResult::fail(*r, "x", "y", "no")
            } else {
                RuleResult::pass(*r, "x", "y")
            }
        })
        .collect();
    let v = GateVerdict::new(ConfigHash::of_text("cfg"), &rs).unwrap();
    GateSummary::from_verdict(&v, t(18))
}

fn evidence() -> Evidence {
    Evidence {
        trades: 40,
        win_rate: 0.5,
        profit_factor: 1.4,
        avg_r: 0.2,
        max_drawdown_pct: 8.0,
        period: "2021 to 2025".into(),
        costs_applied: true,
        out_of_sample: true,
    }
}

fn evidence_factor(trades: u32, pf: Option<f64>, costs: bool, oos: bool) -> WhyFactor {
    WhyFactor::Evidence {
        trades,
        profit_factor: pf,
        costs_applied: costs,
        out_of_sample: oos,
    }
}

fn all_factors() -> Vec<WhyFactor> {
    vec![
        // Deliberately out of order: the renderer fixes the order.
        WhyFactor::Events {
            checked: true,
            notes: vec!["Results on 24 Nov".into()],
        },
        WhyFactor::Ban {
            checked: true,
            list_date: Some("2026-11-20".into()),
        },
        WhyFactor::Invalidation {
            stop: d(96),
            max_hold_bars: 10,
        },
        WhyFactor::Liquidity {
            median_traded_value: d(125_000_000),
        },
        evidence_factor(40, Some(1.4), true, true),
        WhyFactor::RegimeFit {
            market_trend: "up".into(),
            stock_trend: "up".into(),
            vol: "normal".into(),
            penalised: false,
        },
        WhyFactor::Confluence {
            families_agreeing: 2,
            families_total: 4,
            names: vec!["trend".into(), "momentum".into()],
        },
        WhyFactor::Reward {
            target: d(110),
            reward_risk: reward_risk(d(102), d(96), d(110)).unwrap(),
        },
        WhyFactor::Risk {
            risk_rupees: d(60),
            limit_rupees: d(1000),
            qty: 10,
        },
    ]
}

struct Spec {
    action: Action,
    why: Vec<WhyFactor>,
    backtest: Option<Evidence>,
    qty: u32,
    gate: GateSummary,
    warnings: Vec<Warning>,
}

fn spec(why: Vec<WhyFactor>) -> Spec {
    Spec {
        action: Action::Buy,
        why,
        backtest: Some(evidence()),
        qty: 10,
        gate: gate(&[]),
        warnings: vec![],
    }
}

fn make(s: Spec) -> Recommendation {
    let sell = s.action == Action::Sell;
    let (stop, target) = if sell {
        (d(106), d(90))
    } else {
        (d(96), d(110))
    };
    let worst = if sell { d(100) } else { d(102) };
    let p = RecommendationParts {
        id: 7,
        generated_at: t(18),
        data_as_of: t(15),
        instrument: InstrumentRef {
            instrument_token: 1,
            tradingsymbol: "ABC".into(),
            name: "Abc Ltd".into(),
        },
        action: s.action,
        entry_zone: (d(100), d(102)),
        stop_loss: stop,
        targets: vec![target],
        horizon: Horizon::Swing,
        size: PositionSize {
            qty: s.qty,
            lots: 1,
        },
        risk_rupees: d(60),
        reward_risk: reward_risk(worst, stop, target).unwrap(),
        agreeing_strategies: vec!["donchian_breakout".into()],
        confidence: 80,
        regime: RegimeState::unknown(t(15), Fact::new("a", "b", "c")),
        backtest: s.backtest,
        why: s.why,
        warnings: s.warnings,
        gate: s.gate,
    };
    Recommendation::new(p, 30, 55).unwrap()
}

fn full() -> Recommendation {
    make(spec(all_factors()))
}

/// Render a rec built from just these factors; returns all lines.
fn lines_of(why: Vec<WhyFactor>) -> Vec<String> {
    render(&make(spec(why)))
}

/// The one line that starts with `start`.
fn line<'a>(lines: &'a [String], start: &str) -> &'a str {
    let hits: Vec<&String> = lines.iter().filter(|l| l.starts_with(start)).collect();
    assert_eq!(hits.len(), 1, "{start}: {lines:#?}");
    hits[0]
}

#[test]
fn full_render_has_d12_order_and_exact_text() {
    let lines = render(&full());
    assert_eq!(lines.len(), 11);
    assert_eq!(
        lines[0],
        "If this goes wrong and your stop is hit, you lose about Rs 60 or more, as a gap or slippage can cost extra (your limit per trade is Rs 1,000). Quantity 10 shares, entry Rs 100 to Rs 102, stop Rs 96 (below your entry)."
    );
    assert_eq!(
        lines[1],
        "Target Rs 110. If the target is reached, for every Re 1 you risk you would make about Re 1.33 (reward to risk 1.33 : 1) on the worst fill, before charges and slippage. A target is a hope, not a promise."
    );
    assert_eq!(
        lines[2],
        "2 of 4 method families agree: trend, momentum. Score 80 out of 100 (a rule-fit score, not a chance of winning)."
    );
    assert_eq!(
        lines[3],
        "The market is up and this stock is up, normal volatility. This method is meant for that kind of market."
    );
    assert_eq!(
        lines[4],
        "Tested on 40 past trades after costs, on data the method had not seen: 1.40 profit factor."
    );
    assert_eq!(
        lines[5],
        "Typical daily turnover of this share is about Rs 12.50 crore."
    );
    assert_eq!(
        lines[6],
        "The idea is wrong if the price closes below Rs 96, or if it has not worked after 10 trading days."
    );
    assert_eq!(
        lines[7],
        "Not on the ban list dated 2026-11-20. Check today's list before acting."
    );
    assert_eq!(lines[8], "Dates to know: Results on 24 Nov.");
    assert_eq!(lines[9], GATE_PASSED);
    assert_eq!(lines[10], FOOTER);
}

#[test]
fn render_is_deterministic() {
    assert_eq!(render(&full()), render(&full()));
}

#[test]
fn sell_fixture_flips_direction_words() {
    let mut s = spec(all_factors());
    s.action = Action::Sell;
    let lines = render(&make(s));
    assert!(
        lines[0].ends_with("stop Rs 106 (above your entry)."),
        "{}",
        lines[0]
    );
    assert_eq!(
        line(&lines, "The idea is wrong"),
        "The idea is wrong if the price closes above Rs 106, or if it has not worked after 10 trading days."
    );
}

#[test]
fn missing_factors_are_never_silent() {
    let lines = lines_of(vec![WhyFactor::Confluence {
        families_agreeing: 1,
        families_total: 1,
        names: vec![],
    }]);
    // Risk, Reward, Confluence, Evidence, Invalidation, Ban, Events, gate, footer.
    assert_eq!(lines.len(), 9, "{lines:#?}");
    assert!(lines[0].contains("you lose about Rs 60 or more"));
    assert!(!lines[0].contains("your limit per trade"));
    assert!(lines[1].starts_with("Target Rs 110."));
    assert_eq!(lines[3], NO_TESTING);
    assert_eq!(
        lines[4],
        "The idea is wrong if the price closes below Rs 96."
    );
    assert_eq!(lines[5], BAN_UNCHECKED);
    assert_eq!(lines[6], EVENTS_UNCHECKED);
}

#[test]
fn factors_contradicting_the_rec_do_not_change_money_lines() {
    let lines = lines_of(vec![
        WhyFactor::Risk {
            risk_rupees: d(5),
            limit_rupees: d(1000),
            qty: 1,
        },
        WhyFactor::Reward {
            target: d(999),
            reward_risk: d(9),
        },
        WhyFactor::Invalidation {
            stop: d(50),
            max_hold_bars: 3,
        },
    ]);
    assert!(lines[0].contains("about Rs 60 or more"), "{}", lines[0]);
    assert!(lines[0].contains("Quantity 10 shares"), "{}", lines[0]);
    assert!(lines[0].contains("stop Rs 96"), "{}", lines[0]);
    assert!(lines[1].starts_with("Target Rs 110."), "{}", lines[1]);
    assert!(lines[1].contains("Re 1.33"), "{}", lines[1]);
    let inv = line(&lines, "The idea is wrong");
    assert!(
        inv.contains("below Rs 96") && inv.contains("after 3 trading days"),
        "{inv}"
    );
}

#[test]
fn evidence_factor_can_only_weaken_the_backtest() {
    let mk = |f: WhyFactor| line(&lines_of(vec![f]), "Tested on").to_owned();
    // Fewer trades, costs off, in-sample, lower profit factor: all taken.
    let weak = mk(evidence_factor(35, Some(0.9), false, false));
    assert!(
        weak.starts_with("Tested on 35 past trades before costs"),
        "{weak}"
    );
    assert!(weak.contains("fitted to"), "{weak}");
    assert!(
        weak.contains("0.90 profit factor (below 1.00 means it lost money)"),
        "{weak}"
    );
    // A stronger factor changes nothing: 40 trades, 1.4, flags on stay as the backtest says.
    let strong = mk(evidence_factor(500, Some(9.0), true, true));
    assert_eq!(
        strong,
        "Tested on 40 past trades after costs, on data the method had not seen: 1.40 profit factor."
    );
    // A factor with an unavailable profit factor blanks it.
    let none = mk(evidence_factor(40, None, true, true));
    assert!(none.contains("profit factor not available"), "{none}");
    // Zero trades in the factor: no testing.
    let lines = lines_of(vec![evidence_factor(0, None, true, true)]);
    assert!(lines.contains(&NO_TESTING.to_owned()));
}

#[test]
fn backtest_flags_beat_a_rosy_factor() {
    let mut s = spec(vec![evidence_factor(40, Some(1.4), true, true)]);
    let mut e = evidence();
    e.costs_applied = false;
    e.out_of_sample = false;
    s.backtest = Some(e);
    let lines = render(&make(s));
    let ev = line(&lines, "Tested on");
    assert!(
        ev.contains("before costs") && ev.contains("fitted to"),
        "{ev}"
    );
    assert!(
        ev.ends_with("This is low evidence, so the score is held down."),
        "{ev}"
    );
}

#[test]
fn no_backtest_is_no_testing_even_with_a_rosy_factor() {
    let mut s = spec(all_factors());
    s.backtest = None;
    let lines = render(&make(s));
    assert_eq!(lines[4], NO_TESTING);
    assert!(!lines.iter().any(|l| l.contains("profit factor")));
}

#[test]
fn low_evidence_is_labelled() {
    let mut s = spec(all_factors());
    let mut e = evidence();
    e.trades = 10;
    s.backtest = Some(e);
    let lines = render(&make(s));
    assert!(
        lines[4].starts_with("Tested on 10 past trades")
            && lines[4].ends_with("This is low evidence, so the score is held down."),
        "{}",
        lines[4]
    );
}

#[test]
fn profit_factor_is_cut_not_rounded_up() {
    let mk = |pf: f64| {
        let mut s = spec(vec![]);
        let mut e = evidence();
        e.profit_factor = pf;
        s.backtest = Some(e);
        s.why = vec![evidence_factor(40, Some(pf), true, true)];
        line(&render(&make(s)), "Tested on").to_owned()
    };
    assert!(mk(1.996).contains(": 1.99 profit factor."), "{}", mk(1.996));
    assert!(
        mk(0.999).contains("0.99 profit factor (below 1.00"),
        "{}",
        mk(0.999)
    );
    assert!(mk(1.15).contains(": 1.15 profit factor."), "{}", mk(1.15));
}

#[test]
fn profit_factor_nan_or_inf_in_the_factor_is_not_shown() {
    for pf in [None, Some(f64::NAN), Some(f64::INFINITY)] {
        let lines = lines_of(vec![evidence_factor(40, pf, true, true)]);
        let ev = line(&lines, "Tested on");
        assert!(ev.contains("profit factor not available"), "{ev}");
        assert!(
            !ev.to_lowercase().contains("nan") && !ev.contains("inf"),
            "{ev}"
        );
    }
}

#[test]
fn penalised_regime_says_rougher_than_ideal() {
    let lines = lines_of(vec![WhyFactor::RegimeFit {
        market_trend: "up".into(),
        stock_trend: "up".into(),
        vol: "high".into(),
        penalised: true,
    }]);
    let l = line(&lines, "The market is");
    assert!(
        l.contains("rougher than ideal, so the score is cut."),
        "{l}"
    );
}

fn ban(checked: bool, date: Option<&str>) -> WhyFactor {
    WhyFactor::Ban {
        checked,
        list_date: date.map(Into::into),
    }
}

fn events(checked: bool, notes: &[&str]) -> WhyFactor {
    WhyFactor::Events {
        checked,
        notes: notes.iter().map(|s| (*s).to_owned()).collect(),
    }
}

#[test]
fn ban_variants() {
    let l = |f| lines_of(vec![f]);
    assert!(l(ban(false, None)).contains(&BAN_UNCHECKED.to_owned()));
    assert!(l(ban(true, None)).contains(
        &"Not on the ban list (list date unknown). Check today's list before acting.".to_owned()
    ));
    assert!(l(ban(true, Some("2026-11-19"))).contains(
        &"Not on the ban list dated 2026-11-19. Check today's list before acting.".to_owned()
    ));
}

#[test]
fn duplicate_ban_and_events_factors_take_the_weakest_reading() {
    let l = lines_of(vec![ban(true, Some("2026-11-20")), ban(false, None)]);
    assert!(l.contains(&BAN_UNCHECKED.to_owned()), "{l:#?}");
    let l = lines_of(vec![ban(false, None), ban(true, Some("2026-11-20"))]);
    assert!(l.contains(&BAN_UNCHECKED.to_owned()), "{l:#?}");
    let l = lines_of(vec![events(true, &["a"]), events(false, &[])]);
    assert!(l.contains(&EVENTS_UNCHECKED.to_owned()), "{l:#?}");
    let l = lines_of(vec![events(true, &["a"]), events(true, &["b"])]);
    assert!(l.contains(&"Dates to know: a; b.".to_owned()), "{l:#?}");
}

#[test]
fn events_variants_never_say_none_found_when_unchecked() {
    let l = lines_of(vec![events(false, &[])]);
    assert!(l.contains(&EVENTS_UNCHECKED.to_owned()));
    let l = lines_of(vec![events(true, &[])]);
    assert!(l.contains(&"No results or policy dates were found in the checked window.".to_owned()));
    // Checked factor, but the rec carries the EventsNotChecked warning: unchecked wins.
    let mut s = spec(vec![events(true, &[])]);
    s.warnings = vec![Warning::EventsNotChecked];
    let l = render(&make(s));
    assert!(l.contains(&EVENTS_UNCHECKED.to_owned()), "{l:#?}");
    assert!(!l.iter().any(|x| x.contains("were found")));
}

#[test]
fn quantity_one_is_singular() {
    let mut s = spec(all_factors());
    s.qty = 1;
    let lines = render(&make(s));
    assert!(lines[0].contains("Quantity 1 share,"), "{}", lines[0]);
}

#[test]
fn gate_hard_soft_and_paper_only() {
    let hard = gate_line(&gate(&[RuleId::G16]));
    assert_eq!(
        hard,
        "Blocked by the safety checks: failed checks that cannot be overridden (G16)."
    );
    let mixed = gate_line(&gate(&[RuleId::G05, RuleId::G01]));
    assert_eq!(
        mixed,
        "Blocked by the safety checks: failed checks you can override with a written reason (G05); failed checks that can be overridden in paper mode only (G01)."
    );
    let paper = gate_line(&gate(&[RuleId::G01]));
    assert!(paper.contains("paper mode only (G01)") && !paper.contains("written reason"));
    let both = gate_line(&gate(&[RuleId::G16, RuleId::G01]));
    assert!(both.contains("(G16)") && both.contains("(G01)"));
    assert_eq!(gate_line(&gate(&[])), GATE_PASSED);
}

#[test]
fn hostile_note_is_escaped() {
    let note = "Results\n<script>alert(\"x\")</script> & 'more'\r\u{0007}\u{202E}end";
    let l = lines_of(vec![events(true, &[note])]);
    let line = line(&l, "Dates to know");
    assert!(
        !line.contains(['\n', '\r', '<', '>', '&', '"', '\'', '\u{7}', '\u{202E}']),
        "{line:?}"
    );
    assert!(
        line.contains("Results script alert( x ) /script more end"),
        "{line}"
    );
}

#[test]
fn very_long_note_is_capped() {
    let l = lines_of(vec![events(true, &[&"a".repeat(5000)])]);
    assert_eq!(
        line(&l, "Dates to know"),
        format!("Dates to know: {}.", "a".repeat(MAX_TEXT_CHARS))
    );
}

#[test]
fn long_multibyte_note_does_not_split_a_char() {
    let l = lines_of(vec![events(true, &[&"₹".repeat(500)])]);
    let n = line(&l, "Dates to know")
        .chars()
        .filter(|c| *c == '₹')
        .count();
    assert_eq!(n, MAX_TEXT_CHARS);
}

#[test]
fn huge_and_tiny_rupees() {
    let mut s = spec(vec![WhyFactor::Risk {
        risk_rupees: d(0),
        limit_rupees: Decimal::new(1, 2),
        qty: 3,
    }]);
    s.qty = 10;
    let l = render(&make(s));
    assert!(l[0].contains("limit per trade is Rs 0.01"), "{}", l[0]);
    let liq = |v: Decimal| {
        line(
            &lines_of(vec![WhyFactor::Liquidity {
                median_traded_value: v,
            }]),
            if v < d(10_000_000) {
                "Only about"
            } else {
                "Typical"
            },
        )
        .to_owned()
    };
    assert!(liq(Decimal::new(5, 2)).starts_with("Only about Rs 0.05 of this share"));
    assert_eq!(
        liq(d(10_000_000)),
        "Typical daily turnover of this share is about Rs 1 crore."
    );
    // 1.999 crore is cut to 1.99, not rounded up.
    assert!(liq(d(19_999_000)).ends_with("about Rs 1.99 crore."));
    assert!(liq(Decimal::new(123_456_789_012_345, 0)).contains("Rs 1,23,45,678.90 crore"));
}

#[test]
fn ratio_is_cut_not_rounded_up() {
    assert_eq!(ratio(Decimal::new(19999, 4)), "1.99");
    assert_eq!(ratio(d(2)), "2.00");
}

fn variants() -> Vec<Recommendation> {
    let mut low = evidence();
    low.trades = 3;
    let mut no_costs = evidence();
    no_costs.costs_applied = false;
    let mut penal = all_factors();
    penal[5] = WhyFactor::RegimeFit {
        market_trend: "up".into(),
        stock_trend: "up".into(),
        vol: "high".into(),
        penalised: true,
    };
    let mut unchecked = all_factors();
    unchecked[0] = events(false, &[]);
    unchecked[1] = ban(false, None);
    let with = |f: &dyn Fn(&mut Spec)| {
        let mut s = spec(all_factors());
        f(&mut s);
        make(s)
    };
    vec![
        full(),
        with(&|s| s.backtest = None),
        with(&|s| s.backtest = Some(low.clone())),
        with(&|s| s.backtest = Some(no_costs.clone())),
        make(Spec {
            why: penal,
            ..spec(vec![])
        }),
        make(Spec {
            why: unchecked,
            ..spec(vec![])
        }),
        with(&|s| s.gate = gate(&[RuleId::G16, RuleId::G05, RuleId::G01])),
        with(&|s| s.gate = gate(&[RuleId::G01])),
        with(&|s| s.action = Action::Sell),
        with(&|s| s.qty = 1),
        make(spec(vec![evidence_factor(40, Some(f64::NAN), true, true)])),
        make(spec(vec![ban(true, None), events(true, &[])])),
        with(&|s| s.warnings = vec![Warning::EventsNotChecked]),
    ]
}

#[test]
fn no_forbidden_words_in_any_variant() {
    for rec in variants() {
        for line in render(&rec) {
            let mut lower = line.to_lowercase();
            for e in EXEMPT {
                lower = lower.replace(e, " ");
            }
            let words: Vec<&str> = lower
                .split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '-'))
                .collect();
            for b in FORBIDDEN {
                let hit = if b.contains(' ') || b.contains('%') {
                    lower.contains(b)
                } else {
                    words.contains(b)
                };
                assert!(!hit, "forbidden `{b}` in: {line}");
            }
        }
    }
}

#[test]
fn forbidden_check_catches_a_planted_word() {
    // The guard must bite: a line with a forbidden word is flagged by the same logic.
    let line = "this is a safe and guaranteed idea";
    let words: Vec<&str> = line.split(' ').collect();
    assert!(FORBIDDEN.iter().any(|b| words.contains(b)));
}

#[test]
fn exempt_phrases_are_really_printed() {
    let lines = render(&full());
    assert!(lines[9].contains(EXEMPT[0]), "{}", lines[9]);
    assert!(lines[1].contains(EXEMPT[1]), "{}", lines[1]);
}

#[test]
fn confidence_is_a_score_never_a_percent() {
    for rec in variants() {
        let text = render(&rec).join(" ");
        assert!(!text.contains("80%") && !text.contains("55%"), "{text}");
    }
    assert!(render(&full())[2].contains("Score 80 out of 100"));
}
