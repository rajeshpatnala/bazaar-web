use bazaar_regime::{Fact, RegimeState};
use bazaar_risk::config::ConfigHash;
use bazaar_risk::verdict::{GateVerdict, RuleId, RuleResult};
use chrono::{DateTime, TimeZone};
use chrono_tz::{Asia::Kolkata, Tz};

use super::*;
use crate::types::{
    Action, Evidence, InstrumentRef, PositionSize, RecommendationParts, Warning, reward_risk,
};

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
        WhyFactor::Evidence {
            trades: 40,
            profit_factor: Some(1.4),
            costs_applied: true,
            out_of_sample: true,
        },
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

fn build(
    why: Vec<WhyFactor>,
    backtest: Option<Evidence>,
    qty: u32,
    g: GateSummary,
) -> Recommendation {
    let p = RecommendationParts {
        id: 7,
        generated_at: t(18),
        data_as_of: t(15),
        instrument: InstrumentRef {
            instrument_token: 1,
            tradingsymbol: "ABC".into(),
            name: "Abc Ltd".into(),
        },
        action: Action::Buy,
        entry_zone: (d(100), d(102)),
        stop_loss: d(96),
        targets: vec![d(110)],
        horizon: Horizon::Swing,
        size: PositionSize { qty, lots: 1 },
        risk_rupees: d(60),
        reward_risk: reward_risk(d(102), d(96), d(110)).unwrap(),
        agreeing_strategies: vec!["donchian_breakout".into()],
        confidence: 80,
        regime: RegimeState::unknown(t(15), Fact::new("a", "b", "c")),
        backtest,
        why,
        warnings: vec![Warning::EventsNotChecked],
        gate: g,
    };
    Recommendation::new(p, 30, 55).unwrap()
}

fn full() -> Recommendation {
    build(all_factors(), Some(evidence()), 10, gate(&[]))
}

fn one(f: WhyFactor) -> String {
    let lines = render(&build(vec![f], Some(evidence()), 10, gate(&[])));
    assert_eq!(lines.len(), 3, "factor + gate + footer");
    lines[0].clone()
}

#[test]
fn full_render_has_d12_order_and_exact_text() {
    let lines = render(&full());
    assert_eq!(lines.len(), 11);
    assert_eq!(
        lines[0],
        "If this goes wrong and your stop is hit, you lose about Rs 60 (your limit per trade is Rs 1,000). Quantity 10 shares, entry Rs 100 to Rs 102, stop Rs 96."
    );
    assert_eq!(
        lines[1],
        "Target Rs 110. On the worst fill you would make about 1.33 : 1 for every Re 1 you risk (reward to risk). A target is a hope, not a promise."
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
        "About Rs 12.50 crore of this share trades on a typical day, so you can get in and out."
    );
    assert_eq!(
        lines[6],
        "The idea is wrong if the price closes below Rs 96, or if it has not worked after 10 days."
    );
    assert_eq!(lines[7], "No ban today (list dated 2026-11-20).");
    assert_eq!(lines[8], "Dates to know: Results on 24 Nov.");
    assert_eq!(
        lines[9],
        "The safety checks passed when this was scanned; they run again before any order."
    );
    assert_eq!(lines[10], FOOTER);
}

#[test]
fn render_is_deterministic() {
    assert_eq!(render(&full()), render(&full()));
}

#[test]
fn no_evidence_says_no_testing_and_no_zeros() {
    let r = build(all_factors(), None, 10, gate(&[]));
    let line = &render(&r)[4];
    assert!(line.starts_with("No past testing exists"), "{line}");
    assert!(!line.contains("profit factor"));
}

#[test]
fn low_evidence_is_labelled() {
    let mut e = evidence();
    e.trades = 10;
    let r = build(all_factors(), Some(e), 10, gate(&[]));
    let line = &render(&r)[4];
    assert!(
        line.ends_with("This is low evidence, so the score is held down."),
        "{line}"
    );
}

#[test]
fn costs_and_sample_flags_are_shown() {
    let r = build(
        vec![WhyFactor::Evidence {
            trades: 40,
            profit_factor: Some(1.4),
            costs_applied: false,
            out_of_sample: false,
        }],
        Some(evidence()),
        10,
        gate(&[]),
    );
    let line = &render(&r)[0];
    assert!(line.contains("before costs"), "{line}");
    assert!(line.contains("fitted to"), "{line}");
}

#[test]
fn penalised_regime_says_rougher_than_ideal() {
    let line = one(WhyFactor::RegimeFit {
        market_trend: "up".into(),
        stock_trend: "up".into(),
        vol: "high".into(),
        penalised: true,
    });
    assert!(
        line.contains("rougher than ideal, so the score is cut."),
        "{line}"
    );
}

#[test]
fn ban_variants() {
    assert_eq!(
        one(WhyFactor::Ban {
            checked: false,
            list_date: None
        }),
        "The ban list was not checked, so confirm this name is not banned before acting."
    );
    assert_eq!(
        one(WhyFactor::Ban {
            checked: true,
            list_date: None
        }),
        "No ban today."
    );
}

#[test]
fn events_variants_never_say_no_events_when_unchecked() {
    let unchecked = one(WhyFactor::Events {
        checked: false,
        notes: vec![],
    });
    assert_eq!(unchecked, "Results or policy dates were not checked.");
    let none = one(WhyFactor::Events {
        checked: true,
        notes: vec![],
    });
    assert_eq!(
        none,
        "No results or policy dates were found in the checked window."
    );
}

#[test]
fn quantity_one_is_singular() {
    let line = &render(&build(
        vec![WhyFactor::Risk {
            risk_rupees: d(60),
            limit_rupees: d(1000),
            qty: 1,
        }],
        None,
        1,
        gate(&[]),
    ))[0];
    assert!(line.contains("Quantity 1 share,"), "{line}");
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
}

#[test]
fn horizon_unit_is_days_for_daily() {
    assert_eq!(time_unit(Horizon::Swing), "days");
}

#[test]
fn hostile_note_is_escaped() {
    let note = "Results\n<script>alert(\"x\")</script> & 'more'\r\u{0007}\u{202E}end";
    let line = one(WhyFactor::Events {
        checked: true,
        notes: vec![note.into()],
    });
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
    let note = "a".repeat(5000);
    let line = one(WhyFactor::Events {
        checked: true,
        notes: vec![note],
    });
    assert_eq!(
        line,
        format!("Dates to know: {}.", "a".repeat(MAX_TEXT_CHARS))
    );
}

#[test]
fn long_multibyte_note_does_not_split_a_char() {
    let note = "₹".repeat(500);
    let line = one(WhyFactor::Events {
        checked: true,
        notes: vec![note],
    });
    assert_eq!(line.chars().filter(|c| *c == '₹').count(), MAX_TEXT_CHARS);
}

#[test]
fn huge_and_tiny_rupees() {
    let line = one(WhyFactor::Risk {
        risk_rupees: Decimal::new(123_456_789_012, 2),
        limit_rupees: Decimal::new(1, 2),
        qty: 3,
    });
    assert!(line.contains("Rs 1,23,45,67,890.12"), "{line}");
    assert!(line.contains("limit per trade is Rs 0.01"), "{line}");
    let liq = one(WhyFactor::Liquidity {
        median_traded_value: Decimal::new(5, 2),
    });
    assert!(liq.starts_with("Only about Rs 0.05 of this share"), "{liq}");
    let big = one(WhyFactor::Liquidity {
        median_traded_value: d(10_000_000),
    });
    assert!(big.starts_with("About Rs 1 crore"), "{big}");
}

#[test]
fn profit_factor_none_or_nan_is_not_shown_as_a_number() {
    for pf in [None, Some(f64::NAN), Some(f64::INFINITY)] {
        let line = one(WhyFactor::Evidence {
            trades: 40,
            profit_factor: pf,
            costs_applied: true,
            out_of_sample: true,
        });
        assert!(line.contains("profit factor not available"), "{line}");
        assert!(
            !line.to_lowercase().contains("nan") && !line.contains("inf"),
            "{line}"
        );
    }
}

#[test]
fn empty_names_and_missing_factors_do_not_panic() {
    let r = build(
        vec![WhyFactor::Confluence {
            families_agreeing: 1,
            families_total: 1,
            names: vec![],
        }],
        None,
        10,
        gate(&[]),
    );
    assert_eq!(render(&r).len(), 3);
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
    unchecked[0] = WhyFactor::Events {
        checked: false,
        notes: vec![],
    };
    unchecked[1] = WhyFactor::Ban {
        checked: false,
        list_date: None,
    };
    let blocked = gate(&[RuleId::G16, RuleId::G05, RuleId::G01]);
    vec![
        full(),
        build(all_factors(), None, 10, gate(&[])),
        build(all_factors(), Some(low), 10, gate(&[])),
        build(all_factors(), Some(no_costs), 10, gate(&[])),
        build(penal, Some(evidence()), 10, gate(&[])),
        build(unchecked, Some(evidence()), 10, gate(&[])),
        build(all_factors(), Some(evidence()), 10, blocked),
        build(all_factors(), Some(evidence()), 10, gate(&[RuleId::G01])),
    ]
}

#[test]
fn no_forbidden_words_in_any_variant() {
    let banned = [
        "guaranteed",
        "sure",
        "certain",
        "certainly",
        "risk-free",
        "can't lose",
        "cannot lose",
        "will",
        "strong buy",
        "probability of profit",
        "% chance",
    ];
    for rec in variants() {
        for line in render(&rec) {
            let lower = line.to_lowercase();
            let words: Vec<&str> = lower
                .split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '-'))
                .collect();
            for b in banned {
                let hit = if b.contains(' ') || b.contains('%') {
                    lower.contains(b)
                } else {
                    words.contains(&b)
                };
                assert!(!hit, "forbidden `{b}` in: {line}");
            }
        }
    }
}

#[test]
fn confidence_is_a_score_never_a_percent() {
    for rec in variants() {
        let text = render(&rec).join(" ");
        assert!(!text.contains("80%") && !text.contains("55%"), "{text}");
    }
    assert!(render(&full())[2].contains("Score 80 out of 100"));
}

#[test]
fn ratio_is_cut_not_rounded_up() {
    assert_eq!(ratio(Decimal::new(19999, 4)), "1.99 : 1");
    assert_eq!(ratio(d(2)), "2.00 : 1");
}
