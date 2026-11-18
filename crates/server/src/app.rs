//! Router assembly (slice F1/server-app).
//!
//! Layers, outermost first: `security_headers`, `TraceLayer` (path only), `host_guard`,
//! `origin_guard`, `problem_backstop`, then the request timeout and body limit (so a 408 or 413
//! also becomes problem+json). Public: `/healthz`, `/auth`; `/api/v1/**` needs a session.

use std::time::Duration;

use axum::{
    Router,
    extract::DefaultBodyLimit,
    http::StatusCode,
    middleware::{from_fn, from_fn_with_state},
    routing::{any, get},
};
use tower_http::{
    services::{ServeDir, ServeFile},
    timeout::TimeoutLayer,
    trace::TraceLayer,
};

use crate::{error, openapi, routes, security, state::AppState};

const BODY_LIMIT: usize = 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const NO_UI_HINT: &str = "Web UI not built: run `pnpm -C web build`.";

/// Build the full router.
pub fn build(state: AppState) -> Router {
    let (api, _doc) = openapi::router().split_for_parts();
    let api = api
        .route_layer(from_fn_with_state(
            state.security.clone(),
            security::csrf_guard,
        ))
        .route_layer(from_fn_with_state(
            state.security.clone(),
            security::session_guard,
        ));

    let public = Router::new()
        .route("/healthz", get(routes::status::healthz))
        .route("/auth", get(security::auth))
        .route("/api/{*rest}", any(error::unknown_api));

    let index = state.web_dist.join("index.html");
    let router = Router::new()
        .merge(public)
        .merge(api)
        .nest_service("/assets", ServeDir::new(state.web_dist.join("assets")));
    let router = if index.is_file() {
        router.fallback_service(ServeDir::new(&state.web_dist).fallback(ServeFile::new(index)))
    } else {
        tracing::warn!("web/dist missing; the API still runs");
        router.fallback(|| async { NO_UI_HINT })
    };

    // The LAST `.layer` is outermost.
    router
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .layer(from_fn(error::problem_backstop))
        .layer(from_fn_with_state(
            state.security.clone(),
            security::origin_guard,
        ))
        .layer(from_fn_with_state(
            state.security.clone(),
            security::host_guard,
        ))
        .layer(TraceLayer::new_for_http().make_span_with(security::path_only_span))
        .layer(from_fn(security::security_headers))
        .with_state(state)
}
