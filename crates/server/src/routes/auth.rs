//! `GET /api/v1/auth/me`.

use axum::{Extension, Json, extract::State};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{dto::AuthMeDto, error::Problem, security::Session, state::AppState};

/// Session probe: 200 with the CSRF token, else 401 from the session guard.
#[utoipa::path(get, path = "/auth/me", tag = "auth",
    responses(
        (status = 200, body = AuthMeDto),
        (status = 401, body = Problem, content_type = "application/problem+json"),
    ))]
#[allow(clippy::unused_async)] // axum handlers must be async fns
async fn me(State(s): State<AppState>, Extension(sess): Extension<Session>) -> Json<AuthMeDto> {
    Json(AuthMeDto {
        csrf_token: sess.csrf,
        mode: s.mode.into(),
        live_enabled: s.live_enabled,
    })
}

/// Routes of this module, relative to `/api/v1`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(me))
}
