//! `TestApp`: the real router (`app::build`) run in-process with a fake clock and a temp web dir.

#![allow(dead_code, unreachable_pub, clippy::unwrap_used, clippy::expect_used)]

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

use axum::{
    Router,
    body::Body,
    http::{Request, Response, header},
};
use bazaar_core::{mode::ExecutionMode, testutil::FakeClock};
use bazaar_server::{app, security::Security, state::AppState};
use chrono::TimeZone;
use http_body_util::BodyExt;
use tower::ServiceExt;

pub const PORT: u16 = 8787;
pub const HOST: &str = "127.0.0.1:8787";
pub const ORIGIN: &str = "http://127.0.0.1:8787";

static N: AtomicU32 = AtomicU32::new(0);

pub struct TestApp {
    pub router: Router,
    pub security: Security,
    pub dist: PathBuf,
}

impl Drop for TestApp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dist);
    }
}

impl TestApp {
    /// `with_ui`: write a tiny `index.html` and `assets/app.js` into the temp dist.
    pub fn new(with_ui: bool) -> Self {
        let dist = std::env::temp_dir().join(format!(
            "bz-server-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(dist.join("assets")).unwrap();
        if with_ui {
            std::fs::write(dist.join("index.html"), "<html>bazaar-ui</html>").unwrap();
            std::fs::write(dist.join("assets/app.js"), "console.log(1)").unwrap();
        }
        let security = Security::new(PORT, false).unwrap();
        let t = chrono_tz::Asia::Kolkata
            .with_ymd_and_hms(2026, 1, 5, 10, 0, 0)
            .unwrap();
        let state = AppState::new(
            security.clone(),
            Arc::new(FakeClock::at(t)),
            ExecutionMode::Paper,
            dist.clone(),
        );
        Self {
            router: app::build(state),
            security,
            dist,
        }
    }

    pub async fn send(&self, req: Request<Body>) -> Response<Body> {
        self.router.clone().oneshot(req).await.unwrap()
    }

    /// GET with the correct Host and optional cookie.
    pub async fn get(&self, path: &str, cookie: Option<&str>) -> Response<Body> {
        let mut b = Request::get(path).header(header::HOST, HOST);
        if let Some(c) = cookie {
            b = b.header(header::COOKIE, c);
        }
        self.send(b.body(Body::empty()).unwrap()).await
    }

    /// Exchange the launch token; returns the `bz_session=...` cookie pair.
    pub async fn login(&self) -> String {
        let url = self.security.launch_url(PORT, false).unwrap();
        let path = url.strip_prefix(ORIGIN).unwrap().to_owned();
        let res = self.get(&path, None).await;
        assert_eq!(res.status(), 303);
        let sc = res.headers()[header::SET_COOKIE].to_str().unwrap();
        sc.split(';').next().unwrap().to_owned()
    }
}

pub async fn body_bytes(res: Response<Body>) -> Vec<u8> {
    res.into_body().collect().await.unwrap().to_bytes().to_vec()
}

pub async fn body_json(res: Response<Body>) -> serde_json::Value {
    serde_json::from_slice(&body_bytes(res).await).unwrap()
}
