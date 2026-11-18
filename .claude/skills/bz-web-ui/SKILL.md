---
name: bz-web-ui
description: Bazaar web UI under web/ - React 19 + Vite + TS, openapi-fetch client on generated types, WebSocket client, lightweight-charts v5, recommendation cards with Why? panels, permanent disclaimer, approval dialog, INR/IST formatting, browser security. Use when building or changing anything under web/.
---

# Bazaar Web UI

Scope: everything under `web/`: the React app, the typed API client, the WebSocket client, charts, the eight screens and the safety-critical widgets (recommendation card, Why? panel, disclaimer, approval dialog). Neighbours: `bz-web-api` (routes, launch-token session, CSRF, `references/api-contract.md` is the endpoint and WebSocket source of truth), `bz-architecture` (event flow, resilience), `bz-security` (localhost security template: Host/Origin checks, CSP, cookies, red flags), `bz-quant` (the only list of recommendation fields), `bz-execution` (approval queue, modes), `bz-trading-discipline` (what the UI must never soften), `bz-testing` (pyramid, CI), `bz-repo-structure` (scripts, justfile). Open `references/api-client.md` for client, query keys and the WebSocket hook; open `references/lightweight-charts-v5.md` before any chart code. Disclaimer text and wording rules: `bz-india-markets` `references/compliance-and-disclaimer.md`. Mocks and Playwright: `bz-testing` `references/ui-mocked-api.md`.

## 1. Default stack

| Concern | Choice | Note |
|---|---|---|
| UI | React 19.3, Vite 8, TypeScript `~6.0` strict | not TS 7 until typescript-eslint and openapi-typescript support it (pins in `bz-repo-structure` manifests) |
| Routing | react-router 8.5 (`createBrowserRouter`; `RouterProvider` from `react-router/dom`) | one package `react-router`; `react-router-dom` is gone since v8 |
| API | `openapi-fetch` 0.17 + `openapi-typescript` 7.13 | types in `web/src/api/schema.d.ts`, generated, never edited |
| Server state | TanStack Query 5 | one `queryOptions` factory per endpoint |
| UI state | zustand 5 | filters, selected symbol, live ticks only |
| Charts | lightweight-charts 5.3 (ESM only) | imperative wrapper, see reference |
| Tests | Vitest 5 + Testing Library + MSW 3; Playwright 1.65 (CI) | mock layer defined in `bz-testing` |

Versions per `bz-repo-structure` `references/manifests.md` (checked 2026-11-18); run `pnpm outdated` before bumping and keep pins in `web/package.json` only.

## 2. Golden rules

1. The UI renders and asks; it never decides. Sizing, risk, gate checks, evidence thresholds and costs come from API responses. Do not recompute rupee risk or the low-evidence cutoff in TypeScript.
2. No hand-written DTOs. Request and response types come from `schema.d.ts` (`components["schemas"][...]`), regenerated with `just openapi` after any server change. A server change that breaks a screen breaks `pnpm typecheck`.
3. Money is never a JS `number`. The API carries money as decimal strings (`"1234.50"`); format by string manipulation, never `parseFloat` then arithmetic. Quantities and counts are numbers.
4. All times shown are IST (`Asia/Kolkata`) whatever the browser's zone. Wire format is UTC epoch seconds or RFC 3339; convert only at the display edge.
5. The disclaimer is rendered by the app shell outside every error boundary, suspense boundary and route, with no prop or setting that hides it, at every window width. A test proves it.
6. A recommendation is never rendered without the fields the UI displays (section 6). A missing field is a type error, not an empty cell.
7. No order is placed by an effect, a timer, a keyboard shortcut, a WebSocket message or a query. Only a user click in the approval dialog calls `POST /api/v1/approvals/{id}/approve` (there is no place-order endpoint) and the server re-runs the gate. Paper approvals are the same click as `semi_auto`; there is no auto-accept.
8. PAPER and LIVE are labelled in text plus colour on every order surface. Never colour alone.
9. No `any`, no `as` casts on API results, no `dangerouslySetInnerHTML`, no remote scripts, fonts or images, no tokens in `localStorage`/`sessionStorage`.
10. Every screen has loading, empty ("No high-quality setups today" is a first-class state), error and stale-data ("data delayed") states.
11. Beginner first: jargon gets a `<Term>` tooltip (plain English), numbers get units, risk is shown in rupees before reward.

## 3. Layout

```text
web/
  index.html  vite.config.ts  vitest.config.ts  playwright.config.ts  package.json
  src/
    app/        providers (QueryClient, router), AppShell, ErrorBoundary, routes, SessionGate (session-ended screen)
    features/   market-overview/ scanner/ chart/ option-chain/ backtest-lab/ paper-trading/ journal/ settings/
                each: queries.ts (queryOptions), components/, model/ (zustand, pure helpers), index.ts
    shared/     ui/ (Disclaimer, DataDelayedBadge, ModeBadge, Term, Money, IstTime), lib/ (format-inr.ts, format-ist.ts), disclaimer.ts
    api/        schema.d.ts (generated), client.ts (openapi-fetch + middleware), ws.ts (socket + hooks), keys.ts (query keys)
    mocks/      handlers.ts (MSW REST + WebSocket), fixtures.ts; loaded only when VITE_MOCK_API=1
    test/       setup.ts, factories.ts (typed from schema.d.ts)
```

Screens: market-overview (indices, VIX, breadth, sector heatmap, movers, regime badge), scanner (ranked recommendations; filters intraday/swing/positional/options/long-term; long-term is a mode switch in the shell), chart, option-chain (Greeks, OI bars, payoff, margin from the broker margin API only), backtest-lab, paper-trading (live P&L net of charges), journal (win rate by strategy, mistakes, R distribution), settings (broker login status, risk limits read-only from the API, alerts). Features import only each other's `index.ts`; `shared` and `api` import nothing from features.

## 4. API client and requests

In short: One client in `api/client.ts`, created with `createClient<paths>({ baseUrl: location.origin, credentials: "same-origin" })` and called with full paths (`api.GET("/api/v1/recommendations")`, because the generated `paths` include the prefix; ...
Full detail, examples and patterns: `references/api-client-and-requests.md` - read it before changing this area.

## 5. Session, WebSocket, stale data

In short: Session** (`app/SessionGate`): the owner gets in through the one-time link that `just run` prints and opens (`/auth?token=...`, handled by the server, see `bz-security`); the UI has no password or login form. On start the gate loads `GET ...
Full detail, examples and patterns: `references/session-websocket-stale-data.md` - read it before changing this area.

## 6. Recommendation card

In short: The DTO is `components["schemas"]["RecommendationDto"]`, generated from the server. The field list lives only in `bz-quant`; this skill lists what the card shows and must not restate or extend the DTO. Money arrives as decimal strings. The ...
Full detail, examples and patterns: `references/recommendation-card.md` - read it before changing this area.

## 7. Formatting: INR (en-IN grouping) and IST

Indian grouping is last three digits, then pairs: `₹1,00,000.00`. Format from the decimal string; do not feed money to `Intl.NumberFormat` arithmetic.

```ts
// shared/lib/format-inr.ts
export function formatInr(value: string, opts: { decimals?: number } = {}): string {
  const decimals = opts.decimals ?? 2;
  const m = /^(-)?(\d+)(?:\.(\d+))?$/.exec(value.trim());
  if (!m) throw new Error(`bad money value: ${value}`);
  const [, neg = "", int, frac = ""] = m;
  const head = int.slice(0, -3).replace(/\B(?=(\d{2})+(?!\d))/g, ",");
  const grouped = int.length > 3 ? `${head},${int.slice(-3)}` : int;
  const cents = frac.padEnd(decimals, "0").slice(0, decimals);   // truncate; the server already rounded
  return `${neg}₹${grouped}${decimals ? "." + cents : ""}`;
}
// formatInr("100000") === "₹1,00,000.00"; formatInr("-1234.5") === "-₹1,234.50" (sign before the symbol)

// shared/lib/format-ist.ts
const IST = new Intl.DateTimeFormat("en-IN", {
  timeZone: "Asia/Kolkata", day: "2-digit", month: "short", hour: "2-digit", minute: "2-digit", hour12: false,
});
export const formatIst = (epochSeconds: number): string => `${IST.format(epochSeconds * 1000)} IST`;
```

Unit-test grouping at 999, 1000, 99999, 100000, 12345678 (`₹1,23,45,678.00`), zero, negatives and a non-numeric rejection. Compact values (market cap, turnover) use lakh/crore helpers (`bz-india-markets`), not "M/B". Time tests run with `TZ=America/New_York` to prove the browser zone is ignored.

## 8. Disclaimer, always visible

Text, wording rules and constant locations: `bz-india-markets` `references/compliance-and-disclaimer.md` (only copy). The UI imports `DISCLAIMER` from `shared/disclaimer.ts` and never retypes or paraphrases it.

`AppShell` renders `<Disclaimer />` as a footer (`role="contentinfo"`), sibling of the router outlet, outside any boundary, also on the session-ended screen, wrapping (never truncated or hidden) in narrow windows. No dismiss button, no setting, no conditional. Exports and journal screenshots include it.

```tsx
it.each(["/", "/scanner", "/chart/RELIANCE", "/options", "/backtest", "/paper", "/journal", "/settings"])(
  "renders the disclaimer on %s, also when the API is down",
  async (path) => {
    server.use(http.all("/api/*", () => HttpResponse.error()));   // every call fails
    renderApp({ route: path });
    expect(await screen.findByText(DISCLAIMER)).toBeVisible();
    expect(screen.getByRole("contentinfo")).toHaveTextContent(DISCLAIMER);
  },
);
```

## 9. Order approval dialog

Opened from the response to `POST /order-intents` (pending approval), a paper-trading action or the approval list (`bz-execution`). Contents: a mode banner ("PAPER: virtual money" or "LIVE: real money, real broker"; text plus colour), instrument, side, quantity or lots, order type and price, stop, estimated charges (from the server), **maximum loss in rupees and as % of capital shown before the Approve button**, the gate result listing each rule pass or fail, the expiry countdown, and data-as-of.

- Real modal: `<dialog>` with `showModal()`, initial focus on Cancel, Escape cancels, Enter does not approve.
- Approve is disabled while any gate rule fails. An override (only where the gate allows one) needs a typed reason; the UI sends it, the server audits it.
- Approve sends `POST /approvals/{id}/approve` exactly once (disable on click, ignore repeats). The server re-runs the gate with fresh prices; if the risk figure changed or the gate now fails, show it and require a new click.
- Expired approvals show an expired state with "Refresh", never a silent re-submit.
- LIVE strings and styling stay hidden unless `GET /status` reports live as enabled.

## 10. Layout and accessibility

Desktop browser first (the owner uses it on the Mac): layouts work from about 1024 px down to a half-screen window (~720 px) without horizontal page scroll; wide tables scroll inside their own container; charts keep a real CSS height (for example `min-height: 260px`) and `autoSize`; the approval dialog shows the rupee risk and both buttons without scrolling. No fixed pixel widths on screens.

Keyboard-reachable everything, visible focus, labels on inputs, `aria-expanded` on Why? toggles, `role="status"` on badges, colour-blind-safe up/down (arrows or +/- next to red and green), tabular numerals for price columns, a text summary (last price, change) beside every chart canvas, `prefers-reduced-motion` respected for flashing prices.

## 11. Security in the browser

CSP, cookie attributes and the full red-flag scan are owned by `bz-security` (browser security template; run its scan on `web/` changes). The UI side:

- The session is an `HttpOnly` cookie set by the server; the UI never sees or stores it. No tokens, passwords or account data in `localStorage`, `sessionStorage`, URLs or `console.log`. The CSRF token lives in memory only.
- No `dangerouslySetInnerHTML`, `innerHTML`, `eval`, `new Function`, remote scripts, fonts, images or analytics. Server strings render as text.
- `VITE_` variables are inlined into the bundle: never put secrets in them. `.env` is not read by agents; the UI needs none.
- The production CSP is set by the server; do not add inline scripts or `style` attributes beyond what the template allows (the Playwright CSP spec decides whether charts need anything; never widen the policy in code; any change goes to CONTRACTS first).
- Patterns unique to this skill: `localStorage|sessionStorage`, `dangerouslySetInnerHTML`, `fetch\(` outside `api/`, `credentials:\s*"include"`, `approve` calls outside a click handler, timers or effects that reach an approve call.

## 12. Dev server

`vite.config.ts` proxies the API and WebSocket to the local server so the browser sees one origin; `just ui-dev` (human-only) starts a debug server and Vite together; the launch link opens on `localhost:5173`:

```ts
server: { port: 5173, strictPort: true, proxy: {
  "/api": { target: "http://127.0.0.1:8787", changeOrigin: true, ws: true },   // Host rewritten for the allowlist
  "/auth": { target: "http://127.0.0.1:8787", changeOrigin: true },
} }
```

The server accepts the `http://localhost:5173` origin only in debug builds (`bz-security`). `dev:mock` (`VITE_MOCK_API=1`) starts MSW in the browser and needs no server; `main.tsx` imports `./mocks/browser` only behind that flag so production builds drop it (grep `dist/` for `mocks/`). Regenerate types with `just openapi` (never hand-edit `schema.d.ts`; commit the result).

## 13. Tests (unit tests in every PR)

Vitest 5 + Testing Library + MSW with the setup in `bz-testing` `references/ui-mocked-api.md` (`src/test/setup.ts`, `src/mocks/handlers.ts`, factories typed from `schema.d.ts`). Never touch the real server or network; unhandled requests fail the test. Must-have tests:

- disclaimer on every route, on the session-ended screen and in error state; card renders every displayed field and the gate-blocked state; low-evidence badge iff flagged; empty state;
- `formatInr` and `formatIst` tables; request middleware (CSRF header on POST and absent on GET, `same-origin`, 401 shows the session-ended screen);
- session gate (`/auth/me` ok renders the app; 401 renders the session-ended screen with no form and no retry loop);
- approval dialog (Cancel default focus, Approve disabled on gate fail, one approve request, PAPER/LIVE label, rupee risk visible before the click);
- WebSocket hook with a fake socket and fake timers (backoff schedule, resubscribe on open, stale badge while disconnected, burst of ticks gives one render);
- chart wrapper with `lightweight-charts` mocked (`addSeries`, `setData`, `createSeriesMarkers`, `createPriceLine`, `remove` once on unmount).

Commands: `pnpm -C web typecheck`, `lint`, `test:ci`. T1 evidence for UI work is one screenshot of the changed screen from `dev:mock`.

## 14. Checklist

- [ ] New screen: folder under `features/`, loading/empty/error/stale states, usable at a half-screen window (~720 px), disclaimer test still green.
- [ ] Types come from `schema.d.ts` after `just openapi`; no hand-written DTOs, no casts.
- [ ] No money arithmetic in TS; ₹ via `formatInr`, times via `formatIst`.
- [ ] Recommendation card shows what section 6 lists; Why? text comes from the server.
- [ ] Approval dialog safety behaviour unchanged (`risk-guardian` reviews recommendation UI changes).
- [ ] Charts disposed on unmount, markers and price lines cleaned on data change, live updates throttled.
- [ ] No storage of tokens, no `dangerouslySetInnerHTML`, no placeholder text left behind.
- [ ] `pnpm -C web typecheck && pnpm -C web lint && pnpm -C web test:ci` pass; one mocked screenshot attached for T1.
