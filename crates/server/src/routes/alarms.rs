//! `GET /api/v1/alarms`: the alarms raised by background drivers (also on `GET /status`). Reads
//! a snapshot of the in-memory board: no await, no store, no broker, so it cannot block or fail.
//! The route is read-only and never places an order.

use axum::{Json, extract::State};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{dto::AlarmDto, error::Problem, state::AppState};

/// Alarms currently raised (empty when all is well). Text is escaped by `AlarmDto`.
#[utoipa::path(get, path = "/alarms", tag = "alarms",
    responses(
        (status = 200, body = Vec<AlarmDto>),
        (status = 401, body = Problem, content_type = "application/problem+json"),
    ))]
#[allow(clippy::unused_async)] // axum handlers must be async fns
async fn list(State(s): State<AppState>) -> Json<Vec<AlarmDto>> {
    Json(s.alarms.snapshot().into_iter().map(Into::into).collect())
}

/// Routes of this module, relative to `/api/v1`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(list))
}
