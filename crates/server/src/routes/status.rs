//! `GET /healthz` (public) and `GET /api/v1/status`.

use axum::{Json, extract::State};
use serde_json::{Value, json};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{dto::StatusDto, error::Problem, state::AppState};

/// Liveness: no data, no auth.
#[allow(clippy::unused_async)] // axum handlers must be async fns
pub async fn healthz() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

/// Server status (F1 subset).
#[utoipa::path(get, path = "/status", tag = "status",
    responses(
        (status = 200, body = StatusDto),
        (status = 401, body = Problem, content_type = "application/problem+json"),
    ))]
#[allow(clippy::unused_async)] // axum handlers must be async fns
async fn status(State(s): State<AppState>) -> Json<StatusDto> {
    Json(StatusDto {
        server_time: s.clock.now_ist().to_rfc3339(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        mode: s.mode.into(),
        live_enabled: s.live_enabled,
        alarms: s.alarms.snapshot().into_iter().map(Into::into).collect(),
    })
}

/// Routes of this module, relative to `/api/v1`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(status))
}
