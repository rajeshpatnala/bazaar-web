---
name: fe-bootstrap
description: Plan for building a new production React + TypeScript front end from an empty repo with the agent team - inputs to collect, ADR 0001, the phase order (foundation, design system and architecture, auth/a11y/performance, testing in depth, integration), which kit agent builds what, shared-file ownership and the final integration checklist. Use when the request is a new app or repo from scratch, not a change to an existing one.
---

# Bootstrapping a new front end

The tech lead turns this into the first plans (`.agents/plans/F0-*.md`); the manager launches the agents. Every rule of the kit still applies: work branches off the integration branch, PRs against it, reviews on the PR, `just merge-check` before `just merge`, unit tests in every PR, mocks only.

## 1. Inputs (manager collects; defaults go into `docs/adr/0001-initial-decisions.md`)

| Input | Why | Default if missing |
|---|---|---|
| Product name and first 2-3 features | Real features replace the playbooks' `orders` example | Build `orders` from `fe-app-architecture` |
| Backend API spec (OpenAPI file or URL) | Generated types, Zod schemas, contract checks | Mock every endpoint with MSW and build to that contract |
| Single app or several apps | Decides monorepo first (`fe-monorepo`) | Single Vite SPA |
| Auth model | `fe-auth-realtime` assumes HttpOnly cookie sessions with refresh | That contract, mocked with MSW |
| Hosting and CDN | Deploy scripts are host-specific | Static hosting behind a CDN; deploy scripts as documented stubs |
| Error tracking, flags, analytics vendors | Observability wiring | Sentry wired but off without a DSN; OpenFeature in-memory provider |
| Languages | Locale list | `en`, with pseudo-locales `en-XA`, `en-XB` in development |
| GitHub org and team names | CODEOWNERS, rulesets | Placeholder `@acme/*` teams, listed in the ADR |

## 2. Phases (each starts when the previous one is merged into the integration branch and `just gate` is green)

```text
Phase 0  manager + architect   inputs, ADR 0001, project.md, `just setup-branches` (user approves)
Phase 1  dev-app               foundation: package.json, toolchain, tsconfig, Vite, lint, env.ts, http.ts,
                               Vitest + MSW + Playwright setup          (skill fe-repo-foundation)
         (dev-app first runs fe-monorepo's workspace layout if the user chose several apps)
Phase 2  dev-ui                design system: tokens, global CSS, core components, Storybook   ─┐
         dev-app               app architecture: providers, router, first feature, pages, forms  │ parallel
         cicd                  CI workflow, rulesets checklist (fe-cicd-observability ci.md)      │
         dev-platform          runtime config, flags, observability, deploy scripts              │
         dev-app (small slice) team files: CODEOWNERS, PR template, CONTRIBUTING, AGENTS.md,     │
                               ADR/RFC/runbook templates (fe-team-workflow)                      ─┘
Phase 3  dev-auth              auth & real-time                                                 ─┐
         dev-a11y              accessibility & i18n                                              │ parallel
         dev-perf              performance budgets, RUM, lazy loading                           ─┘
Phase 4  tester                testing in depth: final Vitest projects, visual tests, per-feature mocks,
                               Playwright fixtures, a11y scan, contract checks (fe-testing)
Phase 5  tech-lead             integration checklist (section 4), final report via the manager
```

Why this order: everything depends on the foundation; a monorepo must exist before configs are filled; Phase 2 areas touch separate files; Phase 3 extends what Phase 2 created; testing in depth runs across code that exists (every earlier agent still writes its own unit tests).

Slices follow `max_slice_changed_lines`: a phase item that is larger becomes several stacked PRs.

## 3. Shared files (one owner per phase; others send change requests through the tech lead)

| File | Owner | Others contribute |
|---|---|---|
| `package.json` scripts and dependencies | dev-app (Phase 1), then the tech lead plans changes | every agent lists dependencies and scripts it needs in its handoff |
| `src/main.tsx` | dev-app | dev-auth (auth sync), dev-perf (RUM), dev-platform (config, observability, flags), dev-a11y (locale, IntlProvider) |
| `src/app/providers.tsx` | dev-app | dev-platform (OpenFeatureProvider), dev-a11y (IntlProvider, AnnouncerProvider) |
| `src/app/router.tsx` | dev-app | dev-auth (authenticated and login routes) |
| `src/shared/lib/http.ts` | dev-app -> dev-auth (final) | dev-platform (base URL from runtime config) |
| `src/shared/lib/errors.ts` | dev-app | dev-platform (Sentry in `reportError`) |
| `eslint.config.js`, `vite.config.ts` | dev-app | dev-ui (`no-autofocus`), dev-a11y (formatjs), dev-platform (Sentry plugin, version file) |
| `vitest.config.ts`, `src/test/*` | dev-app -> tester (final) | dev-auth (realtime mock), dev-ui (Storybook project) |
| `.github/workflows/*` | cicd | dev-perf (size, Lighthouse steps), monorepo affected runs |
| `.github/CODEOWNERS` | dev-app (team-files slice) | package owners |

In Phase 3, merge changes to `main.tsx` and `providers.tsx` one after another in this order: dev-auth, dev-platform's remaining items, dev-perf, dev-a11y, rebasing between merges. `fe-standards` has the precedence table for files several playbooks write.

## 4. Integration checklist (Phase 5)

- [ ] Clean clone: `pnpm install && just gate` passes (monorepo: `pnpm turbo run typecheck lint test build`).
- [ ] Visual tests, Storybook tests and `pnpm e2e` (including the a11y spec) pass - in CI, or locally if browsers are installed.
- [ ] `pnpm size` and Lighthouse CI are within budget.
- [ ] Sign in, use the first feature, sign out, expire the session, sign in again (lands on the original page), receive a real-time update, switch to `en-XB`: every step works (against MSW or a test backend, never production).
- [ ] Keyboard-only and screen-reader smoke tests on the main journey (`fe-a11y-i18n` manual script), results in the PR.
- [ ] No `TODO` left that isn't listed in the final report.
- [ ] `README.md`, `CONTRIBUTING.md`, `AGENTS.md` and `.agents/project.md` describe the repo as built; `just codemap` is current.

Final report to the user (manager): what was built with PR links, decisions with ADR links, deviations from the playbooks and why, stubs needing human action (hosting, vendor keys, GitHub settings, seeding endpoint), next steps, budget used.
