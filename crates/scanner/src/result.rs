//! What a scan returns: gate-passed ideas, blocked ideas with the reason, and every drop.
//! No `Default`: the verdict is always derived from the lists by [`ScanResult::new`].

use rust_decimal::Decimal;

use crate::types::{InstrumentRef, Recommendation};
use crate::universe::DropReason;

/// Why an idea is shown as blocked and not as a recommendation. Each names the check that refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum BlockReason {
    #[error("gate inputs unavailable: {0}")]
    GateInputsUnavailable(String),
    #[error("could not be sized: {0}")]
    Sizing(String),
    #[error("tick rounding broke the levels: {0}")]
    TickCollapse(String),
    #[error("risk including charges exceeds the budget of one trade")]
    RiskAboveBudget,
    #[error("limits changed between sizing and the gate")]
    LimitsChanged,
    #[error("the gate could not evaluate: {0}")]
    GateReject(String),
}

/// An idea that cannot be a `Recommendation` (no valid size or no gate result). The levels are
/// those the strategy gave, not rounded or sized. It is advice to look, never an order.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockedIdea {
    pub instrument: InstrumentRef,
    pub entry_zone: (Decimal, Decimal),
    pub stop_loss: Decimal,
    pub targets: Vec<Decimal>,
    pub reason: BlockReason,
}

/// The one-line outcome of a scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanVerdict {
    /// At least one sized idea passed the gate.
    Setups { count: usize },
    /// "No high-quality setups today", with the reasons (never empty).
    NoHighQualitySetups { reasons: Vec<String> },
}

/// Ranked ideas that passed the gate, blocked ideas (never truncated, same rank order) and drops.
#[derive(Debug, Clone, PartialEq)]
pub struct ScanResult {
    ideas: Vec<Recommendation>,
    blocked: Vec<BlockedIdea>,
    dropped: Vec<(String, DropReason)>,
    verdict: ScanVerdict,
}

impl ScanResult {
    /// `ideas` must already be gate-passed, ranked and truncated to `max_results`.
    #[must_use]
    pub fn new(
        ideas: Vec<Recommendation>,
        blocked: Vec<BlockedIdea>,
        dropped: Vec<(String, DropReason)>,
    ) -> Self {
        let verdict = if ideas.is_empty() {
            let mut reasons: Vec<String> = blocked
                .iter()
                .map(|b| format!("{}: {}", b.instrument.tradingsymbol, b.reason))
                .collect();
            if reasons.is_empty() {
                reasons.push(format!(
                    "no strategy fired on a name that passed the filters ({} dropped)",
                    dropped.len()
                ));
            }
            ScanVerdict::NoHighQualitySetups { reasons }
        } else {
            ScanVerdict::Setups { count: ideas.len() }
        };
        Self {
            ideas,
            blocked,
            dropped,
            verdict,
        }
    }
    #[must_use]
    pub fn ideas(&self) -> &[Recommendation] {
        &self.ideas
    }
    #[must_use]
    pub fn blocked(&self) -> &[BlockedIdea] {
        &self.blocked
    }
    #[must_use]
    pub fn dropped(&self) -> &[(String, DropReason)] {
        &self.dropped
    }
    #[must_use]
    pub const fn verdict(&self) -> &ScanVerdict {
        &self.verdict
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocked(sym: &str, reason: BlockReason) -> BlockedIdea {
        BlockedIdea {
            instrument: InstrumentRef {
                instrument_token: 1,
                tradingsymbol: sym.into(),
                name: sym.into(),
            },
            entry_zone: (Decimal::from(100), Decimal::from(102)),
            stop_loss: Decimal::from(96),
            targets: vec![Decimal::from(110)],
            reason,
        }
    }

    #[test]
    fn no_ideas_gives_no_setups_with_every_blocked_reason() {
        let r = ScanResult::new(
            vec![],
            vec![
                blocked("AAA", BlockReason::LimitsChanged),
                blocked("BBB", BlockReason::RiskAboveBudget),
            ],
            vec![],
        );
        assert_eq!(
            *r.verdict(),
            ScanVerdict::NoHighQualitySetups {
                reasons: vec![
                    "AAA: limits changed between sizing and the gate".into(),
                    "BBB: risk including charges exceeds the budget of one trade".into()
                ]
            }
        );
        assert_eq!(r.blocked().len(), 2);
    }

    #[test]
    fn an_empty_scan_still_explains_itself() {
        let r = ScanResult::new(vec![], vec![], vec![("X".into(), DropReason::Banned)]);
        assert_eq!(
            *r.verdict(),
            ScanVerdict::NoHighQualitySetups {
                reasons: vec![
                    "no strategy fired on a name that passed the filters (1 dropped)".into()
                ]
            }
        );
        assert_eq!(r.dropped().len(), 1);
    }
}
