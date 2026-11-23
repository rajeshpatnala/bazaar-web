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
        // A fixed category, never the executor's raw text (RISK-144-6).
        Outcome::ExecFailed(e) => reply(OutcomeKind::ExecFailed, Some(error_category(&e))),
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

/// Reply plus whether it may be remembered under the `Idempotency-Key`. Only final outcomes are:
/// a refusal or error the user may retry must run again, not replay (SEC-144-1). A storage
/// failure is 503 (the pipeline put the intent back) and an unknown intent is 404.
fn finish(o: Outcome) -> Result<(DecisionOutcomeDto, bool), ApiError> {
    match reject_repeat(o)? {
        Outcome::Error(ExecError::Storage) => Err(ApiError::StorageUnavailable),
        Outcome::Error(ExecError::InvalidInput("unknown intent")) => Err(ApiError::NotFound),
        o => {
            let is_final = matches!(
                o,
                Outcome::Declined
                    | Outcome::Expired
                    | Outcome::Placed(_)
                    | Outcome::PlacedUnrecorded(_)
            );
            Ok((outcome_dto(o), is_final))
        }
    }
}

/// Orders waiting for a decision, with their expiry.
#[utoipa::path(get, path = "/approvals", tag = "approvals",
    responses(
        (status = 200, body = ApprovalListDto),
        (status = 401, body = Problem, content_type = "application/problem+json"),
        (status = 503, body = Problem, content_type = "application/problem+json"),
    ))]
async fn list_approvals(State(s): State<AppState>) -> Result<Json<ApprovalListDto>, ApiError> {
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
/// decision is 409; the same `Idempotency-Key` returns the first reply, but only for final
/// outcomes (declined, expired, placed). The cache is in memory: a restart or an eviction at 256
/// keys turns a replay into 409, and two simultaneous requests with one key race (the second
/// may get 409); BZ-512 persists keys.
#[utoipa::path(post, path = "/approvals/{id}/decision", tag = "approvals",
    params(("id" = String, Path, description = "Intent id, 32 hex digits"),
           ("Idempotency-Key" = String, Header, description = "8 to 64 printable characters")),
    request_body = DecisionRequestDto,
    responses(
        (status = 200, body = DecisionOutcomeDto),
        (status = 401, body = Problem, content_type = "application/problem+json"),
        (status = 404, body = Problem, content_type = "application/problem+json"),
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
    if let Some((first_approve, first)) = s.idempotency().get(&key) {
        if first_approve != approve {
            return Err(bad(
                "Idempotency-Key",
                "Already used with a different decision.",
            ));
        }
        return Ok(Json(first));
    }
    let out = s.pipeline()?.decide(id, decision, &sess.actor).await?;
    let (dto, is_final) = finish(out)?;
    if is_final {
        s.idempotency().put(key, approve, dto.clone());
    }
    Ok(Json(dto))
}

/// Routes of this module, relative to `/api/v1`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_approvals))
        .routes(routes!(decide))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn storage_failure_is_retryable_503_and_unknown_intent_is_404() {
        assert_eq!(
            finish(Outcome::Error(ExecError::Storage)).unwrap_err(),
            ApiError::StorageUnavailable
        );
        assert_eq!(
            finish(Outcome::Error(ExecError::InvalidInput("unknown intent"))).unwrap_err(),
            ApiError::NotFound
        );
        assert_eq!(
            finish(Outcome::AlreadyDecided(Box::new(Outcome::Declined))).unwrap_err(),
            ApiError::AlreadyDecided
        );
    }

    #[test]
    fn only_final_outcomes_are_remembered() {
        let resolved = ExecError::InvalidInput("order already resolved");
        for (o, kept) in [
            (Outcome::Declined, true),
            (Outcome::Expired, true),
            (Outcome::GateBlocked("x".into()), false),
            (Outcome::GuardBlocked(ExecError::KillSwitch), false),
            (Outcome::Error(ExecError::Unknown), false),
            (Outcome::Error(resolved), false),
        ] {
            let label = format!("{o:?}");
            assert_eq!(finish(o).unwrap().1, kept, "{label}");
        }
    }

    #[test]
    fn exec_failure_detail_is_a_fixed_category() {
        let d = outcome_dto(Outcome::ExecFailed(ExecError::InvalidInput("raw text")));
        assert_eq!(d.detail.as_deref(), Some("execution"));
    }
}
