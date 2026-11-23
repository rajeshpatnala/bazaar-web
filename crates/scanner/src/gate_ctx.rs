//! What the scanner needs to run the gate for its ideas, read once per scan through
//! [`crate::ports::GateContextSource`]. The scanner never builds a favourable default: whatever the
//! source cannot supply stays `Tri::Unknown` or absent, and the gate then says no.

use std::collections::BTreeMap;

use bazaar_risk::config::EffectiveLimits;
use bazaar_risk::gate::{CostModel, ModeEvidence};
use bazaar_risk::rules::exposure::ExposureInputs;
use bazaar_risk::rules::inputs::DataInputs;
use bazaar_risk::snapshot::PortfolioSnapshot;
use rust_decimal::Decimal;

/// The context could not be read; every idea of the scan is blocked with this reason.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ContextUnavailable(pub String);

/// Gate inputs for one scan. Read once; built only through [`ScanGateContext::new`], which removes
/// any approval, so nothing a scan produces can look approved (an approval is bound to one order).
#[derive(Debug)]
pub struct ScanGateContext {
    snapshot: PortfolioSnapshot,
    limits: EffectiveLimits,
    cost: Box<dyn CostModel>,
    data: DataInputs,
    mode: ModeEvidence,
    reference_prices: BTreeMap<u64, Decimal>,
    exposure: Option<ExposureInputs>,
}

impl ScanGateContext {
    /// `limits` must come from `risk::baseline::rule_ctx` with the same config and baseline as the
    /// `Gate`; the scanner checks that they still match after the gate ran (never two copies).
    /// `reference_prices` maps instrument token to the decision-bar close; a missing token gives
    /// an Unknown live price (G10 fails closed).
    #[must_use]
    pub fn new(
        snapshot: PortfolioSnapshot,
        limits: EffectiveLimits,
        cost: Box<dyn CostModel>,
        data: DataInputs,
        mut mode: ModeEvidence,
        reference_prices: BTreeMap<u64, Decimal>,
        exposure: Option<ExposureInputs>,
    ) -> Self {
        mode.approval = None;
        Self {
            snapshot,
            limits,
            cost,
            data,
            mode,
            reference_prices,
            exposure,
        }
    }
    #[must_use]
    pub const fn snapshot(&self) -> &PortfolioSnapshot {
        &self.snapshot
    }
    #[must_use]
    pub const fn limits(&self) -> &EffectiveLimits {
        &self.limits
    }
    #[must_use]
    pub fn cost(&self) -> &dyn CostModel {
        self.cost.as_ref()
    }
    #[must_use]
    pub const fn data(&self) -> &DataInputs {
        &self.data
    }
    #[must_use]
    pub const fn mode(&self) -> &ModeEvidence {
        &self.mode
    }
    #[must_use]
    pub fn reference_price(&self, instrument_token: u64) -> Option<Decimal> {
        self.reference_prices.get(&instrument_token).copied()
    }
    #[must_use]
    pub const fn exposure(&self) -> Option<&ExposureInputs> {
        self.exposure.as_ref()
    }
}
