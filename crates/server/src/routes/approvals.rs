//! `GET /api/v1/approvals` and `POST /api/v1/approvals/{id}/decision`. No route places an order:
//! a decision goes only through `PipelineHandle::decide`, which checks the TTL and re-runs the
//! gate with a fresh quote before any send (RISK-132-1). The actor is the session's, never the
//! body's (SEC-132-1).

use axum::{Extension, Json, extract::State, http::HeaderMap};
use bazaar_core::ports::ApprovalDecision;
use bazaar_execution::{approval::Outcome, executor::ExecError, types::IntentId};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{
    dto::{
        ApprovalDto, ApprovalListDto, DecisionOutcomeDto, DecisionRequestDto, OutcomeKind,
        escape_control,
    },
    error::{ApiError, ApiJson, ApiPath, FieldError, Problem, reject_repeat},
    security::Session,
    state::AppState,
};

/// Header that makes a retried decision return the first reply.
pub const IDEMPOTENCY_HEADER: &str = "idempotency-key";
const KEY_MIN: usize = 8;
const KEY_MAX: usize = 64;

fn bad(field: &str, message: &str) -> ApiError {
    ApiError::Validation(vec![FieldError {
        field: field.to_owned(),
        message: message.to_owned(),
    }])
}

fn parse_id(s: &str) -> Result<IntentId, ApiError> {
    if s.len() != 32 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(bad("id", "Must be 32 hex digits."));
    }
    u128::from_str_radix(s, 16)
        .map(IntentId)
        .map_err(|_| bad("id", "Must be 32 hex digits."))
}

fn idempotency_key(h: &HeaderMap) -> Result<String, ApiError> {
    let msg = "Required (8 to 64 printable characters).";
    let k = h
        .get(IDEMPOTENCY_HEADER)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| bad("Idempotency-Key", msg))?;
    if !(KEY_MIN..=KEY_MAX).contains(&k.len()) || !k.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(bad("Idempotency-Key", msg));
    }
    Ok(k.to_owned())
}

fn reply(outcome: OutcomeKind, detail: Option<String>) -> DecisionOutcomeDto {
    DecisionOutcomeDto {
        outcome,
        detail: detail.map(|d| escape_control(&d)),
    }
}

/// Domain outcome to wire reply. A repeat decision is rejected before this (409).
fn outcome_dto(o: Outcome) -> DecisionOutcomeDto {
    match o {
        Outcome::Queued => reply(OutcomeKind::Queued, None),
        Outcome::Declined => reply(OutcomeKind::Declined, None),
        Outcome::Expired => reply(OutcomeKind::Expired, None),
        Outcome::GateBlocked(why) => reply(OutcomeKind::Blocked, Some(why)),
        Outcome::QuoteDrifted { new_risk } => reply(
            OutcomeKind::QuoteDrifted,
            Some(format!("new_risk={new_risk}")),
        ),
        Outcome::GuardBlocked(e) => reply(OutcomeKind::GuardBlocked, Some(e.to_string())),
        Outcome::Placed(_) => reply(OutcomeKind::Placed, None),
        Outcome::PlacedUnrecorded(_) => reply(OutcomeKind::PlacedUnrecorded, None),
        Outcome::ExecFailed(e) => reply(OutcomeKind::ExecFailed, Some(e.to_string())),
        // Unreachable after `reject_repeat`; shown as an error, never as success.
        Outcome::AlreadyDecided(_) => reply(OutcomeKind::Error, Some("already decided".into())),
        Outcome::Error(e) => reply(OutcomeKind::Error, Some(error_category(&e))),
    }
}

fn error_category(e: &ExecError) -> String {
    match e {
        ExecError::Storage => "storage".to_owned(),
        _ => "execution".to_owned(),
    }
}

/// Orders waiting for a decision, with their expiry.
#[utoipa::path(get, path = "/approvals", tag = "approvals",
    responses(
        (status = 200, body = ApprovalListDto),
        (status = 401, body = Problem, content_type = "application/problem+json"),
        (status = 503, body = Problem, content_type = "application/problem+json"),
    ))]
async fn list(State(s): State<AppState>) -> Result<Json<ApprovalListDto>, ApiError> {
    let rows = s
        .pending()?
        .list()
        .await
        .ok_or(ApiError::StorageUnavailable)?;
    Ok(Json(ApprovalListDto {
        approvals: rows.into_iter().map(ApprovalDto::from).collect(),
    }))
}

/// Approve or decline one pending order. Approve re-checks the TTL and re-runs the gate with a
/// fresh quote; with no quote source every approve ends `blocked` and nothing is sent. A second
/// decision is 409; the same `Idempotency-Key` returns the first reply.
#[utoipa::path(post, path = "/approvals/{id}/decision", tag = "approvals",
    params(("id" = String, Path, description = "Intent id, 32 hex digits"),
           ("Idempotency-Key" = String, Header, description = "8 to 64 printable characters")),
    request_body = DecisionRequestDto,
    responses(
        (status = 200, body = DecisionOutcomeDto),
        (status = 401, body = Problem, content_type = "application/problem+json"),
        (status = 409, body = Problem, content_type = "application/problem+json"),
        (status = 422, body = Problem, content_type = "application/problem+json"),
        (status = 503, body = Problem, content_type = "application/problem+json"),
    ))]
async fn decide(
    State(s): State<AppState>,
    Extension(sess): Extension<Session>,
    ApiPath(id): ApiPath<String>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<DecisionRequestDto>,
) -> Result<Json<DecisionOutcomeDto>, ApiError> {
    let id = parse_id(&id)?;
    let key = format!("{}:{id}:{}", sess.actor, idempotency_key(&headers)?);
    let decision = ApprovalDecision::from(body.decision);
    let approve = decision == ApprovalDecision::Approve;
    if let Some((first_approve, first)) = s.idempotency.get(&key) {
        if first_approve != approve {
            return Err(bad(
                "Idempotency-Key",
                "Already used with a different decision.",
            ));
        }
        return Ok(Json(first));
    }
    let out = s.pipeline()?.decide(id, decision, &sess.actor).await?;
    let dto = outcome_dto(reject_repeat(out)?);
    s.idempotency.put(key, approve, dto.clone());
    Ok(Json(dto))
}

/// Routes of this module, relative to `/api/v1`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list))
        .routes(routes!(decide))
}
