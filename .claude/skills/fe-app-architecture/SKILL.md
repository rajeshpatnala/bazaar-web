---
name: fe-app-architecture
description: "Builds the application layer of the React + TypeScript front end: entry point and providers, TanStack Query client policy, typed TanStack Router routes with loaders and validated search params, feature modules (model/api/components), pages, loading/empty/error states and error boundaries, React Hook Form + Zod forms with server 422 mapping, Zustand client state, and route/feature tests. Use whenever adding or changing a feature, page, route, query, mutation, form, error handling or client store, or reviewing such code."
---

# App architecture

Turns the foundation into a working app shell plus one complete reference feature (`orders`) that every later feature copies. Load `fe-standards` alongside it; requires `fe-repo-foundation` to be in place.

## Best practices

Binding rules; the reference files show how to apply them.

1. **Give every piece of state one home** — the URL for shareable view state (filters, page, tab), the query cache for server data, components for transient UI state, and a global store only as a last resort.
2. **Start fetching in route loaders, read with `useSuspenseQuery`** — no request waterfalls and no `isLoading` chains in components.
3. **Share one `queryOptions` object between loader and component** — cache keys can't drift apart.
4. **Use hierarchical query keys** (`['orders', 'list', …]`) — one call invalidates a whole feature after a write.
5. **Validate every response with Zod inside the feature's `api/`** — a backend change shows up as one clear error, not a crash deep in the UI.
6. **Build features as `model/` → `api/` → `components/`, exposed only through `index.ts`** — business rules stay pure and testable, and features can't reach into each other.
7. **Keep pages thin** — they connect the URL to features and contain no fetching or business logic.
8. **Set retry policy in one place** — never retry 4xx responses or writes automatically.
9. **Give each screen state one owner** — skeletons from the router, errors from boundaries, empty states from the feature, 404s from `notFound()`.
10. **Turn errors into user messages in one function** — expected 4xx outcomes aren't reported as bugs; everything else is.
11. **Make writes feel instant with optimistic updates, always with rollback** — then invalidate, so the screen ends on server truth.
12. **Map server validation errors back onto form fields** — users see one consistent error style wherever an error came from.
13. **Add global client state only when it is client-only, not shareable, and needed far apart** — and always subscribe through narrow selectors.
14. **Test features through their real route with a mocked network** — assert what users see, not hooks or cache keys.

## Workflow

1. Read "What this playbook builds" and "Data flow at a glance" in `references/app-shell.md`.
2. `errors.ts`, `query-client.ts`, `providers.tsx`, `main.tsx` → `app-shell.md` (Entry point).
3. `router.tsx` and `root-layout.tsx` with typed search params, loaders, focus management and 404 handling → `app-shell.md` (Routing).
4. Build the `orders` feature: model (Zod schemas + pure helpers), api (`queryOptions`, mutations with optimistic rollback), components, `index.ts` → `references/feature-module.md`.
5. Pages and UI states (skeletons, empty, error fallback, `WidgetBoundary`) → `references/pages-and-states.md`.
6. Forms with server-error mapping, then Zustand store only where needed → `pages-and-states.md` (Forms, Client state).
7. Route and feature tests (filter → URL, empty, error, optimistic rollback) → `references/testing.md`. Run `pnpm verify`.

New features copy the `orders` structure: schemas first, then `queryOptions`, then components, then the page, then tests.

## Files this playbook owns (Build mode)

`src/app/main.tsx`, `src/app/providers.tsx`, `src/app/router.tsx`, `src/app/root-layout.tsx`, `src/app/query-client.ts`, `src/shared/lib/errors.ts`, `src/features/orders/**`, `src/pages/**`, `src/shared/ui/{error-fallback,widget-boundary,skeleton}`, `src/shared/stores/ui-store.ts`, `src/test/render-route.tsx`, `src/test/factories.ts`. Other playbooks request changes to `main.tsx`, `providers.tsx` and `router.tsx` through this playbook's owner.

## Done when

`pnpm verify` is green; the orders list filters through the URL (back button restores filters); detail 404s render Not Found; create form maps 422 field errors onto inputs; cancel is optimistic and rolls back on failure; route changes move focus to `#main`; every screen has loading, empty and error states with a retry.

## Reference files

Read only the file the current step needs. Paths are relative to this skill's folder (`.claude/skills/<skill>/`).

- `references/app-shell.md` — What gets built, data flow, entry point, providers, query policy, router and root layout.
- `references/feature-module.md` — The `orders` feature: model, api (queries, mutations, optimistic updates), components and public index.
- `references/pages-and-states.md` — Pages, loading/empty/error states and boundaries, forms with 422 mapping, Zustand store.
- `references/testing.md` — Route render helper, factories, model tests and page tests.
