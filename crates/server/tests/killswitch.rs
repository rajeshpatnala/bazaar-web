//! Kill-switch routes (BZ-511 S4): in-process router over a temp store and the real pipeline.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::{path::PathBuf, sync::Arc};

use axum::{
    body::Body,
    http::{Request, Response, header},
};
use bazaar_cli::compose::{Ports, UnknownCost, build_pipeline, open_store};
use bazaar_core::{
    calendar::MarketCalendar,
    config::{CalendarConfig, load},
    ports::KillSwitchRepo,
    testutil::FakeClock,
};
use bazaar_execution::fills::RecordingExecutor;
use bazaar_server::{
    runtime::{LedgerPnl, NoBaseline, ProdBase, UnavailableQuotes},
    security::SESSION_ACTOR,
    state::{KillSwitchRead, KillSwitchReading, ReadFuture},
};
use bazaar_store::SqliteStore;
use chrono::TimeZone;
use common::{HOST, ORIGIN, TestApp, body_json};
use sqlx::{ConnectOptions, sqlite::SqliteConnectOptions};

const RISK: &str = include_str!("../../../fixtures/config/risk.toml");
const PHRASE: &str = "RELEASE KILL SWITCH";
const ENGAGE: &str = "/api/v1/killswitch/engage";
const RELEASE: &str = "/api/v1/killswitch/release";

struct StoreRead(SqliteStore);
impl KillSwitchRead for StoreRead {
    fn read(&self) -> ReadFuture<'_> {
        Box::pin(async move {
            match self.0.is_engaged().await {
                Ok(engaged) => KillSwitchReading {
                    engaged,
                    reason: None,
                    readable: true,
                    latched: false,
                },
                Err(_) => KillSwitchReading {
                    engaged: true,
                    reason: None,
                    readable: false,
                    latched: false,
                },
            }
        })
    }
}

struct Fixed(KillSwitchReading);
impl KillSwitchRead for Fixed {
    fn read(&self) -> ReadFuture<'_> {
        let r = self.0.clone();
        Box::pin(async move { r })
    }
}

struct Rig {
    app: TestApp,
    cookie: String,
    csrf: String,
    db: PathBuf,
    _dir: tempfile::TempDir,
}

fn calendar() -> MarketCalendar {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../config");
    MarketCalendar::from_config(&load::<CalendarConfig>(std::path::Path::new(dir)).unwrap())
}

async fn login(app: &TestApp) -> (String, String) {
    let cookie = app.login().await;
    let me = body_json(app.get("/api/v1/auth/me", Some(&cookie)).await).await;
    let csrf = me["csrf_token"].as_str().unwrap().to_owned();
    (cookie, csrf)
}

/// Real pipeline over a temp store; `fixed` replaces the read port (else it reads the store).
async fn rig_with(fixed: Option<KillSwitchReading>) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("t.db");
    let clock = Arc::new(FakeClock::at(
        chrono_tz::Asia::Kolkata
            .with_ymd_and_hms(2026, 11, 19, 11, 0, 0)
            .unwrap(),
    ));
    let store = open_store(&db).await.unwrap();
    let ports = Ports {
        executor: RecordingExecutor::default(),
        pnl: LedgerPnl::new(store.clone(), Arc::new(|| false), clock.clone()),
        quotes: UnavailableQuotes,
        base: ProdBase,
        clock: clock.clone(),
        calendar: calendar(),
        baseline: Arc::new(NoBaseline),
        cost: Arc::new(UnknownCost),
    };
    let mut pipe = build_pipeline(&store, RISK, ports).await.unwrap();
    pipe.reload().await.unwrap();
    let handle = pipe.spawn();
    let port: Arc<dyn KillSwitchRead> = match fixed {
        Some(f) => Arc::new(Fixed(f)),
        None => Arc::new(StoreRead(store)),
    };
    let app = TestApp::with_state(false, |s| s.with_pipeline(handle, port));
    let (cookie, csrf) = login(&app).await;
    Rig {
        app,
        cookie,
        csrf,
        db,
        _dir: dir,
    }
}

async fn rig() -> Rig {
    rig_with(None).await
}

async fn post(app: &TestApp, cookie: &str, csrf: &str, path: &str, body: &str) -> Response<Body> {
    app.send(
        Request::post(path)
            .header(header::HOST, HOST)
            .header(header::ORIGIN, ORIGIN)
            .header(header::COOKIE, cookie)
            .header("x-csrf-token", csrf)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_owned()))
            .unwrap(),
    )
    .await
}

impl Rig {
    async fn post(&self, path: &str, body: &str) -> Response<Body> {
        post(&self.app, &self.cookie, &self.csrf, path, body).await
    }

    async fn count(&self, sql: &str) -> i64 {
        let mut c = SqliteConnectOptions::new()
            .filename(&self.db)
            .read_only(true)
            .connect()
            .await
            .unwrap();
        // Test-only SQL written in this file.
        let r: (i64,) = sqlx::query_as(sqlx::AssertSqlSafe(sql.to_owned()))
            .fetch_one(&mut c)
            .await
            .unwrap();
        r.0
    }

    async fn state_rows(&self) -> i64 {
        self.count("SELECT COUNT(*) FROM kill_switch_events").await
    }

    async fn audit_rows(&self) -> i64 {
        self.count("SELECT COUNT(*) FROM audit_log WHERE kind LIKE 'kill_switch%'")
            .await
    }

    async fn exec(&self, sql: &str) {
        let mut c = SqliteConnectOptions::new()
            .filename(&self.db)
            .connect()
            .await
            .unwrap();
        sqlx::query(sqlx::AssertSqlSafe(sql.to_owned()))
            .execute(&mut c)
            .await
            .unwrap();
    }

    async fn get(&self) -> serde_json::Value {
        body_json(self.app.get("/api/v1/killswitch", Some(&self.cookie)).await).await
    }
}

#[tokio::test]
async fn engage_then_release_writes_one_state_and_one_audit_row_each() {
    let r = rig().await;
    assert_eq!(r.get().await["engaged"], false);
    let res = r.post(ENGAGE, r#"{"reason":"manual stop"}"#).await;
    assert_eq!(res.status(), 200);
    let v = body_json(res).await;
    assert_eq!(v["engaged"], true);
    assert_eq!(v["readable"], true);
    assert_eq!((r.state_rows().await, r.audit_rows().await), (1, 1));
    assert_eq!(r.get().await["engaged"], true);
    let by_session = r
        .count(&format!(
            "SELECT COUNT(*) FROM audit_log WHERE kind = 'kill_switch_engaged' AND actor = '{SESSION_ACTOR}'"
        ))
        .await;
    assert_eq!(by_session, 1, "actor comes from the session");

    let res = r
        .post(RELEASE, &format!(r#"{{"phrase":"{PHRASE}"}}"#))
        .await;
    assert_eq!(res.status(), 200);
    assert_eq!(body_json(res).await["engaged"], false);
    assert_eq!((r.state_rows().await, r.audit_rows().await), (2, 2));
}

#[tokio::test]
async fn second_engage_writes_no_second_row() {
    let r = rig().await;
    assert_eq!(r.post(ENGAGE, r#"{"reason":"a"}"#).await.status(), 200);
    assert_eq!(r.post(ENGAGE, r#"{"reason":"b"}"#).await.status(), 200);
    assert_eq!((r.state_rows().await, r.audit_rows().await), (1, 1));
}

#[tokio::test]
async fn wrong_empty_or_short_phrase_is_403_naming_the_rule_and_writes_nothing() {
    let r = rig().await;
    r.post(ENGAGE, r#"{"reason":"x"}"#).await;
    for phrase in ["", "RELEASE", "release kill switch", "RELEASE KILL SWITCH "] {
        let res = r
            .post(RELEASE, &format!(r#"{{"phrase":"{phrase}"}}"#))
            .await;
        assert_eq!(res.status(), 403, "{phrase:?}");
        assert_eq!(body_json(res).await["code"], "release_phrase_mismatch");
    }
    assert_eq!((r.state_rows().await, r.audit_rows().await), (1, 1));
    assert_eq!(r.get().await["engaged"], true);
}

#[tokio::test]
async fn body_actor_and_unknown_fields_are_rejected_and_nothing_written() {
    let r = rig().await;
    let res = r.post(ENGAGE, r#"{"reason":"x","actor":"mallory"}"#).await;
    assert_eq!(res.status(), 422);
    let res = r
        .post(RELEASE, &format!(r#"{{"phrase":"{PHRASE}","actor":"m"}}"#))
        .await;
    assert_eq!(res.status(), 422);
    assert_eq!((r.state_rows().await, r.audit_rows().await), (0, 0));
}

#[tokio::test]
async fn over_500_bytes_is_422_and_nothing_is_written() {
    let r = rig().await;
    let long = "x".repeat(501);
    let res = r.post(ENGAGE, &format!(r#"{{"reason":"{long}"}}"#)).await;
    assert_eq!(res.status(), 422);
    assert_eq!(body_json(res).await["code"], "validation_error");
    let res = r.post(RELEASE, &format!(r#"{{"phrase":"{long}"}}"#)).await;
    assert_eq!(res.status(), 422);
    assert_eq!((r.state_rows().await, r.audit_rows().await), (0, 0));
    // exactly 500 bytes is accepted
    let ok = "y".repeat(500);
    let res = r.post(ENGAGE, &format!(r#"{{"reason":"{ok}"}}"#)).await;
    assert_eq!(res.status(), 200);
    assert_eq!((r.state_rows().await, r.audit_rows().await), (1, 1));
}

#[tokio::test]
async fn failed_engage_write_is_503_not_200() {
    let r = rig().await;
    r.exec("CREATE TRIGGER fail_engage BEFORE INSERT ON kill_switch_events BEGIN SELECT RAISE(ABORT, 'x'); END").await;
    let res = r.post(ENGAGE, r#"{"reason":"x"}"#).await;
    assert_eq!(res.status(), 503);
    let v = body_json(res).await;
    assert_eq!(v["code"], "kill_switch_not_saved");
    assert!(
        v["detail"]
            .as_str()
            .unwrap()
            .contains("latched in this process")
    );
    assert_eq!(
        r.count("SELECT COUNT(*) FROM kill_switch WHERE engaged = 1")
            .await,
        0
    );
}

#[tokio::test]
async fn blank_reason_is_stored_as_unspecified() {
    let r = rig().await;
    assert_eq!(r.post(ENGAGE, r#"{"reason":""}"#).await.status(), 200);
    assert_eq!(
        r.count("SELECT COUNT(*) FROM kill_switch_events WHERE reason = 'unspecified'")
            .await,
        1
    );
}

#[tokio::test]
async fn release_when_not_engaged_is_200_and_writes_nothing() {
    let r = rig().await;
    let res = r
        .post(RELEASE, &format!(r#"{{"phrase":"{PHRASE}"}}"#))
        .await;
    assert_eq!(res.status(), 200);
    assert_eq!(body_json(res).await["engaged"], false);
    assert_eq!((r.state_rows().await, r.audit_rows().await), (0, 0));
}

#[tokio::test]
async fn correct_phrase_after_a_refused_attempt_releases() {
    let r = rig().await;
    r.post(ENGAGE, r#"{"reason":"x"}"#).await;
    let bad = r.post(RELEASE, r#"{"phrase":"nope"}"#).await;
    assert_eq!(bad.status(), 403);
    assert_eq!(r.get().await["engaged"], true);
    let ok = r
        .post(RELEASE, &format!(r#"{{"phrase":"{PHRASE}"}}"#))
        .await;
    assert_eq!(ok.status(), 200);
    assert_eq!(body_json(ok).await["engaged"], false);
    assert_eq!((r.state_rows().await, r.audit_rows().await), (2, 2));
}

#[tokio::test]
async fn failed_release_audit_write_is_503_and_stays_engaged() {
    let r = rig().await;
    r.post(ENGAGE, r#"{"reason":"x"}"#).await;
    r.exec("DROP TABLE audit_log").await;
    let res = r
        .post(RELEASE, &format!(r#"{{"phrase":"{PHRASE}"}}"#))
        .await;
    assert_eq!(res.status(), 503);
    assert_eq!(body_json(res).await["code"], "storage_unavailable");
    assert_eq!(r.state_rows().await, 1);
    assert_eq!(r.get().await["engaged"], true);
}

#[tokio::test]
async fn unreadable_state_reports_engaged_true_not_off() {
    let r = rig().await;
    r.exec("DROP TABLE kill_switch").await;
    let v = r.get().await;
    assert_eq!(v["engaged"], true);
    assert_eq!(v["readable"], false);
}

#[tokio::test]
async fn reason_is_escaped_on_the_wire() {
    let r = rig_with(Some(KillSwitchReading {
        engaged: true,
        reason: Some("\u{1b}[31mx\u{7}\ny\u{202e}z".into()),
        readable: true,
        latched: false,
    }))
    .await;
    let res = r.app.get("/api/v1/killswitch", Some(&r.cookie)).await;
    let raw = String::from_utf8(common::body_bytes(res).await).unwrap();
    assert!(!raw.contains('\u{1b}') && !raw.contains('\u{7}') && !raw.contains('\u{202e}'));
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(v["reason"], "\\x1b[31mx\\x07\\x0ay\\u{202e}z");
}

#[tokio::test]
async fn unwired_is_503_pipeline_closed_on_all_routes() {
    let app = TestApp::new(false);
    let (cookie, csrf) = login(&app).await;
    let g = app.get("/api/v1/killswitch", Some(&cookie)).await;
    assert_eq!(g.status(), 503);
    assert_eq!(body_json(g).await["code"], "pipeline_closed");
    for (p, b) in [
        (ENGAGE, r#"{"reason":"x"}"#),
        (RELEASE, r#"{"phrase":"abc"}"#),
    ] {
        let res = post(&app, &cookie, &csrf, p, b).await;
        assert_eq!(res.status(), 503, "{p}");
        assert_eq!(body_json(res).await["code"], "pipeline_closed");
    }
}

#[tokio::test]
async fn no_session_or_no_csrf_is_rejected_on_both_writes_and_changes_nothing() {
    let r = rig().await;
    r.post(ENGAGE, r#"{"reason":"x"}"#).await;
    let body_release = format!(r#"{{"phrase":"{PHRASE}"}}"#);
    for (path, body) in [
        (ENGAGE, r#"{"reason":"y"}"#),
        (RELEASE, body_release.as_str()),
    ] {
        let base = || {
            Request::post(path)
                .header(header::HOST, HOST)
                .header(header::ORIGIN, ORIGIN)
                .header(header::CONTENT_TYPE, "application/json")
        };
        let res = r
            .app
            .send(base().body(Body::from(body.to_owned())).unwrap())
            .await;
        assert_eq!(res.status(), 401, "{path}");
        let res = r
            .app
            .send(
                base()
                    .header(header::COOKIE, &r.cookie)
                    .body(Body::from(body.to_owned()))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), 403, "{path}");
        assert_eq!(body_json(res).await["code"], "csrf_invalid");
    }
    assert_eq!((r.state_rows().await, r.audit_rows().await), (1, 1));
    assert_eq!(r.get().await["engaged"], true);
}

/// Source scan: fragile by nature (a renamed helper evades it). The real guard is the type-level
/// boundary: `AppState` holds no repo, only the `PipelineHandle` and a read-only port.
#[test]
fn routes_use_the_pipeline_handle_not_the_repo_and_escape_the_reason() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/routes/killswitch.rs"
    ))
    .unwrap();
    let prod = src.split("#[cfg(test)]").next().unwrap();
    assert!(prod.contains("set_kill_switch("));
    assert!(!prod.contains("KillSwitchRepo") && !prod.contains("bazaar_store"));
    assert!(prod.contains("escape_control(&body.reason)"));
}
