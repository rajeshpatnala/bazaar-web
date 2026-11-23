//! Runs strategies over the universe; scores, ranks and explains recommendations.
//!
//! Step 1 of F11: the output types and the ports the scanner reads through. A `Recommendation` is
//! advice for the user to read, never an order; `confidence` is a rule-fit score, not a probability.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod ports;
pub mod result;
pub mod runner;
pub mod score;
pub mod sizing;
pub mod types;
pub mod universe;

pub use ports::{BanList, BanListStatus, EventCalendar, EvidenceSource};
pub use result::{BlockReason, BlockedIdea, ScanResult, ScanVerdict};
pub use runner::{
    Candidate, CircuitState, Hit, ScanContext, ScanError, ScanOutput, UniverseEntry, build,
    effective_ban_age_hours, family_of, run,
};
pub use score::{Components, EVENT_WINDOW_DAYS, ScoreContext, Scored, rank, score, score_and_rank};
pub use sizing::{Levels, SizedIdea, round_levels, size_idea};
pub use types::{
    Action, Evidence, GateSummary, Horizon, InstrumentRef, PositionSize, RecError, Recommendation,
    RecommendationParts, Warning, WhyFactor, reward_risk,
};
pub use universe::DropReason;
