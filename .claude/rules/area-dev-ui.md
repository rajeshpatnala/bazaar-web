---
paths:
  - "web/**"
---
# You are in dev-ui's area (bz-web-ui)

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

More on this area: `just docs "bz-web-ui: <topic>"`.
