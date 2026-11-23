//! `GET /api/v1/killswitch`, `POST /api/v1/killswitch/engage` and `/release`. Writes go only
//! through `PipelineHandle::set_kill_switch` (never the repo); the actor is the session's, never
//! the body's. Reason, actor and phrase are capped at 500 bytes (422 `TooLong`, nothing written);
//! a wrong, empty or short phrase is 403 and writes nothing. Reads fail closed: an unreadable
//! state is reported engaged (see `KillSwitchDto`). The kill switch never places an order.

use axum::{Extension, Json, extract::State};
use bazaar_execution::{executor::ExecError, killswitch::ReleaseError, pipeline::PipelineError};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{
    dto::{EngageRequestDto, KillSwitchDto, ReleaseRequestDto, escape_control},
    error::{ApiError, ApiJson, Problem},
    security::Session,
    state::AppState,
};

/// Unwired stays `pipeline_closed`; a pipeline that stopped under us is `pipeline_stopped`.
fn map_err(e: PipelineError) -> ApiError {
    match e {
        PipelineError::Closed => ApiError::PipelineStopped,
        other => ApiError::from(other),
    }
}

async fn current(s: &AppState) -> Result<Json<KillSwitchDto>, ApiError> {
    Ok(Json(KillSwitchDto::from(s.kill_switch()?.read().await)))
}

/// Current kill-switch state. An unreadable state is reported `engaged: true, readable: false`.
#[utoipa::path(get, path = "/killswitch", tag = "killswitch",
    responses(
        (status = 200, body = KillSwitchDto),
        (status = 401, body = Problem, content_type = "application/problem+json"),
        (status = 503, body = Problem, content_type = "application/problem+json"),
    ))]
async fn get_state(State(s): State<AppState>) -> Result<Json<KillSwitchDto>, ApiError> {
    current(&s).await
}

/// Engage the kill switch. Already engaged writes no second row. A failed write is 503, but the
/// in-process latch stays set, so orders are still refused; the 503 body says so (the GET reads
/// the store only and shows `engaged: false` until a save succeeds).
#[utoipa::path(post, path = "/killswitch/engage", tag = "killswitch",
    request_body = EngageRequestDto,
    responses(
        (status = 200, body = KillSwitchDto),
        (status = 401, body = Problem, content_type = "application/problem+json"),
        (status = 422, body = Problem, content_type = "application/problem+json"),
        (status = 503, body = Problem, content_type = "application/problem+json"),
    ))]
async fn engage(
    State(s): State<AppState>,
    Extension(sess): Extension<Session>,
    ApiJson(body): ApiJson<EngageRequestDto>,
) -> Result<Json<KillSwitchDto>, ApiError> {
    let already_engaged = s.kill_switch()?.read().await.engaged;
    s.pipeline()?
        .set_kill_switch(true, &sess.actor, &body.reason)
        .await
        .map_err(|e| match e {
            // Latched in this process but not persisted: say so (the GET reads the store only).
            PipelineError::Exec(ExecError::Storage) => ApiError::KillSwitchNotSaved,
            other => map_err(other),
        })?;
    tracing::warn!(
        actor = %escape_control(&sess.actor),
        reason = %escape_control(&body.reason),
        already_engaged,
        "kill switch engaged"
    );
    current(&s).await
}

/// Release with the typed phrase. A wrong, empty or short phrase is 403 and nothing is written;
/// a failed audit write is 503 and the switch stays engaged.
#[utoipa::path(post, path = "/killswitch/release", tag = "killswitch",
    request_body = ReleaseRequestDto,
    responses(
        (status = 200, body = KillSwitchDto),
        (status = 401, body = Problem, content_type = "application/problem+json"),
        (status = 403, body = Problem, content_type = "application/problem+json"),
        (status = 422, body = Problem, content_type = "application/problem+json"),
        (status = 503, body = Problem, content_type = "application/problem+json"),
    ))]
async fn release(
    State(s): State<AppState>,
    Extension(sess): Extension<Session>,
    ApiJson(body): ApiJson<ReleaseRequestDto>,
) -> Result<Json<KillSwitchDto>, ApiError> {
    s.kill_switch()?;
    s.pipeline()?
        .set_kill_switch(false, &sess.actor, &body.phrase)
        .await
        .map_err(|e| {
            if matches!(e, PipelineError::Release(ReleaseError::WrongPhrase)) {
                // Never log the phrase itself.
                tracing::warn!(
                    actor = %escape_control(&sess.actor),
                    "kill switch release refused: wrong phrase"
                );
            }
            map_err(e)
        })?;
    tracing::warn!(actor = %escape_control(&sess.actor), "kill switch released");
    current(&s).await
}

/// Routes of this module, relative to `/api/v1`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(get_state))
        .routes(routes!(engage))
        .routes(routes!(release))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn stopped_pipeline_differs_from_unwired() {
        let stopped = map_err(PipelineError::Closed);
        assert_eq!(stopped.code(), "pipeline_stopped");
        assert_ne!(stopped.code(), ApiError::PipelineClosed.code());
        assert_eq!(stopped.status(), 503);
    }

    #[test]
    fn too_long_is_422_and_wrong_phrase_is_403() {
        assert_eq!(map_err(PipelineError::TooLong).status(), 422);
        let w = map_err(PipelineError::Release(ReleaseError::WrongPhrase));
        assert_eq!(
            (w.status().as_u16(), w.code()),
            (403, "release_phrase_mismatch")
        );
    }
}
