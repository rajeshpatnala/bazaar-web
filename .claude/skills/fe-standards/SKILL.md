---
name: fe-standards
description: Shared engineering standards for the company's React + TypeScript front end — the stack and versions, global rules, folder layout, code style, naming, which playbook owns which area, how later playbooks override earlier ones, and the report format every sub-agent returns. Load this first whenever building, changing or reviewing any part of the front-end codebase, together with the area playbook for the task.
---

# Front-end standards (shared by every playbook)

These are the rules every agent follows regardless of which part of the app it builds. Area playbooks (`fe-repo-foundation`, `fe-app-architecture`, …) add detail; this playbook is the common ground that keeps their work consistent.

## Stack (October 2026 — install current versions with `pnpm add`, respect pins)

| Layer | Choice | Notes |
|---|---|---|
| Runtime | Node.js 24 LTS | pinned in `.nvmrc` and `engines` |
| Package manager | pnpm 11 | workspaces, catalogs, release-age cooldown |
| Language | TypeScript 6.0 (`~6.0`) | move to 7.0 (native, 8–12x faster) only when typescript-eslint supports it |
| UI | React 19.3 + React Compiler 1.0 | no hand-written `useMemo`/`useCallback` unless profiled |
| Build | Vite 8 (Rolldown) | `@vitejs/plugin-react` v6 + `@rolldown/plugin-babel` for the compiler |
| Public SSR site (optional) | Next.js 16 | see `fe-monorepo` |
| Routing | TanStack Router | typed routes and search params |
| Server state | TanStack Query | `queryOptions` per query |
| Client state | Zustand (rarely) | only for client-only, non-shareable, far-apart state |
| Forms | React Hook Form + Zod | one schema for types and validation |
| Styling | CSS Modules + design tokens | native CSS, cascade layers |
| Lint / format | Oxlint → ESLint 10 (flat) + typescript-eslint; Prettier | |
| Tests | Vitest 4.1+ (5.x available), Testing Library, MSW, Playwright, axe | |
| Monitoring | Sentry + `web-vitals` | |
| CI | GitHub Actions + merge queue | |

If documented code fails against the installed version, make the smallest fix that keeps the intent and record it as a deviation (see report format).

## Global rules (apply to every change)

1. **Automate rules instead of remembering them** — if a human must remember it, add a lint rule, type, test or CI check.
2. **Feature-first structure with one-way dependencies** — `app → pages → features → shared`; features expose only `index.ts`.
3. **Type safety end to end** — strict TypeScript, no `any`, every API response parsed with Zod at the boundary.
4. **Accessibility (WCAG 2.2 AA) and performance budgets are requirements**, checked in CI.
5. **Small, reversible changes** — PRs under ~400 lines into the integration branch (`develop`), feature flags for unfinished work. `main` moves only by approved promotion; `develop` and `main` are never deleted, force-pushed or reset.
6. **Measure in production** — real-user Core Web Vitals and error rates decide priorities.
7. **Write for the reader** — clear names, comments explain *why*, no cleverness.

## Folder layout (single app)

```text
src/
├─ app/        bootstrap: providers, router, root layout, startup wiring
├─ pages/      route entry points; thin, compose features
├─ features/<name>/{model,api,components}/index.ts
├─ shared/{ui,lib,config,styles,stores}/
└─ test/       setup, MSW handlers, render helpers, factories
```
kebab-case file names; no barrel files except each feature's `index.ts`; tests, stories and CSS modules colocated.

## Code style essentials

- `unknown` not `any`; explicit return types on exported functions; discriminated unions with exhaustive `switch` (`state satisfies never`).
- No `enum`/`namespace` (erasable syntax only); string-literal unions or `as const`.
- Named exports only; `readonly` by default; `satisfies` for config objects; branded ids where mix-ups are possible.
- React: function components, derive values during render (no `useEffect` to sync state), stable keys from ids, effects only for external systems.
- Naming: PascalCase components/types, camelCase functions (verbs), `is/has/can` booleans, `onX` props / `handleX` handlers, `useX` hooks, UPPER_SNAKE constants.

## Which playbook owns what

| Playbook | Area |
|---|---|
| `fe-repo-foundation` | configs, tooling, lint, hooks, env, HTTP client, test setup |
| `fe-app-architecture` | providers, router, data layer, feature modules, pages, errors, forms, client state |
| `fe-auth-realtime` | cookie sessions, token refresh, route guards, multi-tab sync, WebSockets/SSE |
| `fe-design-system` | tokens, theming, global CSS, core components, Storybook, Stylelint |
| `fe-testing` | test strategy, factories, mocks, visual regression, E2E, contracts, flakiness |
| `fe-performance` | budgets, RUM, LCP/INP/CLS, bundle size, third parties |
| `fe-cicd-observability` | CI/CD workflows, runtime config, deploys, flags, Sentry, alerts |
| `fe-team-workflow` | git, PR authoring and review, CODEOWNERS, AGENTS.md, ADRs |
| `fe-monorepo` | pnpm workspaces, Turborepo, shared packages, Next.js public site |
| `fe-a11y-i18n` | accessible patterns, a11y testing, react-intl, RTL, translation pipeline |

## Precedence when playbooks give different code for the same file

Later playbooks refine earlier files; the **later version wins**, but every playbook's best practices stay binding.

| Topic | Evolution | Final authority |
|---|---|---|
| `http.ts` | `fe-repo-foundation` (basic) → `fe-auth-realtime` (CSRF and refresh) → `fe-cicd-observability` (`runtimeConfig().apiBaseUrl`) → `fe-monorepo` (moved into `@acme/api-client` as `createHttpClient`, monorepo only) | `fe-auth-realtime` code + `fe-cicd-observability` change (+ `fe-monorepo` if monorepo) |
| Test handlers | `fe-repo-foundation` → `fe-auth-realtime` (signed in by default, `signedOut`) → `fe-testing` (per-feature `*.mocks.ts`, aggregated) | `fe-testing` |
| `vitest.config.ts` | `fe-repo-foundation` → `fe-testing` (unit and `vrt` projects) + `fe-design-system` (Storybook project) | `fe-testing` + `fe-design-system` |
| `reportError` | `fe-app-architecture` (console) → `fe-cicd-observability` (Sentry) | `fe-cicd-observability` |
| `main.tsx` startup order | `fe-app-architecture` → `fe-auth-realtime` → `fe-performance` → `fe-cicd-observability` (config → observability → flags → app) → `fe-a11y-i18n` (locale first) | `fe-cicd-observability`'s order with `fe-a11y-i18n`'s locale step before rendering |
| CI workflow | `fe-cicd-observability` (+ `fe-performance` steps, + `fe-monorepo` for a monorepo) | `fe-cicd-observability` |
| UI strings | `fe-app-architecture`, `fe-auth-realtime`, `fe-design-system` hard-code English for readability | `fe-a11y-i18n`: every string is a message |
| Form markup | `fe-app-architecture` inline label, input and error | `fe-design-system` `Field` and `Input` |
| Environment values | `fe-repo-foundation` `env.ts` (build-time) | `fe-cicd-observability` runtime config for per-environment values; `VITE_RELEASE` stays build-time |
| Coverage thresholds | `fe-repo-foundation` disabled → `fe-testing` policy | `fe-testing` |

## Report format

Inside the agent kit, agents return the kit's handoff (`.agents/templates/handoff.md`) and put change requests for shared files, deviations and stubs under its Notes. The seven headings below are for the standalone bootstrap build (`fe-bootstrap`).

```markdown
1. Summary — what was built (3–5 bullets)
2. PRs — title + one line each
3. Files created or changed
4. Change requests for shared files — file, exact change, why
5. Deviations from the playbook — what, why, the error that forced it
6. Stubs and TODOs left — with owner
7. Open questions for the human
```

For the full stack rationale and the original long-form guidance, read `references/playbook-overview.md`.
