---
name: fe-auth-realtime
description: "Implements authentication and real-time updates for the React front end: HttpOnly cookie sessions, CSRF headers, single-flight token refresh in the HTTP client, session query and permissions, protected/admin routes, login page with safe redirects, session expiry and multi-tab sync via BroadcastChannel, and a resilient WebSocket client (backoff with jitter, heartbeats, dedupe) wired into the query cache. Use whenever touching login/logout, sessions, tokens, cookies, CSRF, route guards, permissions, WebSockets, SSE or live updates, or reviewing security-sensitive front-end code."
---

# Auth & real-time

Adds sign-in, protected routes and live data on top of `fe-app-architecture`. This playbook owns the final `shared/lib/http.ts`. Load `fe-standards` alongside it.

## Best practices

Binding rules; the reference files show how to apply them.

**Authentication**

1. **Never let JavaScript hold a token** — sessions live in `HttpOnly` cookies, so an injected script can't steal them; the app learns who is signed in only from `/auth/me`.
2. **Short-lived access, rotated and path-scoped refresh** — a leaked cookie expires in minutes, and refresh-token reuse is detectable.
3. **Send a CSRF token on every write** — `SameSite` cookies are a second layer, not the only one.
4. **Handle `401` in one place: one shared refresh, retry once, never loop** — ten failing requests cause one refresh, and a real denial still surfaces.
5. **Treat the session as server state (one query), not a separate store** — nothing to keep in sync.
6. **Guard routes before their loaders run** — signed-out users never trigger protected API calls or see protected UI.
7. **Save the return path, but allow only same-origin relative redirects** — convenience without open-redirect attacks.
8. **Hide restricted areas with 404, not 403** — don't reveal what exists.
9. **UI permission checks are UX; the server enforces** — every endpoint authorizes independently.
10. **When a session ends, clear every cached response** — sign-out reloads the page, and all tabs follow via `BroadcastChannel`, so the next user never sees the previous user's data.
11. **One generic login error, plus correct `autocomplete` values** — no account enumeration, and password managers work.

**Real-time**

12. **Pick the simplest transport that is fast enough: polling, then SSE, then WebSocket** — less infrastructure, fewer failure modes.
13. **Real-time messages only update the query cache** — components never read a socket, so screens behave the same online and offline.
14. **Assume the connection will fail silently** — jittered backoff, heartbeats, reconnect on `online`, de-duplicate by event id, and refetch active queries after reconnecting.
15. **Validate every real-time payload like an API response, and check `Origin` on the socket server** — cookies are sent automatically, so any site could otherwise open a socket as the user.
16. **Test the failure paths** — concurrent `401`s, failed refresh, malicious redirects, malformed events and reconnect timing are where the bugs are.

## Workflow

1. Confirm the backend contract (cookies, endpoints, status codes) → `references/auth.md` (Auth model). If the backend differs, record it under Open questions before coding.
2. Replace `http.ts` with the CSRF + single-flight refresh version → `auth.md` (HTTP client).
3. Auth feature (session query, `useCan`, login/logout, BroadcastChannel) → `auth.md` (Auth feature).
4. Router guards, login page, `safeRedirect` → `references/routes-and-session.md`; send router edits to the `fe-app-architecture` owner as change requests.
5. Session expiry handling and multi-tab sync → `routes-and-session.md` (Session expiry and multi-tab sync).
6. Choose the transport, then build `RealtimeClient` and connect events to the cache → `references/realtime.md`.
7. Tests for refresh, CSRF, guards, backoff and event application, plus the setup mock that keeps sockets out of component tests → `references/testing-and-security.md`. Walk the security checklist. Run `pnpm verify`.

## Files this playbook owns (Build mode)

`src/shared/lib/http.ts` (final), `src/features/auth/**`, `src/app/auth-sync.ts`, `src/shared/lib/realtime.ts`, `src/app/realtime.ts`, `src/features/orders/api/orders-realtime.ts`, `src/app/authenticated-layout.tsx`, `e2e/auth.setup.ts`. Router and `main.tsx` changes go to the `fe-app-architecture` owner.

## Done when

Ten concurrent 401s trigger exactly one refresh; unsafe requests carry `X-CSRF-Token`; signed-out users are redirected to `/login?redirect=…` and return afterwards; external redirect URLs are rejected; logout in one tab signs out all tabs; a dropped socket reconnects with jittered backoff and refetches; close code 4401 triggers re-auth; every item in the security checklist is satisfied; `pnpm verify` is green.

## Reference files

Read only the file the current step needs. Paths are relative to this skill's folder (`.claude/skills/<skill>/`).

- `references/auth.md` — Backend contract, session-expiry flow, final `http.ts`, session/permission APIs and login/logout.
- `references/routes-and-session.md` — Route guards, login page, safe redirects, session expiry and multi-tab sync.
- `references/realtime.md` — Choosing a transport, the resilient WebSocket client and connecting events to the query cache.
- `references/testing-and-security.md` — Tests for auth and real-time failure paths, E2E auth setup and the security review checklist.
