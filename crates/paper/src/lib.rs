//! `PaperExecutor` and `PaperAccount` (virtual rupee cash).

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::unused_async,
        clippy::unused_async_trait_impl
    )
)]

pub mod account;
pub mod cost_pool;
pub mod executor;
pub mod fill;
pub mod pnl;
pub mod reset_guard;
pub mod squareoff;
pub mod watch;
