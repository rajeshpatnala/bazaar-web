//! Approval routes through the real router, in-process, over a temp SQLite store and a
//! `RecordingExecutor` (BZ-511 S3). No live broker; no quote source, so every approve is refused
//! by the re-gate (fail closed) and nothing is ever sent.

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
    dto::{DecisionOutcomeDto, OutcomeKind},
    runtime::{LedgerPnl, NoBaseline, ProdBase, StorePending, UnavailableQuotes},
    state::{
        IDEMPOTENCY_CAP, IdempotencyCache, KillSwitchRead, KillSwitchReading, PendingFuture,
        PendingRead, ReadFuture,
    },
};
use chrono::TimeZone;
use common::{HOST, ORIGIN, TestApp, body_json};
use rust_decimal::Decimal;
use sqlx::{ConnectOptions, sqlite::SqliteConnectOptions};

const RISK: &str = include_str!("../../../fixtures/config/risk.toml");
const KEY: &str = "key-0000001";

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
    clock: Arc<FakeClock>,
    db: PathBuf,
    _dir: tempfile::TempDir,
}

fn calendar() -> MarketCalendar {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../config");
    MarketCalendar::from_config(&load::<CalendarConfig>(std::path::Path::new(dir)).unwrap())
}

/// Pipeline over a temp store holding `seed` pending orders, reloaded then spawned.
async fn rig(seed: &[u128]) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("t.db");
    let clock = Arc::new(FakeClock::at(
        chrono_tz::Asia::Kolkata
            .with_ymd_and_hms(2026, 11, 19, 11, 0, 0)
            .unwrap(),
    ));
    let store = open_store(&db).await.unwrap();
    for &id in seed {
        let i = OrderIntent {
            id: IntentId(id),
            symbol: "ABC".into(),
            side: Side::Buy,
            qty: 1,
            kind: OrderKind::Limit(Decimal::from(50)),
            product: Product::Mis,
            reduce_only: false,
            created_at: clock.now_ist(),
        };
        let draft = AuditDraft {
            ts_ist_rfc3339: clock.now_ist().to_rfc3339(),
            actor: "seed".into(),
            kind: "proposed".into(),
            payload: format!("{{\"intent\":\"{}\"}}", IntentId(id)),
        };
        OrderRepo::insert(
            &store,
            OrderRecord {
                intent_id: id,
                version: 0,
                state: "pending_approval".into(),
                payload: i.to_payload(),
            },
            draft,
            chain_hash,
        )
        .await
        .unwrap();
    }
    let rec = RecordingExecutor::default();
    let ports = Ports {
        executor: rec.clone(),
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
    let pending = Arc::new(StorePending {
        store,
        clock: clock.clone(),
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
        clock,
        db,
        _dir: dir,
    }
}

fn hex(id: u128) -> String {
    format!("{id:032x}")
}

impl Rig {
    async fn post(&self, id: &str, key: Option<&str>, body: &str) -> Response<Body> {
        let mut b = Request::post(format!("/api/v1/approvals/{id}/decision"))
            .header(header::HOST, HOST)
            .header(header::ORIGIN, ORIGIN)
            .header(header::COOKIE, &self.cookie)
            .header("x-csrf-token", &self.csrf)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(k) = key {
            b = b.header("idempotency-key", k);
        }
        self.app
            .send(b.body(Body::from(body.to_owned())).unwrap())
            .await
    }

    async fn rows(&self, sql: &str) -> Vec<(String, String)> {
        let mut c = SqliteConnectOptions::new()
            .filename(&self.db)
            .read_only(true)
            .connect()
            .await
            .unwrap();
        // Test-only SQL built from hex ids made in this file.
        sqlx::query_as(sqlx::AssertSqlSafe(sql.to_owned()))
            .fetch_all(&mut c)
            .await
            .unwrap()
    }

    async fn order_state(&self, id: u128) -> String {
        let r = self
            .rows(&format!(
                "SELECT state, '' FROM orders WHERE order_id = '{id}'"
            ))
            .await;
        r[0].0.clone()
    }

    async fn approval_rows(&self) -> Vec<(String, String)> {
        self.rows("SELECT decision, actor FROM approvals").await
    }

    /// Actors of the audit rows that mention order `id`, excluding the seed row.
    async fn audit_actors(&self, id: u128) -> Vec<(String, String)> {
        self.rows(&format!(
            "SELECT kind, actor FROM audit_log WHERE payload LIKE '%{}%' AND actor <> 'seed' ORDER BY seq",
            hex(id)
        ))
        .await
    }
}

#[tokio::test]
async fn list_shows_pending_orders_with_expiry() {
    let r = rig(&[1, 2]).await;
    let res = r.app.get("/api/v1/approvals", Some(&r.cookie)).await;
    assert_eq!(res.status(), 200);
    let v = body_json(res).await;
    let a = v["approvals"].as_array().unwrap();
    assert_eq!(a.len(), 2);
    assert_eq!(a[0]["id"], hex(1));
    assert_eq!(a[0]["symbol"], "ABC");
    assert_eq!(a[0]["side"], "buy");
    assert_eq!(a[0]["qty"], 1);
    assert_eq!(a[0]["created_at"], "2026-11-19T11:00:00+05:30");
    assert_eq!(a[0]["expires_at"], "2026-11-19T11:05:00+05:30");
}

#[tokio::test]
async fn body_actor_is_rejected_and_nothing_changes() {
    let r = rig(&[1]).await;
    let res = r
        .post(
            &hex(1),
            Some(KEY),
            r#"{"decision":"approve","actor":"mallory"}"#,
        )
        .await;
    assert_eq!(res.status(), 422);
    assert_eq!(body_json(res).await["code"], "validation_error");
    assert_eq!(r.order_state(1).await, "pending_approval");
    assert_eq!(r.approval_rows().await, Vec::new());
    assert_eq!(r.rec.placed(), Vec::<IntentId>::new());
}

#[tokio::test]
async fn failing_regate_ends_blocked_with_session_actor_and_nothing_sent() {
    let r = rig(&[1]).await;
    r.clock.advance(chrono::Duration::seconds(2));
    let res = r
        .post(&hex(1), Some(KEY), r#"{"decision":"approve"}"#)
        .await;
    assert_eq!(res.status(), 200);
    let v = body_json(res).await;
    assert_eq!(v["outcome"], "blocked", "{v}");
    // Which rule refused: the re-gate (no fresh quote), not the TTL.
    let why = v["detail"].as_str().unwrap().to_lowercase();
    assert!(why.contains("quote"), "{why}");
    assert_eq!(r.order_state(1).await, "blocked");
    assert_eq!(r.rec.placed(), Vec::<IntentId>::new());
    // A re-gate refusal is the pipeline's own decision: one `gate_blocked` audit row written by
    // the pipeline, and no approval row exists (the user's approve never took effect). The
    // session actor on decision rows is proven by the decline test.
    assert_eq!(
        r.audit_actors(1).await,
        vec![("gate_blocked".to_owned(), "pipeline".to_owned())]
    );
    assert_eq!(r.approval_rows().await, Vec::new());
}

#[tokio::test]
async fn decline_stores_session_actor_in_approval_and_audit_rows() {
    let r = rig(&[1]).await;
    let res = r
        .post(&hex(1), Some(KEY), r#"{"decision":"decline"}"#)
        .await;
    assert_eq!(res.status(), 200);
    assert_eq!(body_json(res).await["outcome"], "declined");
    assert_eq!(r.order_state(1).await, "declined");
    assert_eq!(
        r.approval_rows().await,
        vec![("declined".to_owned(), "owner".to_owned())]
    );
    let audit = r.audit_actors(1).await;
    assert!(!audit.is_empty() && audit.iter().all(|(_, a)| a == "owner"));
    assert_eq!(r.rec.placed(), Vec::<IntentId>::new());
}

#[tokio::test]
async fn approve_past_ttl_is_expired_and_nothing_sent() {
    let r = rig(&[1]).await;
    r.clock.advance(chrono::Duration::seconds(301));
    let res = r
        .post(&hex(1), Some(KEY), r#"{"decision":"approve"}"#)
        .await;
    assert_eq!(res.status(), 200);
    let v = body_json(res).await;
    assert_eq!(v["outcome"], "expired", "{v}");
    assert_eq!(r.order_state(1).await, "expired");
    assert_eq!(r.rec.placed(), Vec::<IntentId>::new());
    assert!(
        r.approval_rows()
            .await
            .iter()
            .all(|(d, a)| d != "approved" && a == "owner")
    );
}

#[tokio::test]
async fn second_decision_is_409_already_decided_and_changes_nothing() {
    let r = rig(&[1]).await;
    let first = r
        .post(&hex(1), Some(KEY), r#"{"decision":"decline"}"#)
        .await;
    assert_eq!(first.status(), 200);
    let rows = r.approval_rows().await;
    let audit = r.audit_actors(1).await;
    let second = r
        .post(&hex(1), Some("key-0000002"), r#"{"decision":"approve"}"#)
        .await;
    assert_eq!(second.status(), 409);
    assert_eq!(body_json(second).await["code"], "already_decided");
    assert_eq!(r.order_state(1).await, "declined");
    assert_eq!(r.approval_rows().await, rows);
    assert_eq!(r.audit_actors(1).await, audit);
    assert_eq!(r.rec.placed(), Vec::<IntentId>::new());
}

#[tokio::test]
async fn same_idempotency_key_returns_the_first_reply_and_a_changed_decision_is_refused() {
    let r = rig(&[1]).await;
    let a = r
        .post(&hex(1), Some(KEY), r#"{"decision":"decline"}"#)
        .await;
    assert_eq!(a.status(), 200);
    let a = body_json(a).await;
    let rows = r.approval_rows().await;
    let b = r
        .post(&hex(1), Some(KEY), r#"{"decision":"decline"}"#)
        .await;
    assert_eq!(b.status(), 200);
    assert_eq!(body_json(b).await, a);
    assert_eq!(r.approval_rows().await, rows);
    let c = r
        .post(&hex(1), Some(KEY), r#"{"decision":"approve"}"#)
        .await;
    assert_eq!(c.status(), 422);
    assert_eq!(r.order_state(1).await, "declined");
}

#[tokio::test]
async fn bad_key_and_bad_id_are_422_and_touch_nothing() {
    let r = rig(&[1]).await;
    let body = r#"{"decision":"approve"}"#;
    for key in [
        None,
        Some("short"),
        Some("has space in key"),
        Some(&*"k".repeat(65)),
    ] {
        let res = r.post(&hex(1), key, body).await;
        assert_eq!(res.status(), 422, "{key:?}");
        assert_eq!(
            body_json(res).await["errors"][0]["field"],
            "Idempotency-Key"
        );
    }
    for id in ["zz", "123", &"g".repeat(32)] {
        let res = r.post(id, Some(KEY), body).await;
        assert_eq!(res.status(), 422, "{id}");
        assert_eq!(body_json(res).await["errors"][0]["field"], "id");
    }
    assert_eq!(r.order_state(1).await, "pending_approval");
    assert_eq!(r.approval_rows().await, Vec::new());
}

#[tokio::test]
async fn no_session_or_no_csrf_is_rejected() {
    let r = rig(&[1]).await;
    let no_session = r
        .app
        .send(
            Request::post(format!("/api/v1/approvals/{}/decision", hex(1)))
                .header(header::HOST, HOST)
                .header(header::ORIGIN, ORIGIN)
                .header(header::CONTENT_TYPE, "application/json")
                .header("idempotency-key", KEY)
                .body(Body::from(r#"{"decision":"approve"}"#))
                .unwrap(),
        )
        .await;
    assert_eq!(no_session.status(), 401);
    let no_csrf = r
        .app
        .send(
            Request::post(format!("/api/v1/approvals/{}/decision", hex(1)))
                .header(header::HOST, HOST)
                .header(header::ORIGIN, ORIGIN)
                .header(header::COOKIE, &r.cookie)
                .header(header::CONTENT_TYPE, "application/json")
                .header("idempotency-key", KEY)
                .body(Body::from(r#"{"decision":"approve"}"#))
                .unwrap(),
        )
        .await;
    assert_eq!(no_csrf.status(), 403);
    assert_eq!(body_json(no_csrf).await["code"], "csrf_invalid");
    let no_list = r.app.get("/api/v1/approvals", None).await;
    assert_eq!(no_list.status(), 401);
    assert_eq!(r.order_state(1).await, "pending_approval");
    assert_eq!(r.approval_rows().await, Vec::new());
}

#[tokio::test]
async fn unwired_pipeline_answers_503_on_both_routes() {
    let app = TestApp::new(false);
    let cookie = app.login().await;
    let list = app.get("/api/v1/approvals", Some(&cookie)).await;
    assert_eq!(list.status(), 503);
    assert_eq!(body_json(list).await["code"], "pipeline_closed");
    let me = body_json(app.get("/api/v1/auth/me", Some(&cookie)).await).await;
    let res = app
        .send(
            Request::post(format!("/api/v1/approvals/{}/decision", hex(1)))
                .header(header::HOST, HOST)
                .header(header::ORIGIN, ORIGIN)
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", me["csrf_token"].as_str().unwrap())
                .header(header::CONTENT_TYPE, "application/json")
                .header("idempotency-key", KEY)
                .body(Body::from(r#"{"decision":"approve"}"#))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), 503);
    assert_eq!(body_json(res).await["code"], "pipeline_closed");
}

struct BrokenPending;
impl PendingRead for BrokenPending {
    fn list(&self) -> PendingFuture<'_> {
        Box::pin(async { None })
    }
}

#[tokio::test]
async fn list_is_503_storage_unavailable_when_the_store_read_fails() {
    let app = TestApp::with_state(false, |s| s.with_pending(Arc::new(BrokenPending)));
    let cookie = app.login().await;
    let res = app.get("/api/v1/approvals", Some(&cookie)).await;
    assert_eq!(res.status(), 503);
    assert_eq!(body_json(res).await["code"], "storage_unavailable");
}

#[tokio::test]
async fn unknown_intent_is_404_and_is_not_remembered() {
    let r = rig(&[1]).await;
    for _ in 0..2 {
        let res = r
            .post(&hex(99), Some(KEY), r#"{"decision":"approve"}"#)
            .await;
        assert_eq!(res.status(), 404);
        assert_eq!(body_json(res).await["code"], "not_found");
    }
    assert_eq!(r.order_state(1).await, "pending_approval");
    assert_eq!(r.rec.placed(), Vec::<IntentId>::new());
}

#[tokio::test]
async fn undecodable_pending_row_is_skipped_not_listed() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(&dir.path().join("u.db")).await.unwrap();
    let clock = Arc::new(FakeClock::at(
        chrono_tz::Asia::Kolkata
            .with_ymd_and_hms(2026, 11, 19, 11, 0, 0)
            .unwrap(),
    ));
    let draft = AuditDraft {
        ts_ist_rfc3339: clock.now_ist().to_rfc3339(),
        actor: "seed".into(),
        kind: "proposed".into(),
        payload: "{}".into(),
    };
    let bad = OrderRecord {
        intent_id: 7,
        version: 0,
        state: "pending_approval".into(),
        payload: "not a payload".into(),
    };
    OrderRepo::insert(&store, bad, draft, chain_hash)
        .await
        .unwrap();
    let pending = StorePending {
        store,
        clock,
        ttl: chrono::Duration::seconds(300),
    };
    assert_eq!(pending.list().await.unwrap().len(), 0);
}

fn remembered(kind: OutcomeKind) -> DecisionOutcomeDto {
    DecisionOutcomeDto {
        outcome: kind,
        detail: None,
    }
}

#[test]
fn idempotency_cache_keeps_the_first_reply_and_evicts_the_oldest_at_the_cap() {
    let c = IdempotencyCache::default();
    c.put("k0".into(), true, remembered(OutcomeKind::Declined));
    c.put("k0".into(), false, remembered(OutcomeKind::Expired));
    assert_eq!(c.get("k0"), Some((true, remembered(OutcomeKind::Declined))));
    for i in 1..IDEMPOTENCY_CAP {
        c.put(format!("k{i}"), true, remembered(OutcomeKind::Expired));
    }
    assert!(c.get("k0").is_some(), "full but not over the cap");
    c.put("extra".into(), true, remembered(OutcomeKind::Expired));
    assert!(c.get("k0").is_none(), "oldest evicted");
    assert!(c.get("k1").is_some() && c.get("extra").is_some());
}

/// RISK-131-1 and no-place-order: nothing under `routes/` may touch the approval or order
/// stores, the atomic decide, or the executor.
const FORBIDDEN: [&str; 7] = [
    "ApprovalRepo",
    "decide_and_advance",
    "OrderRepo",
    "ApprovedOrder",
    "Executor",
    "place(",
    "bazaar_store",
];

fn forbidden_hits(text: &str) -> Vec<&'static str> {
    FORBIDDEN.into_iter().filter(|f| text.contains(f)).collect()
}

#[test]
fn routes_never_use_the_approval_repo_or_decide_and_advance() {
    assert_eq!(forbidden_hits("use x::ApprovalRepo;"), ["ApprovalRepo"]);
    assert_eq!(
        forbidden_hits("s.decide_and_advance(1)"),
        ["decide_and_advance"]
    );
    assert_eq!(
        forbidden_hits("pipeline.decide(id, d, actor)"),
        Vec::<&str>::new()
    );
    assert_eq!(forbidden_hits("x.place(o)"), ["place("]);
    let mut todo = vec![PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/routes"
    ))];
    let mut n = 0;
    while let Some(dir) = todo.pop() {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                todo.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let hits = forbidden_hits(&std::fs::read_to_string(&p).unwrap());
                assert!(hits.is_empty(), "{} uses {hits:?}", p.display());
                n += 1;
            }
        }
    }
    assert!(n >= 5, "scanned {n} route files");
}
