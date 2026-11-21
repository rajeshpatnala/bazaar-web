//! Runs strategies over the universe; scores, ranks and explains recommendations.
//!
//! Step 1 of F11: the output types and the ports the scanner reads through. A `Recommendation` is
//! advice for the user to read, never an order; `confidence` is a rule-fit score, not a probability.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod ports;
pub mod runner;
pub mod types;
pub mod universe;

pub use ports::{BanList, BanListStatus, EventCalendar, EvidenceSource};
pub use runner::{
    Candidate, Hit, ScanContext, ScanError, ScanOutput, UniverseEntry, build, family_of, run,
};
pub use types::{
    Action, Evidence, GateSummary, Horizon, InstrumentRef, PositionSize, RecError, Recommendation,
    RecommendationParts, Warning, WhyFactor,
};
pub use universe::DropReason;
