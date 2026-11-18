# 4. Router and layers

(From `bz-web-api` SKILL.md; open when the task touches this area.)


```rust
// app.rs
pub fn build(state: AppState) -> Router {
    let (api, _doc) = openapi::router().split_for_parts();          // /api/v1/** (session routes + /ws)
    let api = api
        .route_layer(from_fn_with_state(state.security.clone(), security::csrf_guard))     // inner
        .route_layer(from_fn_with_state(state.security.clone(), security::session_guard)); // outer: runs first

    let public = Router::new()
        .route("/healthz", get(status::healthz))
        .route("/auth", get(security::auth))                          // launch-token exchange
        .route("/api/v1/kite/callback", get(routes::kite::callback))  // nonce-checked, no cookie
        .route("/api/{*rest}", any(error::unknown_api));              // 404 problem+json, never index.html

    Router::new()
        .merge(public)
        .merge(api)
        .nest_service("/assets", static_assets(&state))               // real 404 for a missing asset
        .fallback_service(spa(&state))                                // ServeDir + ServeFile(index.html) fallback
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .layer(
            ServiceBuilder::new()                                     // first = outermost
                .layer(TraceLayer::new_for_http().make_span_with(security::path_only_span)) // never the query: /auth?token=
                .layer(security::security_headers())                  // CSP, nosniff, no-referrer, no-store on /api
                .layer(TimeoutLayer::with_status_code(StatusCode::REQUEST_TIMEOUT, Duration::from_secs(30)))
                .layer(from_fn(error::problem_backstop))              // empty 4xx/5xx (408, 405, 413) -> problem+json
                .layer(from_fn_with_state(state.security.clone(), security::host_guard))
                .layer(from_fn_with_state(state.security.clone(), security::origin_guard)), // non-GET + WS upgrade
        )
        .with_state(state)
}
```

Notes: `route_layer` (not `layer`) so 404/405 do not demand a session; later `route_layer` calls wrap earlier ones, hence CSRF first. Headers sit outside the Host/Origin checks so rejections also carry them. There is no `CorsLayer`, `Access-Control-*` header or `cors` feature. Hashed `/assets/*` may get `Cache-Control: public, max-age=31536000, immutable`; `index.html` gets `no-cache`. A missing `web/dist` logs a warning, the API still runs and `/` returns a plain hint ("run `pnpm -C web build`"). `TimeoutLayer::new` is deprecated in tower-http 0.7; use `with_status_code`. Compile-check `method_not_allowed_fallback` (problem+json for 405) against the pinned axum.
