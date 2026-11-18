//! Security layer: launch token, sessions, Host/Origin/CSRF guards, CSP (slice F1/server-security).
//!
//! Guard order (wired once in `app::build`, outermost first): `security_headers` (so guard
//! rejections also carry CSP and nosniff), `host_guard`, `origin_guard`, `problem_backstop`; then on
//! protected routes `session_guard` (outer) and `csrf_guard` (inner). With tower `.layer()` calls the
//! LAST call is outermost, so call `origin_guard` first and `security_headers` last. Exempt: `/healthz`, `/auth` and the
//! Kite callback. There is no CORS, no bind setting and no secret in any log line.

use std::{
    collections::{HashMap, HashSet},
    fmt,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::{Duration, Instant},
};

use axum::{
    extract::{Request, State},
    http::{HeaderName, HeaderValue, Method, header},
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use serde::Deserialize;
use subtle::ConstantTimeEq;

use crate::error::{ApiError, ApiQuery};

/// Content-Security-Policy sent on every response (frozen contract string).
pub const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; \
connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'; object-src 'none'";
/// Session cookie name.
pub const SESSION_COOKIE: &str = "bz_session";
/// CSRF request header.
pub const CSRF_HEADER: &str = "x-csrf-token";
/// Paths that need no session (the exempt list; a test pins it).
pub const EXEMPT_PATHS: [&str; 3] = ["/healthz", "/auth", "/api/v1/kite/callback"];

const DEV_ORIGIN: &str = "http://localhost:5173";
const NONCE_TTL: Duration = Duration::from_secs(300);
const LAUNCH_TTL: Duration = Duration::from_secs(600);
/// A session ends after this long without a request.
const SESSION_IDLE_TTL: Duration = Duration::from_hours(2);
/// A session ends this long after login regardless of activity.
const SESSION_ABS_TTL: Duration = Duration::from_hours(12);
/// Live sessions kept at once; the oldest is evicted beyond this.
const MAX_SESSIONS: usize = 8;

/// Monotonic time source, injected so tests can move time.
type NowFn = Arc<dyn Fn() -> Instant + Send + Sync>;

#[derive(Clone, Copy)]
struct Ttls {
    launch: Duration,
    session_idle: Duration,
    session_abs: Duration,
    max_sessions: usize,
}

impl Ttls {
    const DEFAULT: Self = Self {
        launch: LAUNCH_TTL,
        session_idle: SESSION_IDLE_TTL,
        session_abs: SESSION_ABS_TTL,
        max_sessions: MAX_SESSIONS,
    };
}

struct SessionRec {
    session: Session,
    created: Instant,
    last_seen: Instant,
}

/// Security setup failures.
#[derive(Debug, thiserror::Error)]
pub enum SecurityError {
    /// Listener address is not `127.0.0.1`.
    #[error("refusing to bind non-loopback address {0}")]
    NonLoopbackBind(SocketAddr),
    /// The OS random source failed.
    #[error("operating system random source failed")]
    Rng,
}

/// The only bind address: `127.0.0.1:<port>`. There is no flag or config for the host.
#[must_use]
pub fn loopback_addr(port: u16) -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, port))
}

/// Startup guard: abort on any address other than `127.0.0.1`.
///
/// # Errors
/// `NonLoopbackBind` for `0.0.0.0`, `::`, `::1` or LAN addresses.
pub fn ensure_loopback(addr: SocketAddr) -> Result<SocketAddr, SecurityError> {
    if addr.ip() == IpAddr::V4(Ipv4Addr::LOCALHOST) {
        Ok(addr)
    } else {
        Err(SecurityError::NonLoopbackBind(addr))
    }
}

fn random_hex<const N: usize>() -> Result<String, SecurityError> {
    use std::fmt::Write as _;
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|_| SecurityError::Rng)?;
    let mut s = String::with_capacity(N * 2);
    for x in b {
        // Writing to a String cannot fail.
        let _ = write!(s, "{x:02x}");
    }
    Ok(s)
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    bool::from(a.ct_eq(b))
}

/// A live browser session; inserted as a request extension by `session_guard`.
#[derive(Clone)]
pub struct Session {
    /// CSRF token the client must echo in `X-CSRF-Token` on non-GET requests.
    pub csrf: String,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("csrf", &"<redacted>")
            .finish()
    }
}

/// Shared security state (cheap to clone).
#[derive(Clone)]
pub struct Security(Arc<Inner>);

struct Inner {
    hosts: HashSet<String>,
    origins: HashSet<String>,
    ttls: Ttls,
    now: NowFn,
    launch_token: Mutex<Option<(String, Instant)>>,
    sessions: Mutex<HashMap<String, SessionRec>>,
    nonces: Mutex<HashMap<String, Instant>>,
}

impl fmt::Debug for Security {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Security")
            .field("hosts", &self.0.hosts)
            .finish_non_exhaustive()
    }
}

impl Security {
    /// `dev` is `cfg!(debug_assertions)` in `main`; it adds the Vite origin (Origin only, never Host).
    ///
    /// # Errors
    /// `Rng` if the OS random source fails.
    pub fn new(port: u16, dev: bool) -> Result<Self, SecurityError> {
        Self::with_parts(port, dev, Ttls::DEFAULT, Arc::new(Instant::now))
    }

    fn with_parts(port: u16, dev: bool, ttls: Ttls, now: NowFn) -> Result<Self, SecurityError> {
        let hosts: HashSet<String> =
            [format!("127.0.0.1:{port}"), format!("localhost:{port}")].into();
        let mut origins: HashSet<String> = [
            format!("http://127.0.0.1:{port}"),
            format!("http://localhost:{port}"),
        ]
        .into();
        if dev {
            origins.insert(DEV_ORIGIN.to_owned());
        }
        Ok(Self(Arc::new(Inner {
            hosts,
            origins,
            ttls,
            launch_token: Mutex::new(Some((random_hex::<32>()?, now()))),
            now,
            sessions: Mutex::default(),
            nonces: Mutex::default(),
        })))
    }

    /// Launch URL, printed once to stdout by `main` (never via `tracing`). `None` once consumed or expired.
    #[must_use]
    pub fn launch_url(&self, port: u16, dev: bool) -> Option<String> {
        let base = if dev {
            DEV_ORIGIN.to_owned()
        } else {
            format!("http://127.0.0.1:{port}")
        };
        let mut slot = lock(&self.0.launch_token);
        let now = (self.0.now)();
        if slot
            .as_ref()
            .is_some_and(|(_, born)| now.saturating_duration_since(*born) >= self.0.ttls.launch)
        {
            *slot = None; // expired: drop it so a dead link is never printed
        }
        slot.as_ref().map(|(t, _)| format!("{base}/auth?token={t}"))
    }

    fn take_launch_token(&self, given: &str) -> bool {
        let mut slot = lock(&self.0.launch_token);
        let now = (self.0.now)();
        let ok = slot.as_ref().is_some_and(|(t, born)| {
            now.saturating_duration_since(*born) < self.0.ttls.launch
                && ct_eq(t.as_bytes(), given.as_bytes())
        });
        if ok {
            *slot = None; // single use; a wrong guess does not consume it
        }
        ok
    }

    fn new_session(&self) -> Result<String, SecurityError> {
        let (id, csrf) = (random_hex::<32>()?, random_hex::<32>()?);
        let now = (self.0.now)();
        let mut m = lock(&self.0.sessions);
        let t = self.0.ttls;
        m.retain(|_, r| Self::live(r, now, t));
        while m.len() >= t.max_sessions {
            let Some(oldest) = m
                .iter()
                .min_by_key(|(_, r)| r.created)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            m.remove(&oldest);
        }
        m.insert(
            id.clone(),
            SessionRec {
                session: Session { csrf },
                created: now,
                last_seen: now,
            },
        );
        Ok(id)
    }

    fn live(r: &SessionRec, now: Instant, t: Ttls) -> bool {
        now.saturating_duration_since(r.created) < t.session_abs
            && now.saturating_duration_since(r.last_seen) < t.session_idle
    }

    /// Look up a live session, refreshing its idle clock; expired ones are removed.
    fn session(&self, id: &str) -> Option<Session> {
        let now = (self.0.now)();
        let mut m = lock(&self.0.sessions);
        let alive = m.get(id).is_some_and(|r| Self::live(r, now, self.0.ttls));
        if !alive {
            m.remove(id);
            return None;
        }
        let r = m.get_mut(id)?;
        r.last_seen = now;
        Some(r.session.clone())
    }

    /// Mint a one-time Kite login nonce (5 minute TTL).
    ///
    /// # Errors
    /// `Rng` if the OS random source fails.
    pub fn mint_kite_nonce(&self) -> Result<String, SecurityError> {
        let n = random_hex::<16>()?;
        let mut m = lock(&self.0.nonces);
        let now = (self.0.now)();
        m.retain(|_, born| now.saturating_duration_since(*born) < NONCE_TTL);
        m.insert(n.clone(), now);
        Ok(n)
    }

    /// Consume a nonce: true only for a known, unexpired, unused one.
    #[must_use]
    pub fn take_kite_nonce(&self, n: &str) -> bool {
        let now = (self.0.now)();
        lock(&self.0.nonces)
            .remove(n)
            .is_some_and(|born| now.saturating_duration_since(born) < NONCE_TTL)
    }
}

/// 1. Outermost. DNS rebinding: a rebound page carries the attacker's Host. Missing Host is refused.
pub async fn host_guard(State(s): State<Security>, req: Request, next: Next) -> Response {
    let ok = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|h| s.0.hosts.contains(&h.to_ascii_lowercase()));
    if ok {
        next.run(req).await
    } else {
        ApiError::ForbiddenHost.into_response()
    }
}

/// 2. Non-GET and WebSocket upgrades need an exactly matching Origin; missing or `null` is refused.
pub async fn origin_guard(State(s): State<Security>, req: Request, next: Next) -> Response {
    let is_ws = req.headers().contains_key(header::UPGRADE);
    let safe = matches!(*req.method(), Method::GET | Method::HEAD);
    if safe && !is_ws {
        return next.run(req).await;
    }
    let ok = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|o| s.0.origins.contains(o));
    if ok {
        next.run(req).await
    } else {
        ApiError::ForbiddenOrigin.into_response()
    }
}

/// 3. Session cookie, for protected routes (`route_layer`, outer of the two).
pub async fn session_guard(
    State(s): State<Security>,
    jar: CookieJar,
    mut req: Request,
    next: Next,
) -> Response {
    let Some(sess) = jar.get(SESSION_COOKIE).and_then(|c| s.session(c.value())) else {
        return ApiError::Unauthenticated.into_response();
    };
    req.extensions_mut().insert(sess);
    next.run(req).await
}

/// 4. CSRF header on non-GET (`route_layer` inside `session_guard`, so `Session` exists).
pub async fn csrf_guard(req: Request, next: Next) -> Response {
    if !matches!(*req.method(), Method::GET | Method::HEAD) {
        let Some(sess) = req.extensions().get::<Session>() else {
            return ApiError::Unauthenticated.into_response();
        };
        let sent = req
            .headers()
            .get(CSRF_HEADER)
            .map_or(&[][..], HeaderValue::as_bytes);
        if !ct_eq(sent, sess.csrf.as_bytes()) {
            return ApiError::CsrfInvalid.into_response();
        }
    }
    next.run(req).await
}

/// Response headers on every response: CSP, nosniff, no-referrer; `no-store` on `/api/*`.
pub async fn security_headers(req: Request, next: Next) -> Response {
    let is_api = req.uri().path().starts_with("/api/");
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    if is_api {
        h.insert(
            HeaderName::from_static("cache-control"),
            HeaderValue::from_static("no-store"),
        );
    }
    res
}

/// `TraceLayer` span: method and path only, never the query (`/auth?token=`, Kite callback).
pub fn path_only_span<B>(req: &axum::http::Request<B>) -> tracing::Span {
    tracing::info_span!("http", method = %req.method(), path = %req.uri().path())
}

#[derive(Deserialize)]
pub struct AuthQuery {
    token: String,
}

impl fmt::Debug for AuthQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthQuery")
            .field("token", &"<redacted>")
            .finish()
    }
}

/// `GET /auth?token=...`: exchange the launch token for the session cookie, then 303 to `/`.
#[allow(clippy::unused_async)] // axum handlers must be async fns
pub async fn auth(
    State(s): State<Security>,
    ApiQuery(q): ApiQuery<AuthQuery>,
    jar: CookieJar,
) -> Response {
    if !s.take_launch_token(&q.token) {
        return ApiError::LaunchTokenInvalid.into_response();
    }
    let Ok(id) = s.new_session() else {
        tracing::error!("session id generation failed");
        return ApiError::Internal.into_response();
    };
    let cookie = Cookie::build((SESSION_COOKIE, id))
        .http_only(true)
        .same_site(SameSite::Strict)
        .path("/")
        .build();
    (jar.add(cookie), Redirect::to("/")).into_response()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use axum::{
        Router, body::Body, http::StatusCode, middleware::from_fn_with_state, routing::get,
        routing::post,
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    const PORT: u16 = 8787;

    fn app(sec: &Security) -> Router {
        let protected = Router::new()
            .route(
                "/api/v1/thing",
                get(|| async { "ok" }).post(|| async { "done" }),
            )
            .route_layer(from_fn_with_state(sec.clone(), csrf_guard))
            .route_layer(from_fn_with_state(sec.clone(), session_guard));
        Router::new()
            .route("/healthz", get(|| async { "ok" }))
            .route("/auth", get(auth))
            .route("/open", post(|| async { "open" }))
            .merge(protected)
            // last `.layer` is outermost: security_headers, host_guard, origin_guard
            .layer(from_fn_with_state(sec.clone(), origin_guard))
            .layer(from_fn_with_state(sec.clone(), host_guard))
            .layer(axum::middleware::from_fn(security_headers))
            .with_state(sec.clone())
    }

    /// One second short of `d`.
    fn before(d: Duration) -> Duration {
        d.checked_sub(Duration::from_secs(1)).unwrap()
    }

    /// A controllable monotonic clock for TTL tests.
    #[derive(Clone)]
    struct Tick {
        base: Instant,
        off: Arc<Mutex<Duration>>,
    }
    impl Tick {
        fn new() -> Self {
            Self {
                base: Instant::now(),
                off: Arc::default(),
            }
        }
        fn advance(&self, d: Duration) {
            *lock(&self.off) += d;
        }
        fn now_fn(&self) -> NowFn {
            let t = self.clone();
            Arc::new(move || t.base + *lock(&t.off))
        }
    }

    fn sec_with(ttls: Ttls) -> (Security, Tick) {
        let t = Tick::new();
        (
            Security::with_parts(PORT, false, ttls, t.now_fn()).unwrap(),
            t,
        )
    }

    fn req(method: &str, uri: &str, host: Option<&str>, extra: &[(&str, &str)]) -> Request<Body> {
        let mut b = Request::builder().method(method).uri(uri);
        if let Some(h) = host {
            b = b.header(header::HOST, h);
        }
        for (k, v) in extra {
            b = b.header(*k, *v);
        }
        b.body(Body::empty()).unwrap()
    }

    fn token(sec: &Security) -> String {
        sec.launch_url(PORT, false)
            .unwrap()
            .split("token=")
            .nth(1)
            .unwrap()
            .to_owned()
    }

    async fn code(res: Response) -> String {
        let b = res.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice::<serde_json::Value>(&b).unwrap()["code"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }

    /// Log in and return `(cookie, csrf)`.
    async fn login(sec: &Security) -> (String, String) {
        let uri = format!("/auth?token={}", token(sec));
        let res = app(sec)
            .oneshot(req("GET", &uri, Some("127.0.0.1:8787"), &[]))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(res.headers()[header::LOCATION], "/");
        let set = res.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .to_owned();
        let cookie = set.split(';').next().unwrap().to_owned();
        let id = cookie.split('=').nth(1).unwrap();
        let csrf = sec.session(id).unwrap().csrf;
        (cookie, csrf)
    }

    fn sec() -> Security {
        Security::new(PORT, false).unwrap()
    }

    #[tokio::test]
    async fn no_session_is_401_problem() {
        let s = sec();
        let res = app(&s)
            .oneshot(req("GET", "/api/v1/thing", Some("127.0.0.1:8787"), &[]))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            res.headers()[header::CONTENT_TYPE],
            "application/problem+json"
        );
        assert_eq!(code(res).await, "unauthenticated");
    }

    #[tokio::test]
    async fn bad_or_missing_host_is_403() {
        let s = sec();
        for host in [
            Some("evil.example:8787"),
            Some("127.0.0.1:9999"),
            Some("localhost"),
            None,
        ] {
            let res = app(&s)
                .oneshot(req("GET", "/healthz", host, &[]))
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::FORBIDDEN, "{host:?}");
            assert_eq!(code(res).await, "forbidden_host");
        }
        let ok = app(&s)
            .oneshot(req("GET", "/healthz", Some("LocalHost:8787"), &[]))
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn origin_required_and_exact_on_non_get() {
        let s = sec();
        let h = Some("127.0.0.1:8787");
        for origin in [
            None,
            Some("null"),
            Some("http://127.0.0.1:8787.evil.com"),
            Some("http://localhost:5173"),
        ] {
            let extra: Vec<(&str, &str)> = origin.map(|o| ("origin", o)).into_iter().collect();
            let res = app(&s)
                .oneshot(req("POST", "/open", h, &extra))
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::FORBIDDEN, "{origin:?}");
            assert_eq!(code(res).await, "forbidden_origin");
        }
        let ok = app(&s)
            .oneshot(req(
                "POST",
                "/open",
                h,
                &[("origin", "http://localhost:8787")],
            ))
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
        // websocket-style upgrade GET also needs Origin
        let ws = app(&s)
            .oneshot(req("GET", "/healthz", h, &[("upgrade", "websocket")]))
            .await
            .unwrap();
        assert_eq!(ws.status(), StatusCode::FORBIDDEN);
        // OPTIONS preflight is refused
        let pre = app(&s)
            .oneshot(req("OPTIONS", "/open", h, &[]))
            .await
            .unwrap();
        assert_eq!(pre.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn dev_origin_only_in_dev_builds() {
        let dev = Security::new(PORT, true).unwrap();
        let h = Some("127.0.0.1:8787");
        let res = app(&dev)
            .oneshot(req("POST", "/open", h, &[("origin", DEV_ORIGIN)]))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        // Host stays strict in dev
        let res = app(&dev)
            .oneshot(req("GET", "/healthz", Some("localhost:5173"), &[]))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn auth_sets_strict_httponly_cookie_and_token_is_single_use() {
        let s = sec();
        let tok = token(&s);
        let uri = format!("/auth?token={tok}");
        let res = app(&s)
            .oneshot(req("GET", &uri, Some("127.0.0.1:8787"), &[]))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        let set = res.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .to_owned();
        assert!(set.starts_with("bz_session="));
        assert!(
            set.contains("HttpOnly") && set.contains("SameSite=Strict") && set.contains("Path=/")
        );
        assert!(!set.contains("Domain") && !set.contains("Secure"));
        assert!(s.launch_url(PORT, false).is_none());
        let again = app(&s)
            .oneshot(req("GET", &uri, Some("127.0.0.1:8787"), &[]))
            .await
            .unwrap();
        assert_eq!(again.status(), StatusCode::FORBIDDEN);
        assert_eq!(code(again).await, "launch_token_invalid");
    }

    #[tokio::test]
    async fn wrong_token_does_not_burn_the_real_one() {
        let s = sec();
        let h = Some("127.0.0.1:8787");
        let bad = app(&s)
            .oneshot(req("GET", "/auth?token=nope", h, &[]))
            .await
            .unwrap();
        assert_eq!(bad.status(), StatusCode::FORBIDDEN);
        let missing = app(&s).oneshot(req("GET", "/auth", h, &[])).await.unwrap();
        assert_eq!(missing.status(), StatusCode::BAD_REQUEST);
        assert_eq!(code(missing).await, "bad_request");
        assert!(s.launch_url(PORT, false).is_some());
    }

    #[tokio::test]
    async fn expired_launch_token_is_refused() {
        let (s, clock) = sec_with(Ttls::DEFAULT);
        let uri = format!("/auth?token={}", token(&s));
        clock.advance(LAUNCH_TTL);
        let res = app(&s)
            .oneshot(req("GET", &uri, Some("127.0.0.1:8787"), &[]))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        assert_eq!(code(res).await, "launch_token_invalid");
    }

    #[tokio::test]
    async fn launch_token_just_inside_ttl_still_works() {
        let (s, clock) = sec_with(Ttls::DEFAULT);
        let uri = format!("/auth?token={}", token(&s));
        clock.advance(before(LAUNCH_TTL));
        let res = app(&s)
            .oneshot(req("GET", &uri, Some("127.0.0.1:8787"), &[]))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
    }

    #[test]
    fn launch_url_is_none_after_expiry() {
        let (s, clock) = sec_with(Ttls::DEFAULT);
        assert!(s.launch_url(PORT, false).is_some());
        clock.advance(LAUNCH_TTL);
        assert!(s.launch_url(PORT, false).is_none());
    }

    #[tokio::test]
    async fn session_expires_when_idle_but_activity_keeps_it() {
        let (s, clock) = sec_with(Ttls::DEFAULT);
        let (cookie, _) = login(&s).await;
        let h = Some("127.0.0.1:8787");
        let get = |s: Security, cookie: String| async move {
            app(&s)
                .oneshot(req("GET", "/api/v1/thing", h, &[("cookie", &cookie)]))
                .await
                .unwrap()
                .status()
        };
        let almost = before(SESSION_IDLE_TTL);
        clock.advance(almost);
        assert_eq!(get(s.clone(), cookie.clone()).await, StatusCode::OK);
        clock.advance(almost); // idle clock was refreshed
        assert_eq!(get(s.clone(), cookie.clone()).await, StatusCode::OK);
        clock.advance(SESSION_IDLE_TTL);
        assert_eq!(
            get(s.clone(), cookie.clone()).await,
            StatusCode::UNAUTHORIZED
        );
        assert!(lock(&s.0.sessions).is_empty());
    }

    #[tokio::test]
    async fn session_has_absolute_lifetime_despite_activity() {
        let (s, clock) = sec_with(Ttls::DEFAULT);
        let (cookie, _) = login(&s).await;
        let h = Some("127.0.0.1:8787");
        let step = before(SESSION_IDLE_TTL);
        let mut elapsed = Duration::ZERO;
        while elapsed + step < SESSION_ABS_TTL {
            clock.advance(step);
            elapsed += step;
            let r = app(&s)
                .oneshot(req("GET", "/api/v1/thing", h, &[("cookie", &cookie)]))
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::OK);
        }
        clock.advance(SESSION_ABS_TTL.checked_sub(elapsed).unwrap());
        let r = app(&s)
            .oneshot(req("GET", "/api/v1/thing", h, &[("cookie", &cookie)]))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn live_sessions_are_capped_and_oldest_evicted() {
        let ttls = Ttls {
            max_sessions: 2,
            ..Ttls::DEFAULT
        };
        let (s, clock) = sec_with(ttls);
        let a = s.new_session().unwrap();
        clock.advance(Duration::from_secs(1));
        let b = s.new_session().unwrap();
        clock.advance(Duration::from_secs(1));
        let c = s.new_session().unwrap();
        assert_eq!(lock(&s.0.sessions).len(), 2);
        assert!(s.session(&a).is_none());
        assert!(s.session(&b).is_some() && s.session(&c).is_some());
    }

    #[tokio::test]
    async fn session_get_ok_and_csrf_enforced_on_post() {
        let s = sec();
        let (cookie, csrf) = login(&s).await;
        let h = Some("127.0.0.1:8787");
        let origin = ("origin", "http://127.0.0.1:8787");
        let get = app(&s)
            .oneshot(req("GET", "/api/v1/thing", h, &[("cookie", &cookie)]))
            .await
            .unwrap();
        assert_eq!(get.status(), StatusCode::OK);
        assert_eq!(get.headers()[header::CACHE_CONTROL], "no-store");

        let none = app(&s)
            .oneshot(req(
                "POST",
                "/api/v1/thing",
                h,
                &[("cookie", &cookie), origin],
            ))
            .await
            .unwrap();
        assert_eq!(none.status(), StatusCode::FORBIDDEN);
        assert_eq!(code(none).await, "csrf_invalid");
        let wrong = app(&s)
            .oneshot(req(
                "POST",
                "/api/v1/thing",
                h,
                &[("cookie", &cookie), origin, ("x-csrf-token", "bad")],
            ))
            .await
            .unwrap();
        assert_eq!(wrong.status(), StatusCode::FORBIDDEN);
        let good = app(&s)
            .oneshot(req(
                "POST",
                "/api/v1/thing",
                h,
                &[("cookie", &cookie), origin, ("x-csrf-token", &csrf)],
            ))
            .await
            .unwrap();
        assert_eq!(good.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn forged_session_cookie_is_401() {
        let s = sec();
        let res = app(&s)
            .oneshot(req(
                "GET",
                "/api/v1/thing",
                Some("127.0.0.1:8787"),
                &[("cookie", "bz_session=deadbeef")],
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn headers_on_every_response_including_rejections() {
        let s = sec();
        let rejected = app(&s)
            .oneshot(req("GET", "/healthz", Some("evil.test"), &[]))
            .await
            .unwrap();
        // security_headers is outermost, so guard rejections carry the headers too
        assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
        assert_eq!(rejected.headers()[header::CONTENT_SECURITY_POLICY], CSP);
        assert_eq!(
            rejected.headers()[header::X_CONTENT_TYPE_OPTIONS],
            "nosniff"
        );
        let ok = app(&s)
            .oneshot(req("GET", "/healthz", Some("127.0.0.1:8787"), &[]))
            .await
            .unwrap();
        assert_eq!(ok.headers()[header::CONTENT_SECURITY_POLICY], CSP);
        assert_eq!(ok.headers()[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(ok.headers()[header::REFERRER_POLICY], "no-referrer");
        assert!(ok.headers().get(header::CACHE_CONTROL).is_none());
        assert!(
            ok.headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .is_none()
        );
    }

    #[tokio::test]
    async fn bad_host_beats_bad_origin() {
        let s = sec();
        let res = app(&s)
            .oneshot(req(
                "POST",
                "/open",
                Some("evil.test"),
                &[("origin", "http://evil.test")],
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        assert_eq!(code(res).await, "forbidden_host");
    }

    #[test]
    fn path_only_span_has_no_query() {
        use std::sync::{Arc, Mutex};
        use tracing_subscriber::fmt::MakeWriter;
        #[derive(Clone, Default)]
        struct Buf(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Buf {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                lock(&self.0).extend_from_slice(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        impl<'a> MakeWriter<'a> for Buf {
            type Writer = Buf;
            fn make_writer(&'a self) -> Buf {
                self.clone()
            }
        }
        let buf = Buf::default();
        let sub = tracing_subscriber::fmt()
            .with_writer(buf.clone())
            .with_ansi(false)
            .finish();
        tracing::subscriber::with_default(sub, || {
            let r = Request::builder()
                .uri("/auth?token=SECRETTOKEN")
                .body(())
                .unwrap();
            let span = path_only_span(&r);
            let _g = span.enter();
            tracing::info!("inside");
        });
        let out = String::from_utf8(lock(&buf.0).clone()).unwrap();
        assert!(out.contains("path=/auth"), "{out}");
        assert!(
            !out.contains("SECRETTOKEN") && !out.contains("token="),
            "{out}"
        );
    }

    #[test]
    fn auth_query_debug_is_redacted() {
        let q = AuthQuery {
            token: "SECRETTOKEN".into(),
        };
        assert!(!format!("{q:?}").contains("SECRETTOKEN"));
    }

    #[test]
    fn csp_matches_contract() {
        assert_eq!(
            CSP,
            "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; \
frame-ancestors 'none'; base-uri 'none'; form-action 'self'; object-src 'none'"
        );
        assert!(!CSP.contains("unsafe-"));
    }

    #[test]
    fn exempt_list_is_pinned() {
        assert_eq!(EXEMPT_PATHS, ["/healthz", "/auth", "/api/v1/kite/callback"]);
    }

    #[test]
    fn loopback_guard() {
        assert!(ensure_loopback(loopback_addr(8787)).is_ok());
        for a in [
            "0.0.0.0:8787",
            "[::]:8787",
            "[::1]:8787",
            "192.168.1.5:8787",
        ] {
            let addr: SocketAddr = a.parse().unwrap();
            assert!(
                matches!(
                    ensure_loopback(addr),
                    Err(SecurityError::NonLoopbackBind(_))
                ),
                "{a}"
            );
        }
    }

    #[test]
    fn kite_nonce_is_single_use_and_unknown_rejected() {
        let s = sec();
        let n = s.mint_kite_nonce().unwrap();
        assert_eq!(n.len(), 32);
        assert!(!s.take_kite_nonce("unknown"));
        assert!(s.take_kite_nonce(&n));
        assert!(!s.take_kite_nonce(&n));
    }

    #[test]
    fn kite_nonce_expires() {
        let (s, clock) = sec_with(Ttls::DEFAULT);
        let n = s.mint_kite_nonce().unwrap();
        clock.advance(NONCE_TTL);
        assert!(!s.take_kite_nonce(&n));
        let m = s.mint_kite_nonce().unwrap();
        clock.advance(before(NONCE_TTL));
        assert!(s.take_kite_nonce(&m));
    }

    #[test]
    fn debug_output_redacts_secrets() {
        let s = sec();
        let tok = token(&s);
        let sess = Session {
            csrf: "supersecretcsrf".into(),
        };
        assert!(!format!("{s:?}").contains(&tok));
        assert!(!format!("{sess:?}").contains("supersecretcsrf"));
    }

    #[test]
    fn launch_url_dev_points_at_vite() {
        let s = sec();
        assert!(
            s.launch_url(PORT, true)
                .unwrap()
                .starts_with("http://localhost:5173/auth?token=")
        );
        assert!(
            s.launch_url(PORT, false)
                .unwrap()
                .starts_with("http://127.0.0.1:8787/auth?token=")
        );
    }
}
