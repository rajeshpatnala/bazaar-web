---
paths:
  - "crates/server/**"
---
# Server (axum)

- Load `bz-web-api` (and `bz-security` for the launch token, sessions, Host/Origin checks, CSRF, CSP). Handlers stay thin; logic lives in the crates.
- Every route except `/healthz`, `/auth` and `/api/v1/kite/callback` needs a session; non-GET needs the Origin check and the CSRF token; no route places an order directly.
- Bind `127.0.0.1` only (no bind flag or config); never `0.0.0.0`, any CORS layer, `SameSite=None`, a skipped Host allowlist, or logging of cookies, tokens or request bodies.
- Tests run the router in-process (`tower::ServiceExt::oneshot`) with temp dirs and fakes; order/approval/kill-switch routes need a `risk-guardian` review.
