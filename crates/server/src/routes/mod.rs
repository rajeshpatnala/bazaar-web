//! Route handlers (slice F1/server-app). Handlers are thin; no route places an order.

pub mod auth;
pub mod status;

use utoipa_axum::router::OpenApiRouter;

use crate::state::AppState;

/// Every session-protected route, relative to `/api/v1` (nested by `openapi::router`).
pub fn api() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .merge(auth::router())
        .merge(status::router())
}
