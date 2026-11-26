//! Read-only client for Kite Connect (F3). Endpoint adapters build on [`KiteClient`].
//!
//! The client can only `GET`: it has no way to place, change or cancel an order.

mod client;
mod error;

pub use client::{KiteClient, KiteCredentials};
pub use error::{ApiErrorType, KiteError};
