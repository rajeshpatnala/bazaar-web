//! Order pipeline, approval queue, `Executor` trait, kill switch and audit chain.
//!
//! Slice 0 of F10: shared contracts only (ids, `OrderIntent`, the state machine, `Executor`,
//! the audit hash). Persistence goes through the repo traits in `bazaar_core::ports`.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod approval;
pub mod audit;
pub mod breaker;
pub mod executor;
pub mod fills;
pub mod gate_adapter;
pub mod guarded;
pub mod killswitch;
pub mod pipeline;
pub mod ratelimit;
pub mod sink;
pub mod state;
pub mod types;
