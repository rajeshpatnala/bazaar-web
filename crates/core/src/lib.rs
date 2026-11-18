//! Domain types, `Money`, IST `Clock`, `MarketCalendar` and config loading; depends on no internal crate.
//!
//! The module list is frozen by F1 slice 0; later slices fill their own files only.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod calendar;
pub mod clock;
pub mod config;
pub mod disclaimer;
pub mod error;
pub mod mode;
pub mod money;
pub mod segment;
#[cfg(feature = "test-util")]
pub mod testutil;
