//! `OpenAPI` document, built from the same router that serves traffic (slice F1/server-app).

use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;

use crate::{
    dto::{
        AlarmDto, ApprovalDto, ApprovalListDto, DecisionDto, DecisionOutcomeDto,
        DecisionRequestDto, KillSwitchDto, OutcomeKind,
    },
    error::{FieldError, Problem},
    routes,
    state::AppState,
};

#[derive(OpenApi)]
#[openapi(
    info(title = "Bazaar API", version = "1"),
    components(schemas(
        Problem,
        FieldError,
        AlarmDto,
        DecisionRequestDto,
        DecisionDto,
        ApprovalDto,
        ApprovalListDto,
        DecisionOutcomeDto,
        OutcomeKind,
        KillSwitchDto
    ))
)]
struct ApiDoc;

/// All `/api/v1/**` session routes with their documentation.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::with_openapi(ApiDoc::openapi()).nest("/api/v1", routes::api())
}

/// The generated document (`bazaar-server openapi` prints it; needs no config or socket).
#[must_use]
pub fn document() -> utoipa::openapi::OpenApi {
    let (_, doc) = router().split_for_parts();
    doc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_path_is_under_api_v1_and_version_is_fixed() {
        let doc = document();
        assert!(!doc.paths.paths.is_empty());
        assert!(doc.paths.paths.keys().all(|p| p.starts_with("/api/v1/")));
        assert_eq!(doc.info.version, "1");
    }

    /// The path set is an allowlist: adding a route must edit this list (reviewers check that
    /// none places an order).
    #[test]
    fn paths_equal_the_allowlist_and_none_places_an_order() {
        let doc = document();
        let mut got: Vec<&str> = doc.paths.paths.keys().map(String::as_str).collect();
        got.sort_unstable();
        assert_eq!(
            got,
            [
                "/api/v1/approvals",
                "/api/v1/approvals/{id}/decision",
                "/api/v1/auth/me",
                "/api/v1/status"
            ]
        );
    }
}
