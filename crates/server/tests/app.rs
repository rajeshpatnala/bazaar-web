//! Router-level contract tests (real layer stack, in-process, no socket).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::{
    body::Body,
    http::{Request, header},
};
use bazaar_server::{openapi, security::EXEMPT_PATHS};
use common::{HOST, ORIGIN, TestApp, body_bytes, body_json};

#[tokio::test]
async fn healthz_needs_no_session() {
    let app = TestApp::new(true);
    let res = app.get("/healthz", None).await;
    assert_eq!(res.status(), 200);
    assert_eq!(body_json(res).await, serde_json::json!({"status": "ok"}));
}

#[tokio::test]
async fn headers_present_on_ok_and_rejection() {
    let app = TestApp::new(true);
    let ok = app.get("/healthz", None).await;
    assert!(ok.headers().contains_key(header::CONTENT_SECURITY_POLICY));
    let bad = app
        .send(
            Request::get("/healthz")
                .header(header::HOST, "evil.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(bad.status(), 403);
    assert!(bad.headers().contains_key(header::CONTENT_SECURITY_POLICY));
    assert_eq!(body_json(bad).await["code"], "forbidden_host");
}

#[tokio::test]
async fn every_openapi_path_is_401_without_cookie() {
    let app = TestApp::new(true);
    let doc = openapi::document();
    assert!(!doc.paths.paths.is_empty());
    for path in doc.paths.paths.keys() {
        assert!(!EXEMPT_PATHS.contains(&path.as_str()), "{path} is exempt");
        let res = app.get(path, None).await;
        assert_eq!(res.status(), 401, "{path}");
        assert_eq!(body_json(res).await["code"], "unauthenticated", "{path}");
    }
}

#[tokio::test]
async fn auth_me_and_status_work_after_exchange() {
    let app = TestApp::new(true);
    let cookie = app.login().await;
    let me = app.get("/api/v1/auth/me", Some(&cookie)).await;
    assert_eq!(me.status(), 200);
    let v = body_json(me).await;
    assert_eq!(v["mode"], "paper");
    assert_eq!(v["live_enabled"], false);
    assert_eq!(v["csrf_token"].as_str().unwrap().len(), 64);

    let st = app.get("/api/v1/status", Some(&cookie)).await;
    assert_eq!(st.status(), 200);
    let v = body_json(st).await;
    assert_eq!(v["server_time"], "2026-01-05T10:00:00+05:30");
    assert_eq!(v["mode"], "paper");
}

#[tokio::test]
async fn auth_cookie_flags_and_single_use() {
    let app = TestApp::new(true);
    let url = app.security.launch_url(common::PORT, false).unwrap();
    let path = url.strip_prefix(ORIGIN).unwrap().to_owned();
    let res = app.get(&path, None).await;
    let sc = res.headers()[header::SET_COOKIE].to_str().unwrap();
    assert!(sc.contains("HttpOnly") && sc.contains("SameSite=Strict"));
    let again = app.get(&path, None).await;
    assert_eq!(again.status(), 403);
    assert_eq!(body_json(again).await["code"], "launch_token_invalid");
}

#[tokio::test]
async fn post_without_origin_or_csrf_is_refused() {
    let app = TestApp::new(true);
    let cookie = app.login().await;
    let no_origin = Request::post("/api/v1/status")
        .header(header::HOST, HOST)
        .header(header::COOKIE, &cookie)
        .body(Body::empty())
        .unwrap();
    let res = app.send(no_origin).await;
    assert_eq!(res.status(), 403);
    assert_eq!(body_json(res).await["code"], "forbidden_origin");

    let no_csrf = Request::post("/api/v1/status")
        .header(header::HOST, HOST)
        .header(header::ORIGIN, ORIGIN)
        .header(header::COOKIE, &cookie)
        .body(Body::empty())
        .unwrap();
    let res = app.send(no_csrf).await;
    assert_eq!(res.status(), 403);
    assert_eq!(body_json(res).await["code"], "csrf_invalid");
}

#[tokio::test]
async fn unknown_api_is_problem_404_not_index() {
    let app = TestApp::new(true);
    let res = app.get("/api/v1/nope", None).await;
    assert_eq!(res.status(), 404);
    assert_eq!(
        res.headers()[header::CONTENT_TYPE],
        "application/problem+json"
    );
}

#[tokio::test]
async fn static_ui_spa_fallback_and_assets() {
    let app = TestApp::new(true);
    let res = app.get("/settings", None).await;
    assert_eq!(res.status(), 200);
    assert_eq!(body_bytes(res).await, b"<html>bazaar-ui</html>");
    let js = app.get("/assets/app.js", None).await;
    assert_eq!(js.status(), 200);
    let missing = app.get("/assets/missing.js", None).await;
    assert_eq!(missing.status(), 404);
}

#[tokio::test]
async fn missing_dist_gives_plain_hint_and_api_still_runs() {
    let app = TestApp::new(false);
    let res = app.get("/", None).await;
    assert_eq!(res.status(), 200);
    assert!(
        String::from_utf8(body_bytes(res).await)
            .unwrap()
            .contains("pnpm -C web build")
    );
    assert_eq!(app.get("/healthz", None).await.status(), 200);
}

#[tokio::test]
async fn openapi_contract() {
    let json = serde_json::to_value(openapi::document()).unwrap();
    assert_eq!(json["info"]["version"], "1");
    let paths = json["paths"].as_object().unwrap();
    assert!(paths.contains_key("/api/v1/auth/me"));
    assert!(paths.contains_key("/api/v1/status"));
    assert!(paths.keys().all(|p| p.starts_with("/api/v1/")));
    assert!(json["components"]["schemas"]["Problem"].is_object());
}
