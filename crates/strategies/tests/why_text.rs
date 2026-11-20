//! AC 6 for all three strategies: the why-text names the risk in rupees first, the market regime
//! and the "proven wrong" level, and never promises anything.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

mod common;

use bazaar_regime::label;
use bazaar_strategies::common::WHY_TEXT_LABEL;
use common::{CASES, World, candles, fire};

const BANNED: [&str; 5] = ["sure", "safe", "guaranteed", "certain", "risk-free"];

#[test]
fn every_strategys_why_text_follows_the_template() {
    for c in &CASES {
        let sig = fire(c);
        let text = sig
            .facts
            .iter()
            .find(|f| f.label == WHY_TEXT_LABEL)
            .unwrap_or_else(|| panic!("{} has a why fact", c.id))
            .value
            .clone();
        let prefix = format!("Risk first: you could lose about Rs {}", sig.risk_per_share);
        assert!(text.starts_with(&prefix), "{}: {text}", c.id);
        let bars = candles(c.csv, true);
        let market = label(&World::for_case(c, &bars[c.k]).market);
        assert!(
            text.contains(&format!("Market today: {market}.")),
            "{}: {text}",
            c.id
        );
        assert!(
            text.contains(&format!(
                "Proven wrong if: price falls to or below {:.2}",
                sig.stop
            )),
            "{}: {text}",
            c.id
        );
        let lower = text.to_lowercase();
        for word in BANNED {
            assert!(!lower.contains(word), "{} says {word}", c.id);
        }
    }
}
