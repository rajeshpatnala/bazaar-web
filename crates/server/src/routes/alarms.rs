//! alarm routes (BZ-511 S2/S5 fill it); alarms also appear on `GET /status`.

use utoipa_axum::router::OpenApiRouter;

use crate::state::AppState;

/// Routes of this module, relative to `/api/v1` (none yet).
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
}
