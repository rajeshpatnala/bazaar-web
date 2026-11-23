//! kill-switch routes (BZ-511 S4 fills it). The kill switch never places an order.

use utoipa_axum::router::OpenApiRouter;

use crate::state::AppState;

/// Routes of this module, relative to `/api/v1` (none yet).
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
}
