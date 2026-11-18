# UI testing with MSW (single definition)

Open when writing the first Vitest test, a hook or WebSocket test, the mock handlers, or the Playwright setup. This file is the only definition of the UI mock layer, the Vitest setup, `dev:mock`, and the Playwright config and CI steps; `bz-web-ui` and `bz-repo-structure` link here. Endpoints, CSRF header and WebSocket message types come from `bz-web-api` `references/api-contract.md`; the CSP comes from `bz-security`. MSW docs: https://mswjs.io/docs/ (MSW 3 migration page: `/docs/migrations/2.x-to-3.x`; re-check when bumping).

## 1. Names (fixed by the contract) and additions

| Thing | Name |
|---|---|
| Dev script | `"dev:mock": "VITE_MOCK_API=1 vite --port 5173 --strictPort"` (`just app-dev` runs it) |
| Handlers (REST + WebSocket) | `web/src/mocks/handlers.ts`; fixtures `web/src/mocks/fixtures.ts` |
| Node server / browser worker | `web/src/mocks/server.ts` (`setupServer`), `web/src/mocks/browser.ts` (`setupWorker`) |
| Vitest setup | `web/src/test/setup.ts`; config `web/vitest.config.ts` |
| DTO factories, render helper | `web/src/test/factories.ts`, `web/src/test/render.tsx` |
| Scripts | `"test": "vitest"`, `"test:ci": "vitest run --coverage --passWithNoTests"`, `"e2e": "playwright test"` |
| Playwright | `web/playwright.config.ts`, specs in `web/e2e/`, `baseURL` `http://localhost:5173` |
| WebSocket message types | the `WsOut`/`WsIn` schemas generated from the server (list in `bz-web-api` `references/api-contract.md` section 4) |

## 2. MSW 3 facts that differ from older guides

Verified 2026-11-18 against the MSW site and release notes (3.0 shipped 2026-09-28; 3.0.x current):

- Node 22 or newer; the package is ESM-only.
- Import from subpaths, not the root: `http`, `HttpResponse` from `msw/http`; `ws` from `msw/ws`; `setupServer` from `msw/node`; `setupWorker` from `msw/browser`; `delay`, `passthrough` from `msw/utils/...`.
- The option is `onUnhandledFrame` (was `onUnhandledRequest`), values `"warn"` (default), `"error"`, `"bypass"`; it now also covers WebSocket connections. A custom callback receives `{ frame, defaults }`.
- `worker.stop()` returns a Promise; starting a server or worker twice throws, so one setup file owns `listen()`.
- Fake timers: MSW no longer patches `setTimeout`, so Vitest fake timers do not stall handlers.
- `ws.link(url)` mocks WebSocket in the browser and Node; the link has `addEventListener("connection", ({ client }) => ...)`, `clients`, `broadcast(data)`. The lifecycle event on `server.events` is named `websocket:connection`. Check the installed typings if a name does not resolve.

## 3. Vitest setup

```ts
// web/vitest.config.ts  (Vitest 5 does not look in parent folders)
import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    globals: false,
    css: false,
    coverage: { provider: "v8", thresholds: { lines: 75 } },
  },
});
```

```ts
// web/src/test/setup.ts
import { afterAll, afterEach, beforeAll } from "vitest";
import { cleanup } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { server } from "../mocks/server";
import { resetMockDb } from "../mocks/handlers";
import { setCsrfToken } from "../api/client";

beforeAll(() => server.listen({ onUnhandledFrame: "error" }));   // closed network: surprises fail the test
afterEach(() => {
  cleanup();
  server.resetHandlers();   // drops per-test server.use(...) overrides, REST and WS
  resetMockDb();            // in-memory approvals/intents back to fixtures
  setCsrfToken(null);
});
afterAll(() => server.close());
```

`web/src/mocks/server.ts`: `export const server = setupServer(...handlers)`. `web/src/mocks/browser.ts`: `export const worker = setupWorker(...browserHandlers)`. Both build on the same `handlers` array, so the browser mock and the tests cannot diverge.

The client uses `baseUrl: location.origin` and full `/api/v1/...` paths (CONTRACTS section 4), which also keeps Node's `Request` happy under jsdom; handlers use the same full paths.

## 4. Shared handlers, typed from the generated schema

Handlers are the executable copy of the API contract. Types come from `schema.d.ts` (never hand-written), so when the server DTO changes and `just openapi` regenerates the file, `pnpm -C web typecheck` fails in `handlers.ts` and `factories.ts` until the mocks follow.

```ts
// web/src/mocks/handlers.ts
import { http, HttpResponse } from "msw/http";
import { ws } from "msw/ws";
import type { paths, components } from "../api/schema";
import { db, fixtures } from "./fixtures";

export const API = "/api/v1";
export const MOCK_CSRF = "mock-csrf-token";

type JsonOf<Op, S extends number> =
  Op extends { responses: Record<S, { content: { "application/json": infer B } }> } ? B : never;
type GetBody<P extends keyof paths> = paths[P] extends { get: infer Op } ? JsonOf<Op, 200> : never;
type Problem = components["schemas"]["Problem"];

// Same shape as the server: the UI switches on `code`, never on `detail`.
export const problem = (status: number, code: string, title: string, detail: string, ext: Record<string, unknown> = {}) =>
  HttpResponse.json<Problem>({ type: `urn:bazaar:problem:${code}`, code, title, status, detail, ...ext } as Problem,
    { status, headers: { "content-type": "application/problem+json" } });

// Same rule as the server: every non-GET needs the CSRF header from /auth/me.
const csrfOk = (r: Request) => r.headers.get("x-csrf-token") === MOCK_CSRF;

export const live = ws.link(`ws://${location.host}${API}/ws`);

export const handlers = [
  http.get(`${API}/auth/me`, () =>
    HttpResponse.json<GetBody<"/api/v1/auth/me">>({ csrf_token: MOCK_CSRF, mode: "paper", live_enabled: false })),
  http.get(`${API}/recommendations`, () =>
    HttpResponse.json<GetBody<"/api/v1/recommendations">>(fixtures.recommendations)),
  http.post(`${API}/order-intents`, ({ request }) =>
    csrfOk(request) ? HttpResponse.json(db.createIntent(), { status: 201 })
                    : problem(403, "csrf_invalid", "Forbidden", "CSRF token missing or wrong")),
  http.post(`${API}/approvals/:id/approve`, ({ request, params }) => {
    if (!csrfOk(request)) return problem(403, "csrf_invalid", "Forbidden", "CSRF token missing or wrong");
    const out = db.approve(String(params.id));            // { decided, approval }: second call has decided = true
    return out.decided
      ? problem(409, "approval_already_decided", "Conflict", "Already decided", { approval: out.approval })
      : HttpResponse.json(out.approval);
  }),
  // ...one handler per read endpoint the screens use; field names come from schema.d.ts

  live.addEventListener("connection", ({ client }) => {
    client.send(JSON.stringify({ type: "data_status", data: fixtures.dataStatusFresh }));
  }),
];

// Browser worker only (dev:mock): an unmocked call shows a loud 501 instead of a confusing UI state.
// The Node `setupServer` uses `handlers` alone, so an unanticipated call trips onUnhandledFrame: "error".
export const browserHandlers = [...handlers,
  http.all(`${API}/*`, ({ request }) =>
    problem(501, "not_mocked", "Not mocked", `${request.method} ${new URL(request.url).pathname}`)),
];
```

Rules for handlers and fixtures:
- Fixtures are deterministic: fixed ids, fixed epoch seconds (never `Date.now()`), money as decimal strings. Build them with factories (`makeRecommendation({ ... })`, `makeApproval`, `makeTick`) typed `components["schemas"][...]`; no object literal per test.
- Mutable mock state lives in one `db` object with `resetMockDb()` (tests reset in `afterEach`; a browser reload resets it).
- There is no place-order route. Order flow is `order-intents`, `approvals/{id}/approve`, `reject` only; do not add a handler for anything else.
- Mirror server behaviour that the UI relies on (403 without CSRF, 401 without session, 409 on second approve, problem+json errors). Do not mirror business logic: sizing, gate and costs stay on the server; fixtures hold the finished numbers.
- Mocks never contain tokens, account ids or real prices from a live feed.

## 5. Component tests

Render helper (`web/src/test/render.tsx`): fresh `QueryClient` (`retry: false`, `gcTime: 0`) plus a `createMemoryRouter` at the requested route. Seed the CSRF token the way the app does (call `GET /auth/me` through the same provider, or `setCsrfToken(MOCK_CSRF)` in the helper).

Per-test override and request recording (record inside the handler; it avoids depending on MSW lifecycle event names):

```tsx
it("shows mode and rupee risk before enabling approval, and sends one approve per click", async () => {
  const approves: Request[] = [];
  server.use(
    http.get(`${API}/approvals/:id`, () => HttpResponse.json(makeApproval({ mode: "paper", /* risk field per schema */ }))),
    http.post(`${API}/approvals/:id/approve`, ({ request }) => {
      approves.push(request.clone());
      return HttpResponse.json(makeApproved());
    }),
  );
  renderWithProviders(<ApprovalDialog approvalId="a-1" />);

  expect(await screen.findByText(/paper/i)).toBeInTheDocument();
  expect(screen.getByText(/₹1,000\.00/)).toBeInTheDocument();
  expect(approves).toHaveLength(0);                                  // nothing sent until the click

  const approve = screen.getByRole("button", { name: /approve/i });
  await userEvent.dblClick(approve);                                 // double click
  expect(approves).toHaveLength(1);
  expect(approves[0].headers.get("x-csrf-token")).toBe(MOCK_CSRF);
});
```

Error paths: override with `problem(422, "gate_blocked", ...)` and assert the UI branch chosen by `code` and that the plain-English text is shown, not JSON or a stack; override with 401 for `auth/me` and assert the signed-out state; `HttpResponse.error()` for a network failure and assert the error state plus disclaimer still visible. Slow responses: `await delay(...)` from `msw/utils/delay` to test loading states.

## 6. WebSocket and stale data

`live` (section 4) is the one link for `/api/v1/ws`; the default handler greets with a fresh `data_status`. Tests push or drop through the link:

```tsx
it("marks data delayed when the socket drops, and recovers after reconnect", async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  renderWithProviders(<AppShell />);
  await screen.findByText(/NIFTY 50/);

  live.clients.forEach((c) => c.close(1011, "server restart"));      // abnormal close
  expect(await screen.findByText(/data delayed/i)).toBeInTheDocument();

  await vi.advanceTimersByTimeAsync(2_000);                          // first backoff step
  expect(await screen.findByText(/live/i)).toBeInTheDocument();     // reconnected, greeted again
  vi.useRealTimers();
});

it("applies a burst of ticks with one render", async () => {
  renderWithProviders(<PriceHeader symbol="NIFTY 50" />);
  await screen.findByText(/NIFTY 50/);
  for (const ltp of ["24500.00", "24500.50", "24501.00"]) {
    live.broadcast(JSON.stringify({ type: "tick", data: makeTick({ ltp }) }));
  }
  expect(await screen.findByText(/24,501\.00/)).toBeInTheDocument();
});
```

There is no auth close code: test that an abnormal close makes the client call `GET /api/v1/auth/me` and that a 401 there stops reconnecting and shows the session-ended screen, while a network error keeps backing off. Also test that `resync` invalidates the named query groups and `job_progress` updates the backtest job cache. To change server behaviour for one test, `server.use(live.addEventListener("connection", ...))`. If MSW's interception does not catch jsdom's `WebSocket` in the installed versions, inject a fake `WebSocket` class into the provider instead (pattern in `bz-web-ui` `references/api-client.md`); keep the tests the same.

Never use real timers for backoff or staleness: fake timers plus `advanceTimersByTimeAsync`. Thresholds (stale after N seconds) come from the mocked `GET /status` response, not a UI constant.

## 7. Charts (lightweight-charts 5.x)

jsdom has no canvas. (1) Pure mappers (`toCandleData`, `toMarkers`, `toPriceLines`, level builders) are plain functions tested with fixtures: ordering, duplicate timestamps, IST conversion, missing values. (2) The thin `<Chart>` wrapper is tested with `vi.mock("lightweight-charts", ...)` returning fake `createChart` / `addSeries`: created once, `setData` gets mapped data, resize on container change, `remove()` on unmount; series use `addSeries(CandlestickSeries, ...)`. Visual correctness is checked once by a Playwright screenshot.

## 8. Browser mock (`dev:mock`)

```ts
// web/src/main.tsx, before rendering
if (import.meta.env.VITE_MOCK_API === "1") {
  const { worker } = await import("./mocks/browser");
  await worker.start({ onUnhandledFrame: "bypass", serviceWorker: { url: "/mockServiceWorker.js" } });
}
```

- The worker script is generated once with `pnpm dlx msw init public/ --save` and committed (check the CLI flags for the installed version). `onUnhandledFrame: "bypass"` is right here because Vite serves many same-origin assets; unknown API calls are caught by the last `http.all("/api/v1/*")` handler (501 problem) instead.
- Vite replaces the flag statically, so `pnpm build` drops the dynamic import; verify by grepping `dist/` for `mocks/` and `msw`. The inert `mockServiceWorker.js` in `public/` may still be copied; it is never registered without the flag.
- `localhost` is a secure context, so the service worker registers over plain HTTP.

## 9. Playwright smoke (CI only)

```ts
// web/playwright.config.ts
import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  use: { baseURL: "http://localhost:5173", trace: "retain-on-failure" },
  webServer: [
    { command: "pnpm dev:mock", url: "http://localhost:5173", reuseExistingServer: !process.env.CI },
    // CSP spec only: production build with the mock flag, served by `vite preview` with the production CSP header
    { command: "pnpm build:mock && pnpm preview:mock", url: "http://localhost:5174", reuseExistingServer: !process.env.CI },
  ],
  projects: [{ name: "chromium", use: { browserName: "chromium" } }],
});
```

Scripts: `"build:mock": "VITE_MOCK_API=1 vite build --outDir dist-mock"`, `"preview:mock": "vite preview --outDir dist-mock --port 5174 --strictPort"` (`dist-mock/` gitignored).

Smoke steps (one spec per screen, `web/e2e/<screen>.spec.ts`): the screen loads with no `console.error` or `pageerror`; the disclaimer is visible; the scanner shows the fixture recommendation card with risk in rupees, entry zone, stop, targets, evidence and the Why? panel; the paper approval flow ends in a positions or journal entry; a second approve attempt shows the conflict message (the mock returns 409); the stale badge appears when the mock stops sending (use `page.clock` to move time instead of waiting). Use role and text locators and web-first assertions; no `waitForTimeout`.

**CSP spec** (settles the lightweight-charts inline-style question from `bz-security`). The Vite dev server's inline preamble and HMR socket make a CSP check there noisy, so the spec targets `http://localhost:5174`. The production CSP is set by the server (`security.rs`); to avoid a second copy that drifts, the string lives in `web/e2e/csp.txt`, `vite.config.ts` feeds it to `preview.headers` when the mock-preview mode is used, and a Rust test in `bazaar-server` asserts that file equals the server's CSP constant (`include_str!` of the file). In the spec, register `document.addEventListener("securitypolicyviolation", ...)` via `page.addInitScript`, open the chart screen, and assert the violation list is empty; the failure message names the directive. Follow the `bz-security` rule (relax only `style-src-attr`, never `style-src` or scripts).

CI steps (UI job in `.github/workflows/ci.yml`): `pnpm install --frozen-lockfile`, `pnpm -C web exec playwright install --with-deps chromium`, `pnpm -C web e2e`. Agents do not run Playwright; they use `just app-dev` for one T1 screenshot and never `just ui-dev` or `just run`.

Limits: this proves the UI against fixture DTOs, not the Rust server. Server behaviour is covered by router and WebSocket tests (`bz-testing` section 5.6-5.7); the contract between them by the generated OpenAPI types, the drift check and the typed mocks. There is no browser test against the real server in CI; if one is ever added it is human-run, uses fakes behind the server, and never a live broker.
