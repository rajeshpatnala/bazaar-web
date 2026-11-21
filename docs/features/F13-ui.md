# F13 - UI screens, wave 1 (preview B, light premium, mocked API)

Status: building (UI wave 1; slice 0 of 5: shell and tokens)
Summary: a light, beginner-friendly shell with a permanent disclaimer, then an "Ideas to consider today" screen, a paper-order review dialog and a paper overview, all driven by MSW mocks.
Owner area: `web/` (nothing in `crates/`). Plan: `.agents/plans/F13-ui-slice1.md`. Preview: `previews/B-light-premium.html`.

## Acceptance criteria
- Any route (incl. not-found, session-ended, error) shows the disclaimer as a fixed footer: no props, no dismiss, no condition.
- Mode=paper: header shows the PAPER badge and a non-interactive "Live: off (set on the server)" note; no control can change the mode.
- Every amount goes through `formatInr` (Indian grouping, string/BigInt); every timestamp through `formatIst` / `formatIstIso`, whatever the browser zone.
- `/ideas`: cards with symbol, Buy/Sell tag, Entry/Stop/Target/Max loss, "Why this idea?"; loading, empty, error states; copy never says "will", "guaranteed" or "sure".
- "Review paper order": accessible dialog (focus trap, Esc, labelled) with PAPER badge, quantity, total, charges, max loss, time IST, "Not now" / "Approve paper order"; Approve sends the CSRF header and shows a result; no live wording.
- Each slice: unit tests, a11y basics (labels, roles, AA token contrast, reduced motion).

## What was built
- Slice 0: `web/src/styles/tokens.css` (CSS variables from preview B), `styles.css`, `app/AppShell.tsx`, `app/ModeBadge.tsx` (badge, read-only live note, server time in IST), `shared/ui/Disclaimer.tsx` (fixed footer), `formatIstIso`, aggregate `mocks/handlers.ts` with empty `ideas`/`orders`/`account` stubs, placeholder `/ideas` route.
- Slice 1 (BZ-502): `shared/ui/` Card, Tag, Button, Disclosure, Modal (labelled, Esc, Tab trap, focus return), Money (via `formatInr`), StateViews (Loading/Empty/Error), styled by `shared/ui/ui.css` from tokens only.
- Slice 2 (BZ-503): `/ideas` screen: `features/ideas/` (provisional `types.ts`, `queries.ts`, `IdeasPage.tsx` with `IdeaCard`), mock `GET /api/v1/ideas` in `mocks/ideas.ts`; shows symbol, Buy/Sell, max loss first, entry/stop/target, quantity, low-evidence, regime and data-delayed badges, Why panel; loading/empty/error states.
- Slice 3 (BZ-504): "Review paper order" button on each idea opens `features/orders/ReviewOrderDialog.tsx` (PAPER badge, quantity, max loss, total, charges, IST time; "Not now" / "Approve paper order"). Approve is the only caller of `POST /api/v1/approvals/{id}/approve`, with the `X-CSRF-Token` header (`getCsrfToken` in `api/client.ts`); result shown in the dialog. Mock routes in `mocks/orders.ts`; provisional types in `features/orders/types.ts`. No place-order route, no mode control.

## Disclaimer wording
- Owner approved 2026-11-21: "For education and personal analysis. Not investment advice; not SEBI registered; markets carry risk. Past performance does not guarantee future results." Applied in `crates/core/src/disclaimer.rs`, `web/src/shared/disclaimer.ts`, README, the compliance skill reference and the regime doc comment; byte-equality pinned by `rust_and_ts_disclaimers_are_byte_equal`.

## Decisions
- Preview B, system/Georgia fonts only, no webfonts, no new dependency.
- Disclaimer "not SEBI registered" wording was applied in PR 103 on 2026-11-21 with owner approval, in lockstep in `crates/core/src/disclaimer.rs` and `web/src/shared/disclaimer.ts` (byte-equality test in `crates/cli/tests/workspace.rs`).
- The preview's disabled Live toggle becomes a read-only note: live is chosen on the server, never in the UI.
- Time shown comes from `status.server_time`, never `Date.now()`.
- Types for ideas, order preview and account are provisional (`// PROVISIONAL`) until F13 server slices publish OpenAPI.

## Known limits
- Mock data only; no real recommendations; no server routes, WS, charts, dark theme or live-mode UI.

## Deferred
- DONE (PR 103): add "not SEBI registered" to the disclaimer in `crates/core/src/disclaimer.rs` and `web/src/shared/disclaimer.ts` together.
- Swap provisional types for generated schema types (F13 server slice).
- WebSocket events replacing polling mocks (F13 server slice).
- CR-124-4: per-idea validation deferred to BZ-509 (generated types)
- Owner to confirm the disclaimer wording, the read-only live note and the "Ideas to consider today" copy.

## History
- 2026-11-21: slice 0 (shell, tokens, disclaimer, IST helper, stubs).
- 2026-11-21: slice 1 (shared UI components, BZ-502).
- 2026-11-22: slice 2 (ideas screen, BZ-503).
- 2026-11-22: slice 3 (paper-order review dialog, BZ-504).
