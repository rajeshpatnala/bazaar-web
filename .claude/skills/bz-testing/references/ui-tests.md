# 6. UI tests (Vitest + MSW)

(From `bz-testing` SKILL.md; open when the task touches this area.)


The single definition of the mock layer, handlers, setup, `dev:mock`, Playwright config and CI steps is `references/ui-mocked-api.md`; `bz-web-ui` and `bz-repo-structure` link to it. Open it when writing the first UI test, a hook test, a WebSocket test or the Playwright config.

Essentials:
- Shared handlers in `web/src/mocks/handlers.ts` (REST and the `/api/v1/ws` link) serve deterministic fixtures; the mock enforces the same `X-CSRF-Token` rule as the server, so the UI's CSRF middleware is exercised. Per-test behaviour uses `server.use(...)`; `afterEach` resets it.
- `server.listen({ onUnhandledFrame: "error" })` (MSW 3 name; it was `onUnhandledRequest` in MSW 2): an unanticipated API call fails the test.
- Fresh `QueryClient` per test (`retry: false`), rendered through one `renderWithProviders` helper (router + query client).
- Query by role and visible text, not class names or test ids unless necessary. Assert request bodies and headers by recording requests inside the override handler.
- lightweight-charts needs a canvas: `vi.mock` the module in component tests and unit-test the pure mappers separately (`bz-web-ui` `references/lightweight-charts-v5.md`).
- Must-have UI assertions: disclaimer on every route and in error state; "data delayed" badge when the socket drops or ticks stop; approval dialog shows PAPER/LIVE label and rupee risk before Approve is enabled, and exactly one approve request is sent per click (a 409 from a second attempt shows a plain message); low-evidence badge under 30 trades; Why? panel renders each field; a 401 from `/auth/me` shows the signed-out state; no order-related request fires without the user's click.
- Formatting tests (`en-IN` grouping, rupee sign, IST via `Asia/Kolkata`) per `bz-web-ui` section 7, run with `TZ=UTC` and `TZ=America/New_York`.
- Fake timers (`vi.useFakeTimers({ shouldAdvanceTime: true })`, `await vi.advanceTimersByTimeAsync(...)`) for throttles, reconnect backoff and stale timers.

Playwright smoke runs against `dev:mock` (port 5173) in CI only; agents capture one screenshot of the changed screen from `just app-dev` for T1 evidence instead. One spec per screen: loads with no console errors, shows the disclaimer, renders the recommendation card from fixtures, paper approval flow; plus a CSP spec on a production-style build. Spec list and config: `references/ui-mocked-api.md`.
