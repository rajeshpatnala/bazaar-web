//! F10 T2: wrong-CSRF-token, foreign-Origin and foreign-Host negatives on the three write routes
//! the F10 pipeline owns (approval decision, kill-switch engage and release), over the real
//! pipeline and a temp SQLite store. The existing route tests cover "no session" and "no token";
//! these cover a token that is present but wrong and an Origin/Host that is not ours. Each case
//! asserts WHICH rule refused (`code`) and that nothing moved (order, approvals, kill switch,
//! audit rows).

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
    clock::Clock,
    config::{CalendarConfig, load},
    ports::{AuditDraft, OrderRecord, OrderRepo},
    testutil::FakeClock,
};
use bazaar_execution::{
    audit::chain_hash,
    fills::RecordingExecutor,
    types::{IntentId, OrderIntent, OrderKind, Product},
};
use bazaar_risk::types::Side;
use bazaar_server::{
    runtime::{LedgerPnl, NoBaseline, ProdBase, StorePending, UnavailableQuotes},
    state::{KillSwitchRead, KillSwitchReading, ReadFuture},
};
use chrono::TimeZone;
use common::{HOST, ORIGIN, TestApp, body_json};
use rust_decimal::Decimal;
use sqlx::{ConnectOptions, sqlite::SqliteConnectOptions};

const RISK: &str = include_str!("../../../fixtures/config/risk.toml");
const PHRASE: &str = "RELEASE KILL SWITCH";

struct QuietKill;
impl KillSwitchRead for QuietKill {
    fn read(&self) -> ReadFuture<'_> {
        Box::pin(async {
            KillSwitchReading {
                engaged: false,
                reason: None,
                readable: true,
                latched: false,
            }
        })
    }
}

struct Rig {
    app: TestApp,
    cookie: String,
    csrf: String,
    rec: RecordingExecutor,
    db: PathBuf,
    _dir: tempfile::TempDir,
}

async fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("t.db");
    let clock = Arc::new(FakeClock::at(
        chrono_tz::Asia::Kolkata
            .with_ymd_and_hms(2026, 11, 19, 11, 0, 0)
            .unwrap(),
    ));
    let store = open_store(&db).await.unwrap();
    let intent = OrderIntent {
        id: IntentId(1),
        symbol: "ABC".into(),
        side: Side::Buy,
        qty: 1,
        kind: OrderKind::Limit(Decimal::from(50)),
        product: Product::Mis,
        reduce_only: false,
        created_at: clock.now_ist(),
    };
    OrderRepo::insert(
        &store,
        OrderRecord {
            intent_id: 1,
            version: 0,
            state: "pending_approval".into(),
            payload: intent.to_payload(),
        },
        AuditDraft {
            ts_ist_rfc3339: clock.now_ist().to_rfc3339(),
            actor: "seed".into(),
            kind: "proposed".into(),
            payload: format!("{{\"intent\":\"{}\"}}", IntentId(1)),
        },
        chain_hash,
    )
    .await
    .unwrap();
    let rec = RecordingExecutor::default();
    let calendar = MarketCalendar::from_config(
        &load::<CalendarConfig>(std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../config"
        )))
        .unwrap(),
    );
    let ports = Ports {
        executor: rec.clone(),
        pnl: LedgerPnl::new(store.clone(), Arc::new(|| false), clock.clone()),
        quotes: UnavailableQuotes,
        base: ProdBase,
        clock: clock.clone(),
        calendar,
        baseline: Arc::new(NoBaseline),
        cost: Arc::new(UnknownCost),
    };
    let mut pipe = build_pipeline(&store, RISK, ports).await.unwrap();
    pipe.reload().await.unwrap();
    let handle = pipe.spawn();
    let pending = Arc::new(StorePending {
        store,
        clock,
        ttl: chrono::Duration::seconds(300),
    });
    let app = TestApp::with_state(false, |s| {
        s.with_pipeline(handle, Arc::new(QuietKill))
            .with_pending(pending)
    });
    let cookie = app.login().await;
    let me = body_json(app.get("/api/v1/auth/me", Some(&cookie)).await).await;
    let csrf = me["csrf_token"].as_str().unwrap().to_owned();
    Rig {
        app,
        cookie,
        csrf,
        rec,
        db,
        _dir: dir,
    }
}

impl Rig {
    /// (order state, approval rows, kill-switch event rows, audit rows): everything a refused
    /// write must leave alone.
    async fn snapshot(&self) -> (String, i64, i64, i64) {
        let mut c = SqliteConnectOptions::new()
            .filename(&self.db)
            .read_only(true)
            .connect()
            .await
            .unwrap();
        let state: (String,) = sqlx::query_as("SELECT state FROM orders WHERE order_id = '1'")
            .fetch_one(&mut c)
            .await
            .unwrap();
        let a: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM approvals")
            .fetch_one(&mut c)
            .await
            .unwrap();
        let k: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM kill_switch_events")
            .fetch_one(&mut c)
            .await
            .unwrap();
        let l: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM audit_log")
            .fetch_one(&mut c)
            .await
            .unwrap();
        (state.0, a.0, k.0, l.0)
    }

    fn req(&self, path: &str) -> axum::http::request::Builder {
        Request::post(path)
            .header(header::CONTENT_TYPE, "application/json")
            .header("idempotency-key", "key-0000001")
            .header(header::COOKIE, &self.cookie)
    }

    /// The request with our own Host, Origin and CSRF token.
    async fn send_right(&self, (path, body): (String, String)) -> Response<Body> {
        self.app
            .send(
                self.req(&path)
                    .header(header::HOST, HOST)
                    .header(header::ORIGIN, ORIGIN)
                    .header("x-csrf-token", &self.csrf)
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
    }

    /// Engage through the real route, so that a release let through would write a row. Returns
    /// the snapshot to compare against.
    async fn engaged_snapshot(&self) -> (String, i64, i64, i64) {
        let [_, engage, _] = routes();
        assert_eq!(self.send_right(engage).await.status(), 200);
        let s = self.snapshot().await;
        assert_eq!(s.2, 1, "engage wrote its event");
        s
    }

    /// The snapshot a refusal phase must leave unchanged.
    async fn phase_snapshot(&self, engaged: bool) -> (String, i64, i64, i64) {
        if engaged {
            self.engaged_snapshot().await
        } else {
            self.snapshot().await
        }
    }
}

/// The refusal tests' two phases, as (engage the switch first, routes). Engage is tried while
/// un-engaged: engaging an engaged switch writes nothing, so "nothing moved" could not fail.
fn phases() -> [(bool, Vec<(String, String)>); 2] {
    let [decision, engage, release] = routes();
    [(false, vec![engage]), (true, vec![decision, release])]
}

/// Decision, engage, release.
fn routes() -> [(String, String); 3] {
    [
        (
            format!("/api/v1/approvals/{:032x}/decision", 1),
            r#"{"decision":"approve"}"#.to_owned(),
        ),
        (
            "/api/v1/killswitch/engage".to_owned(),
            r#"{"reason":"x"}"#.to_owned(),
        ),
        (
            "/api/v1/killswitch/release".to_owned(),
            format!(r#"{{"phrase":"{PHRASE}"}}"#),
        ),
    ]
}

async fn code(res: Response<Body>, status: u16, what: &str) -> String {
    assert_eq!(res.status(), status, "{what}");
    body_json(res).await["code"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn wrong_csrf_token_is_403_csrf_invalid_and_nothing_moves() {
    let r = rig().await;
    assert_eq!(r.csrf.len(), 64);
    for (engaged, group) in phases() {
        let before = r.phase_snapshot(engaged).await;
        for (path, body) in group {
            // Right length but wrong value; the real token truncated; empty.
            for bad in ["0".repeat(64), r.csrf[..32].to_owned(), String::new()] {
                let res = r
                    .app
                    .send(
                        r.req(&path)
                            .header(header::HOST, HOST)
                            .header(header::ORIGIN, ORIGIN)
                            .header("x-csrf-token", bad)
                            .body(Body::from(body.clone()))
                            .unwrap(),
                    )
                    .await;
                assert_eq!(code(res, 403, &path).await, "csrf_invalid", "{path}");
            }
        }
        assert_eq!(r.snapshot().await, before, "engaged: {engaged}");
    }
    assert_eq!(r.rec.placed(), Vec::<IntentId>::new());
}

#[tokio::test]
async fn foreign_origin_is_403_forbidden_origin_and_nothing_moves() {
    let r = rig().await;
    for (engaged, group) in phases() {
        let before = r.phase_snapshot(engaged).await;
        for (path, body) in group {
            for origin in [
                "http://evil.example",
                "http://127.0.0.1:9999",
                "https://127.0.0.1:8787",
                "null",
            ] {
                let res = r
                    .app
                    .send(
                        r.req(&path)
                            .header(header::HOST, HOST)
                            .header(header::ORIGIN, origin)
                            // A valid token must not rescue a foreign Origin.
                            .header("x-csrf-token", &r.csrf)
                            .body(Body::from(body.clone()))
                            .unwrap(),
                    )
                    .await;
                assert_eq!(
                    code(res, 403, &format!("{path} {origin}")).await,
                    "forbidden_origin",
                    "{path} {origin}"
                );
            }
        }
        assert_eq!(r.snapshot().await, before, "engaged: {engaged}");
    }
    assert_eq!(r.rec.placed(), Vec::<IntentId>::new());
}

#[tokio::test]
async fn foreign_host_is_403_forbidden_host_on_writes_and_reads() {
    let r = rig().await;
    for (engaged, group) in phases() {
        let before = r.phase_snapshot(engaged).await;
        for (path, body) in group {
            let res = r
                .app
                .send(
                    r.req(&path)
                        .header(header::HOST, "evil.example")
                        .header(header::ORIGIN, ORIGIN)
                        .header("x-csrf-token", &r.csrf)
                        .body(Body::from(body.clone()))
                        .unwrap(),
                )
                .await;
            assert_eq!(code(res, 403, &path).await, "forbidden_host", "{path}");
        }
        assert_eq!(r.snapshot().await, before, "engaged: {engaged}");
    }
    let before_reads = r.snapshot().await;
    for path in ["/api/v1/approvals", "/api/v1/killswitch", "/api/v1/alarms"] {
        let res = r
            .app
            .send(
                Request::get(path)
                    .header(header::HOST, "evil.example")
                    .header(header::COOKIE, &r.cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(code(res, 403, path).await, "forbidden_host", "{path}");
    }
    assert_eq!(r.snapshot().await, before_reads);
    assert_eq!(r.rec.placed(), Vec::<IntentId>::new());
}

#[tokio::test]
async fn control_the_same_request_with_right_headers_is_accepted() {
    // The refusals above came from the headers, not from a broken rig.
    let r = rig().await;
    let before = r.snapshot().await;
    let res = r
        .app
        .send(
            r.req("/api/v1/killswitch/engage")
                .header(header::HOST, HOST)
                .header(header::ORIGIN, ORIGIN)
                .header("x-csrf-token", &r.csrf)
                .body(Body::from(r#"{"reason":"control"}"#))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), 200);
    let s = r.snapshot().await;
    assert_eq!((s.0.as_str(), s.1, s.2), ("pending_approval", 0, 1));
    assert_eq!(s.3, before.3 + 1);
}

/// A mistyped decision path would still be refused by the guards above, so show that the very
/// path and body they send reaches the handler.
#[tokio::test]
async fn control_the_decision_route_reaches_its_handler() {
    let r = rig().await;
    let before = r.snapshot().await;
    let [decision, _, _] = routes();
    let res = r.send_right(decision).await;
    assert_eq!(res.status(), 200);
    let body = body_json(res).await;
    // This rig has no quote source: the approve is blocked and nothing is sent.
    assert_eq!(body["outcome"], "blocked", "{body}");
    assert_eq!(body["detail"], "no fresh quote", "{body}");
    let after = r.snapshot().await;
    assert_eq!(
        (after.0.as_str(), after.1, after.2, after.3),
        ("blocked", 0, 0, before.3 + 1)
    );
    assert_eq!(r.rec.placed(), Vec::<IntentId>::new());
}

#[tokio::test]
async fn control_the_release_route_reaches_its_handler() {
    let r = rig().await;
    let before = r.engaged_snapshot().await;
    let [_, _, release] = routes();
    let res = r.send_right(release).await;
    assert_eq!(res.status(), 200);
    // `QuietKill` answers the read, so the stored release event is the proof, not the body.
    let after = r.snapshot().await;
    assert_eq!(
        (after.0, after.1, after.2, after.3),
        (before.0, before.1, before.2 + 1, before.3 + 1)
    );
    assert_eq!(r.rec.placed(), Vec::<IntentId>::new());
}
