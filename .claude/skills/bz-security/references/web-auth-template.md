# `crates/server/src/security.rs`: code sketch

Open this when writing or reviewing the guards, the launch token, sessions, headers or the Kite nonce. Rules and reasons are in `bz-security` section 5 and `CONTRACTS.md` section 4; this file is the only code template. Versions as of 2026-11-18: axum 0.8.9, tower-http 0.7.x, axum-extra (`cookie`), subtle 2.6, getrandom 0.4.

```toml
# crates/server/Cargo.toml (pin in the lockfile; re-check docs.rs on bump)
axum = { version = "0.8", features = ["ws"] }
axum-extra = { version = "0.12", features = ["cookie"] }   # check the current minor that targets axum 0.8
tower-http = { version = "0.7", features = ["set-header", "timeout", "trace", "fs"] }   # never "cors"; "csrf" is not our check
subtle = "2"
getrandom = "0.4"
```

**Compile-check against the pinned version** before copying: this is a sketch written from docs, not from a build. Things most likely to need a touch-up: the `axum-extra` cookie builder methods, `Next`/`Request` imports, the `TraceLayer` closure type, and whether `Redirect::to` yields 303 in your axum minor (the test asserts it).

```rust
use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::{Duration, Instant},
};
use axum::{
    extract::{DefaultBodyLimit, Query, Request, State},
    http::{header, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Redirect, Response},
    routing::get,
    Extension, Json, Router,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use tower_http::{services::{ServeDir, ServeFile}, set_header::SetResponseHeaderLayer,
                 timeout::TimeoutLayer, trace::TraceLayer};

/// Copy the string from CONTRACTS.md section 4 item 5 verbatim; a test compares them.
pub const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; \
connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'; object-src 'none'";
const DEV_ORIGIN: &str = "http://localhost:5173";   // `just ui-dev`, debug builds only
const NONCE_TTL: Duration = Duration::from_secs(300);

#[derive(Debug)]
pub enum SecurityError { NonLoopbackBind(SocketAddr), Rng }

/// Startup guard. There is no bind flag or config: this is the only place an address is checked.
pub fn loopback_addr(port: u16) -> SocketAddr { SocketAddr::from((Ipv4Addr::LOCALHOST, port)) }
pub fn ensure_loopback(addr: SocketAddr) -> Result<SocketAddr, SecurityError> {
    if addr.ip() == IpAddr::V4(Ipv4Addr::LOCALHOST) { Ok(addr) } else { Err(SecurityError::NonLoopbackBind(addr)) }
}
// main: let l = TcpListener::bind(loopback_addr(port)).await?; ensure_loopback(l.local_addr()?)?;

fn random_hex<const N: usize>() -> Result<String, SecurityError> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|_| SecurityError::Rng)?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> { m.lock().unwrap_or_else(PoisonError::into_inner) }
fn ct_eq(a: &[u8], b: &[u8]) -> bool { bool::from(a.ct_eq(b)) }

#[derive(Clone)]
pub struct Session { pub csrf: String }

#[derive(Clone)]
pub struct Security(Arc<Inner>);
struct Inner {
    hosts: HashSet<String>,                     // lowercase host:port
    origins: HashSet<String>,
    launch_token: Mutex<Option<(String, Instant)>>,
    sessions: Mutex<HashMap<String, Session>>,
    nonces: Mutex<HashMap<String, Instant>>,    // Kite callback nonces
}

impl Security {
    /// `dev` is `cfg!(debug_assertions)` in main; tests pass both values.
    pub fn new(port: u16, dev: bool) -> Result<Self, SecurityError> {
        let hosts: HashSet<String> = [format!("127.0.0.1:{port}"), format!("localhost:{port}")].into();
        let mut origins: HashSet<String> = [format!("http://127.0.0.1:{port}"), format!("http://localhost:{port}")].into();
        if dev { origins.insert(DEV_ORIGIN.into()); }   // Host stays strict: the Vite proxy uses changeOrigin: true
        Ok(Self(Arc::new(Inner {
            hosts, origins,
            launch_token: Mutex::new(Some((random_hex::<32>()?, Instant::now()))),
            sessions: Mutex::default(), nonces: Mutex::default(),
        })))
    }
    /// Printed once to stdout by main (writeln!, never tracing), then opened in the browser.
    /// Debug builds point at the Vite dev server (`just ui-dev`) so the cookie belongs to that host.
    pub fn launch_url(&self, port: u16, dev: bool) -> Option<String> {
        let base = if dev { "http://localhost:5173".to_string() } else { format!("http://127.0.0.1:{port}") };
        lock(&self.0.launch_token).as_ref().map(|(t, _)| format!("{base}/auth?token={t}"))
    }
    fn take_launch_token(&self, given: &str) -> bool {
        let mut slot = lock(&self.0.launch_token);
        let ok = slot.as_ref().is_some_and(|(t, born)| {
            born.elapsed() < Duration::from_secs(600) && ct_eq(t.as_bytes(), given.as_bytes())
        });
        if ok { *slot = None; }                 // single use; a wrong guess does not consume it
        ok
    }
    fn new_session(&self) -> Result<String, SecurityError> {
        let (id, csrf) = (random_hex::<32>()?, random_hex::<32>()?);
        lock(&self.0.sessions).insert(id.clone(), Session { csrf });
        Ok(id)
    }
    fn session(&self, id: &str) -> Option<Session> { lock(&self.0.sessions).get(id).cloned() }

    pub fn mint_kite_nonce(&self) -> Result<String, SecurityError> {
        let n = random_hex::<16>()?;
        let mut m = lock(&self.0.nonces);
        m.retain(|_, born| born.elapsed() < NONCE_TTL);
        m.insert(n.clone(), Instant::now());
        Ok(n)
    }
    pub fn take_kite_nonce(&self, n: &str) -> bool {
        lock(&self.0.nonces).remove(n).is_some_and(|born| born.elapsed() < NONCE_TTL)
    }
}

fn problem(status: StatusCode) -> Response {
    (status, [(header::CONTENT_TYPE, "application/problem+json")],
     format!(r#"{{"type":"about:blank","title":"{}","status":{}}}"#,
             status.canonical_reason().unwrap_or("error"), status.as_u16())).into_response()
}

/// 1. Outermost. DNS rebinding: the Host of a rebound page is the attacker's name.
pub async fn host_guard(State(s): State<Security>, req: Request, next: Next) -> Response {
    let ok = req.headers().get(header::HOST).and_then(|v| v.to_str().ok())
        .is_some_and(|h| s.0.hosts.contains(&h.to_ascii_lowercase()));
    if ok { next.run(req).await } else { problem(StatusCode::FORBIDDEN) }
}

/// 2. Non-GET and WebSocket upgrades need an exactly matching Origin. Missing is rejected.
pub async fn origin_guard(State(s): State<Security>, req: Request, next: Next) -> Response {
    let is_ws = req.headers().contains_key(header::UPGRADE);
    let safe = matches!(*req.method(), Method::GET | Method::HEAD);
    if safe && !is_ws { return next.run(req).await; }
    let ok = req.headers().get(header::ORIGIN).and_then(|v| v.to_str().ok())
        .is_some_and(|o| s.0.origins.contains(o));            // exact match, no prefix logic
    if ok { next.run(req).await } else { problem(StatusCode::FORBIDDEN) }
}

/// 3. Session cookie. Applied with `route_layer` to protected routes only (outer of the two).
pub async fn session_guard(State(s): State<Security>, jar: CookieJar, mut req: Request, next: Next) -> Response {
    let Some(sess) = jar.get("bz_session").and_then(|c| s.session(c.value())) else {
        return problem(StatusCode::UNAUTHORIZED);
    };
    req.extensions_mut().insert(sess);
    next.run(req).await
}

/// 4. CSRF header on non-GET. `route_layer` inside `session_guard`, so the `Session` extension exists.
pub async fn csrf_guard(req: Request, next: Next) -> Response {
    if !matches!(*req.method(), Method::GET | Method::HEAD) {
        let Some(sess) = req.extensions().get::<Session>() else { return problem(StatusCode::UNAUTHORIZED); };
        let sent = req.headers().get("x-csrf-token").map(|v| v.as_bytes()).unwrap_or_default();
        if !ct_eq(sent, sess.csrf.as_bytes()) { return problem(StatusCode::FORBIDDEN); }
    }
    next.run(req).await
}

#[derive(Deserialize)] struct AuthQuery { token: String }

/// GET /auth?token=...: exchange the launch token for the session cookie.
pub async fn auth(State(s): State<Security>, Query(q): Query<AuthQuery>, jar: CookieJar) -> Response {
    if !s.take_launch_token(&q.token) { return problem(StatusCode::FORBIDDEN); }
    let Ok(id) = s.new_session() else { return problem(StatusCode::INTERNAL_SERVER_ERROR); };
    let cookie = Cookie::build(("bz_session", id)).http_only(true).same_site(SameSite::Strict).path("/").build();
    (jar.add(cookie), Redirect::to("/")).into_response()         // 303; token leaves the URL
}

// `GET /api/v1/auth/me` is a normal OpenAPI route in `routes/auth.rs` (`bz-web-api`): it reads
// `Extension<Session>` for `csrf_token` and app state for `mode` and `live_enabled` (`AuthMeDto`).

/// GET /api/v1/kite/login (session required): mint the nonce and send the browser to Kite.
/// `redirect_params` is a URL-encoded query string that Kite appends to the redirect URL
/// (confirm against https://kite.trade/docs/connect/v3/user/ before relying on it).
async fn kite_login(State(s): State<Security> /*, State(app) for the api key */) -> Response {
    let Ok(n) = s.mint_kite_nonce() else { return problem(StatusCode::INTERNAL_SERVER_ERROR); };
    let api_key = "<from SecretStore>";
    Redirect::to(&format!("https://kite.zerodha.com/connect/login?v=3&api_key={api_key}&redirect_params=nonce%3D{n}")).into_response()
}

#[derive(Deserialize)] struct Callback { request_token: String, nonce: String /* status, action, type: ignored */ }
async fn kite_callback(State(s): State<Security>, Query(q): Query<Callback>) -> Response {
    if !s.take_kite_nonce(&q.nonce) { return problem(StatusCode::FORBIDDEN); }
    // exchange q.request_token now (SecretString), SecretStore::set(KiteAccessToken, ..); never log q
    // then back to the UI origin: `/settings?kite=connected` (debug builds: http://localhost:5173/settings?kite=connected)
    Redirect::to("/settings?kite=connected").into_response()
}

// The router is built only in `app.rs` (`bz-web-api` section 4); that layer order is authoritative:
// outermost host_guard, origin_guard, headers/timeout/trace, body limit; on protected /api/v1 routes
// route_layer(session_guard) outside route_layer(csrf_guard); /healthz, /auth and the Kite callback exempt.
// Guards take State<Security>: pass `state.security.clone()` to `from_fn_with_state`.
```

Notes for the implementer:

- The WebSocket route lives in the protected block, so the handshake passes Host, Origin and session guards. Sessions only end when the server restarts, which closes every socket, so the socket task needs no session polling. `TimeoutLayer` covers producing the 101 response only; confirm in a test that an open socket outlives 30 s.
- Handlers that read `Session` use `Extension<Session>`; `AuthMeDto` returns only `csrf_token`, `mode`, `live_enabled`.
- `main` prints `launch_url(port, cfg!(debug_assertions))` with `writeln!(std::io::stdout(), ...)` and opens it with the system `open` command; both ignore failure. The URL appears nowhere else.
- Because `Router::layer` also wraps `fallback_service`, static files get the Host/Origin checks and headers too; they need no session.
- Add `X-CSRF-Token` handling in the UI client only through `bz-web-ui` (`api/client.ts` middleware); the header name is fixed by CONTRACTS.
- tower-http 0.7 has `csrf::CsrfLayer`; it allows requests that carry neither `Origin` nor `Sec-Fetch-Site`, so it does not replace `origin_guard` (our rule: missing Origin on non-GET is a 403). Using it as an extra layer is optional.
- `just ui-dev`: the Vite proxy uses `changeOrigin: true` (`bz-web-ui`), so the server sees `Host: 127.0.0.1:8787` while `Origin` stays `http://localhost:5173`; only the Origin set gains the dev entry, in debug builds only.
