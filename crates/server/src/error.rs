//! problem+json error type and extractors (slice F1/server-security).
//!
//! `ApiError` variants map one-to-one to the stable problem `code`s in the API contract. Details
//! are fixed user-safe strings: never paths, SQL, upstream bodies, tokens or `Debug` output.
//! Domain-error `From` impls (explicit matches, no catch-all) arrive with the routes that need them.

use axum::{
    extract::{
        FromRequest, FromRequestParts, Request,
        rejection::{JsonRejection, PathRejection, QueryRejection},
    },
    http::{HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use bazaar_execution::{
    approval::Outcome, executor::ExecError, killswitch::ReleaseError, pipeline::PipelineError,
};
use serde::Serialize;
use utoipa::ToSchema;

/// Content type of every error body.
pub const PROBLEM_JSON: &str = "application/problem+json";

/// One field-level validation message (422).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct FieldError {
    /// Offending field.
    pub field: String,
    /// User-safe message.
    pub message: String,
}

/// RFC 9457 problem details with the stable machine key `code`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct Problem {
    /// `urn:bazaar:problem:<code>`.
    #[serde(rename = "type")]
    pub type_: String,
    /// Fixed title per code.
    pub title: String,
    /// HTTP status.
    pub status: u16,
    /// User-safe detail.
    pub detail: String,
    /// Stable machine key; the UI switches on this, never on `detail`.
    pub code: String,
    /// Validation errors (422 only; empty otherwise).
    pub errors: Vec<FieldError>,
}

/// Closed set of API errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    /// 400 malformed JSON or query.
    BadRequest,
    /// 401 no live session.
    Unauthenticated,
    /// 403 launch token wrong, used or expired.
    LaunchTokenInvalid,
    /// 403 Host not on the allowlist.
    ForbiddenHost,
    /// 403 Origin missing or not allowed.
    ForbiddenOrigin,
    /// 403 CSRF header missing or wrong.
    CsrfInvalid,
    /// 403 any other refusal (not a guard); generic.
    Forbidden,
    /// 404 unknown API path.
    NotFound,
    /// 405.
    MethodNotAllowed,
    /// 408.
    RequestTimeout,
    /// 413.
    PayloadTooLarge,
    /// 415.
    UnsupportedMediaType,
    /// 422 with per-field messages.
    Validation(Vec<FieldError>),
    /// 429.
    RateLimited,
    /// 409 a second decision on an already decided approval.
    AlreadyDecided,
    /// 503 the trading pipeline is stopped or not wired; nothing was done.
    PipelineClosed,
    /// 503 a store or audit write failed; nothing changed, safe to retry.
    StorageUnavailable,
    /// 500; the cause is logged once by the caller, never sent.
    Internal,
}

impl ApiError {
    /// HTTP status.
    #[must_use]
    pub fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest => StatusCode::BAD_REQUEST,
            Self::Unauthenticated => StatusCode::UNAUTHORIZED,
            Self::Forbidden
            | Self::LaunchTokenInvalid
            | Self::ForbiddenHost
            | Self::ForbiddenOrigin
            | Self::CsrfInvalid => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
            Self::RequestTimeout => StatusCode::REQUEST_TIMEOUT,
            Self::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::UnsupportedMediaType => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Self::Validation(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            Self::AlreadyDecided => StatusCode::CONFLICT,
            Self::PipelineClosed | Self::StorageUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            Self::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Stable machine key.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::BadRequest => "bad_request",
            Self::Unauthenticated => "unauthenticated",
            Self::LaunchTokenInvalid => "launch_token_invalid",
            Self::ForbiddenHost => "forbidden_host",
            Self::ForbiddenOrigin => "forbidden_origin",
            Self::CsrfInvalid => "csrf_invalid",
            Self::Forbidden => "forbidden",
            Self::NotFound => "not_found",
            Self::MethodNotAllowed => "method_not_allowed",
            Self::RequestTimeout => "request_timeout",
            Self::PayloadTooLarge => "payload_too_large",
            Self::UnsupportedMediaType => "unsupported_media_type",
            Self::Validation(_) => "validation_error",
            Self::RateLimited => "rate_limited",
            Self::AlreadyDecided => "already_decided",
            Self::PipelineClosed => "pipeline_closed",
            Self::StorageUnavailable => "storage_unavailable",
            Self::Internal => "internal",
        }
    }

    fn title(&self) -> &'static str {
        match self {
            Self::BadRequest => "Malformed request",
            Self::Unauthenticated => "Login required",
            Self::LaunchTokenInvalid => "Launch token invalid",
            Self::ForbiddenHost => "Host not allowed",
            Self::ForbiddenOrigin => "Origin not allowed",
            Self::CsrfInvalid => "CSRF token invalid",
            Self::Forbidden => "Forbidden",
            Self::NotFound => "Not found",
            Self::MethodNotAllowed => "Method not allowed",
            Self::RequestTimeout => "Request timed out",
            Self::PayloadTooLarge => "Payload too large",
            Self::UnsupportedMediaType => "Unsupported media type",
            Self::Validation(_) => "Validation failed",
            Self::RateLimited => "Too many requests",
            Self::AlreadyDecided => "Already decided",
            Self::PipelineClosed => "Trading pipeline unavailable",
            Self::StorageUnavailable => "Storage unavailable",
            Self::Internal => "Internal error",
        }
    }

    fn detail(&self) -> &'static str {
        match self {
            Self::BadRequest => "The request could not be parsed.",
            Self::Unauthenticated => "No active session. Open the launch link again.",
            Self::LaunchTokenInvalid => "The launch token is invalid, used or expired.",
            Self::ForbiddenHost => "This Host header is not allowed.",
            Self::ForbiddenOrigin => "This Origin is not allowed.",
            Self::CsrfInvalid => "The CSRF token is missing or wrong.",
            Self::Forbidden => "This request is not allowed.",
            Self::NotFound => "No such API route.",
            Self::MethodNotAllowed => "This method is not allowed on this route.",
            Self::RequestTimeout => "The request took too long.",
            Self::PayloadTooLarge => "The request body is too large.",
            Self::UnsupportedMediaType => "Content-Type must be application/json.",
            Self::Validation(_) => "One or more fields are invalid.",
            Self::RateLimited => "Slow down and retry shortly.",
            Self::AlreadyDecided => "This approval was already decided; nothing was sent again.",
            Self::PipelineClosed => "The trading pipeline is not running. Nothing was done.",
            Self::StorageUnavailable => "A storage write failed. Nothing changed; retry shortly.",
            Self::Internal => "Something went wrong on the server.",
        }
    }

    /// The wire body.
    #[must_use]
    pub fn problem(&self) -> Problem {
        Problem {
            type_: format!("urn:bazaar:problem:{}", self.code()),
            title: self.title().to_owned(),
            status: self.status().as_u16(),
            detail: self.detail().to_owned(),
            code: self.code().to_owned(),
            errors: match self {
                Self::Validation(e) => e.clone(),
                _ => Vec::new(),
            },
        }
    }

    /// Map a bare status (empty or plain-text 4xx/5xx) to the closest error.
    #[must_use]
    pub fn from_status(status: StatusCode) -> Self {
        match status {
            StatusCode::BAD_REQUEST => Self::BadRequest,
            StatusCode::UNAUTHORIZED => Self::Unauthenticated,
            StatusCode::NOT_FOUND => Self::NotFound,
            StatusCode::METHOD_NOT_ALLOWED => Self::MethodNotAllowed,
            StatusCode::REQUEST_TIMEOUT => Self::RequestTimeout,
            StatusCode::PAYLOAD_TOO_LARGE => Self::PayloadTooLarge,
            StatusCode::UNSUPPORTED_MEDIA_TYPE => Self::UnsupportedMediaType,
            StatusCode::UNPROCESSABLE_ENTITY => Self::Validation(Vec::new()),
            StatusCode::TOO_MANY_REQUESTS => Self::RateLimited,
            StatusCode::FORBIDDEN => Self::Forbidden,
            s if s.is_client_error() => Self::BadRequest,
            _ => Self::Internal,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = serde_json::to_vec(&self.problem()).unwrap_or_default();
        let mut res = (self.status(), body).into_response();
        res.headers_mut()
            .insert(header::CONTENT_TYPE, HeaderValue::from_static(PROBLEM_JSON));
        res
    }
}

impl From<PipelineError> for ApiError {
    /// Explicit match, no catch-all: a new pipeline error breaks the build until it is mapped.
    fn from(e: PipelineError) -> Self {
        match e {
            PipelineError::TooLong => Self::Validation(vec![FieldError {
                field: "text".to_owned(),
                message: "Too long (500 bytes at most).".to_owned(),
            }]),
            PipelineError::Closed => Self::PipelineClosed,
            PipelineError::Exec(ExecError::Storage)
            | PipelineError::Release(ReleaseError::Storage) => Self::StorageUnavailable,
            PipelineError::Release(ReleaseError::WrongPhrase) => {
                Self::Validation(vec![FieldError {
                    field: "phrase".to_owned(),
                    message: "The release phrase does not match.".to_owned(),
                }])
            }
            PipelineError::Exec(
                ExecError::InvalidInput(_)
                | ExecError::KillSwitch
                | ExecError::BreakerTripped
                | ExecError::RateLimited
                | ExecError::InsufficientFunds
                | ExecError::MarginUnavailable
                | ExecError::Unknown,
            ) => Self::Internal,
        }
    }
}

/// A decision on a finished intent is a 409; any other outcome passes through unchanged.
pub fn reject_repeat(o: Outcome) -> Result<Outcome, ApiError> {
    match o {
        Outcome::AlreadyDecided(_) => Err(ApiError::AlreadyDecided),
        other => Ok(other),
    }
}

impl From<JsonRejection> for ApiError {
    fn from(r: JsonRejection) -> Self {
        Self::from_status(r.status())
    }
}
impl From<QueryRejection> for ApiError {
    fn from(r: QueryRejection) -> Self {
        Self::from_status(r.status())
    }
}
impl From<PathRejection> for ApiError {
    fn from(r: PathRejection) -> Self {
        Self::from_status(r.status())
    }
}

/// JSON body extractor: syntax 400, schema 422, wrong content type 415, oversize 413 as problem+json.
#[derive(Debug, FromRequest)]
#[from_request(via(axum::Json), rejection(ApiError))]
pub struct ApiJson<T>(pub T);

/// Query extractor with problem+json rejections.
#[derive(Debug, FromRequestParts)]
#[from_request(via(axum::extract::Query), rejection(ApiError))]
pub struct ApiQuery<T>(pub T);

/// Path extractor with problem+json rejections.
#[derive(Debug, FromRequestParts)]
#[from_request(via(axum::extract::Path), rejection(ApiError))]
pub struct ApiPath<T>(pub T);

/// Fallback for unknown `/api/*` paths: 404 problem+json, never `index.html`.
#[allow(clippy::unused_async)] // axum handlers must be async fns
pub async fn unknown_api() -> ApiError {
    ApiError::NotFound
}

/// Backstop: any 4xx/5xx that is not already problem+json (empty 405/408/413, plain-text
/// rejections) becomes problem+json.
pub async fn problem_backstop(req: Request, next: Next) -> Response {
    let res = next.run(req).await;
    let status = res.status();
    if !(status.is_client_error() || status.is_server_error()) {
        return res;
    }
    let is_problem = res
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with(PROBLEM_JSON));
    if is_problem {
        return res;
    }
    ApiError::from_status(status).into_response()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use axum::{Router, body::Body, routing::post};
    use http_body_util::BodyExt;
    use serde::Deserialize;
    use tower::ServiceExt;

    #[derive(Debug, Deserialize)]
    struct In {
        qty: u32,
    }

    async fn echo(ApiJson(i): ApiJson<In>) -> String {
        i.qty.to_string()
    }
    async fn q(ApiQuery(i): ApiQuery<In>) -> String {
        i.qty.to_string()
    }

    fn app() -> Router {
        Router::new()
            .route("/echo", post(echo).get(q))
            .route(
                "/empty413",
                post(|| async { StatusCode::PAYLOAD_TOO_LARGE }),
            )
            .layer(axum::middleware::from_fn(problem_backstop))
    }

    async fn run(req: Request<Body>) -> (StatusCode, String, serde_json::Value) {
        let res = app().oneshot(req).await.unwrap();
        let status = res.status();
        let ct = res
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            ct,
            serde_json::from_slice(&bytes).unwrap_or_default(),
        )
    }

    fn post_json(ct: Option<&str>, body: &str) -> Request<Body> {
        let mut b = Request::builder().method("POST").uri("/echo");
        if let Some(ct) = ct {
            b = b.header(header::CONTENT_TYPE, ct);
        }
        b.body(Body::from(body.to_owned())).unwrap()
    }

    #[tokio::test]
    async fn json_syntax_error_is_400_problem() {
        let (s, ct, j) = run(post_json(Some("application/json"), "{nope")).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        assert_eq!(ct, PROBLEM_JSON);
        assert_eq!(j["code"], "bad_request");
    }

    #[tokio::test]
    async fn json_schema_error_is_422_problem() {
        let (s, _, j) = run(post_json(Some("application/json"), r#"{"qty":"x"}"#)).await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(j["code"], "validation_error");
    }

    #[tokio::test]
    async fn wrong_content_type_is_415_problem() {
        let (s, _, j) = run(post_json(Some("text/plain"), r#"{"qty":1}"#)).await;
        assert_eq!(s, StatusCode::UNSUPPORTED_MEDIA_TYPE);
        assert_eq!(j["code"], "unsupported_media_type");
    }

    #[tokio::test]
    async fn valid_json_passes() {
        let (s, _, _) = run(post_json(Some("application/json"), r#"{"qty":1}"#)).await;
        assert_eq!(s, StatusCode::OK);
    }

    #[tokio::test]
    async fn bad_query_is_problem() {
        let req = Request::builder()
            .uri("/echo?qty=abc")
            .body(Body::empty())
            .unwrap();
        let (s, ct, j) = run(req).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        assert_eq!(ct, PROBLEM_JSON);
        assert_eq!(j["code"], "bad_request");
    }

    #[tokio::test]
    async fn backstop_converts_empty_413_and_405() {
        let req = Request::builder()
            .method("POST")
            .uri("/empty413")
            .body(Body::empty())
            .unwrap();
        let (s, ct, j) = run(req).await;
        assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(ct, PROBLEM_JSON);
        assert_eq!(j["code"], "payload_too_large");

        let req = Request::builder()
            .method("DELETE")
            .uri("/echo")
            .body(Body::empty())
            .unwrap();
        let (s, ct, j) = run(req).await;
        assert_eq!(s, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(ct, PROBLEM_JSON);
        assert_eq!(j["code"], "method_not_allowed");
    }

    #[test]
    fn every_variant_has_unique_code_and_matching_status() {
        let all = [
            ApiError::BadRequest,
            ApiError::Unauthenticated,
            ApiError::LaunchTokenInvalid,
            ApiError::ForbiddenHost,
            ApiError::ForbiddenOrigin,
            ApiError::Forbidden,
            ApiError::CsrfInvalid,
            ApiError::NotFound,
            ApiError::MethodNotAllowed,
            ApiError::RequestTimeout,
            ApiError::PayloadTooLarge,
            ApiError::UnsupportedMediaType,
            ApiError::Validation(vec![FieldError {
                field: "qty".into(),
                message: "zero".into(),
            }]),
            ApiError::RateLimited,
            ApiError::Internal,
        ];
        let mut codes: Vec<_> = all.iter().map(ApiError::code).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), all.len());
        for e in &all {
            let p = e.problem();
            assert_eq!(p.status, e.status().as_u16());
            assert_eq!(p.type_, format!("urn:bazaar:problem:{}", p.code));
        }
        assert_eq!(all[12].problem().errors.len(), 1);
    }

    #[tokio::test]
    async fn unknown_api_is_404_problem() {
        let res = unknown_api().await.into_response();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        assert_eq!(res.headers()[header::CONTENT_TYPE], PROBLEM_JSON);
        let b = res.into_body().collect().await.unwrap().to_bytes();
        let j: serde_json::Value = serde_json::from_slice(&b).unwrap();
        assert_eq!(j["code"], "not_found");
    }

    async fn by_id(ApiPath(id): ApiPath<u32>) -> String {
        id.to_string()
    }

    #[tokio::test]
    async fn api_path_rejection_is_problem() {
        let app = Router::new().route("/item/{id}", axum::routing::get(by_id));
        let ok = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/item/7")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
        let bad = app
            .oneshot(
                Request::builder()
                    .uri("/item/abc")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
        assert_eq!(bad.headers()[header::CONTENT_TYPE], PROBLEM_JSON);
        let b = bad.into_body().collect().await.unwrap().to_bytes();
        let j: serde_json::Value = serde_json::from_slice(&b).unwrap();
        assert_eq!(j["code"], "bad_request");
    }

    #[tokio::test]
    async fn backstop_bare_403_is_generic_forbidden() {
        let app = Router::new()
            .route("/x", axum::routing::get(|| async { StatusCode::FORBIDDEN }))
            .layer(axum::middleware::from_fn(problem_backstop));
        let res = app
            .oneshot(Request::builder().uri("/x").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        let b = res.into_body().collect().await.unwrap().to_bytes();
        let j: serde_json::Value = serde_json::from_slice(&b).unwrap();
        assert_eq!(j["code"], "forbidden");
    }

    #[test]
    fn internal_detail_is_generic() {
        let p = ApiError::Internal.problem();
        assert!(!p.detail.contains('/') && !p.detail.to_lowercase().contains("sql"));
    }

    #[test]
    fn pipeline_errors_map_to_problems() {
        let p = ApiError::from(PipelineError::TooLong);
        assert_eq!(p.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(p.problem().errors.len(), 1);
        let c = ApiError::from(PipelineError::Closed);
        assert_eq!(
            (c.status(), c.code()),
            (StatusCode::SERVICE_UNAVAILABLE, "pipeline_closed")
        );
        for e in [
            PipelineError::Exec(ExecError::Storage),
            PipelineError::Release(ReleaseError::Storage),
        ] {
            let a = ApiError::from(e);
            assert_eq!(
                (a.status(), a.code()),
                (StatusCode::SERVICE_UNAVAILABLE, "storage_unavailable")
            );
        }
        let w = ApiError::from(PipelineError::Release(ReleaseError::WrongPhrase));
        assert_eq!(w.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let i = ApiError::from(PipelineError::Exec(ExecError::Unknown));
        assert_eq!(i, ApiError::Internal);
    }

    #[test]
    fn repeat_decision_is_409_and_others_pass() {
        let first = Outcome::AlreadyDecided(Box::new(Outcome::Declined));
        let e = reject_repeat(first).unwrap_err();
        assert_eq!(
            (e.status(), e.code()),
            (StatusCode::CONFLICT, "already_decided")
        );
        assert_eq!(reject_repeat(Outcome::Declined).unwrap(), Outcome::Declined);
    }
}
