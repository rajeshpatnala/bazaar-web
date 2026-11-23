//! `POST /api/v1/approvals/{id}/decision` (BZ-511 S1 fills it). No route places an order.

use utoipa_axum::router::OpenApiRouter;

use crate::state::AppState;

/// Routes of this module, relative to `/api/v1` (none yet).
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
}
