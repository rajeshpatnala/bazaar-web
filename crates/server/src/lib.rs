//! Composition root: axum REST on `127.0.0.1`, security layer, `OpenAPI`, static UI.
//!
//! The module list is frozen by F1 slice 0; slices 5-7 fill their own files only.

pub mod app;
pub mod dto;
pub mod error;
pub mod openapi;
pub mod routes;
pub mod runtime;
pub mod security;
pub mod state;
