//! Market regime detection (trend/range and volatility state).
//!
//! Spec: `docs/strategy/F7-regime-definitions.md`. Not investment advice; not SEBI registered; markets
//! carry risk. A label describes recent behaviour, it does not predict the next bar. Step 1 holds the types and the config; the
//! classifier follows.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

mod classify;
mod fit;
mod params;
mod types;

pub use classify::{ClassifyError, Extras, Series, classify};
pub use fit::{Fit, RegimeFit, RegimeFitError, label};
pub use params::{MarketVolSource, ParamsError, RegimeParams};
pub use types::{Fact, RegimeState, Trend, Tri, VolState};
