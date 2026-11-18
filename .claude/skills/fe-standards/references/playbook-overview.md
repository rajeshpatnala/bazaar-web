# Playbook overview — rationale and cross-cutting baselines

Read when you need the *why* behind a stack choice or a baseline that spans several playbooks.

## Contents
1. Why this stack
2. State homes
3. Security baseline
4. Performance targets
5. Test layers at a glance
6. Git and review at a glance

## 1. Why this stack

- **React + TypeScript + Vite** for app-style products; **Next.js** only where public pages need SSR/SEO. Angular is equally valid for teams that know it — the principles still apply.
- **TanStack Query** owns server state (cache, retries, dedupe, background refresh), so no hand-written loading/error stores.
- **TanStack Router** gives compile-time checked paths, params and search params.
- **CSS Modules + tokens** cost zero runtime and stay native; Tailwind v4 is acceptable if chosen instead (then tokens live in its theme and arbitrary values are banned).
- **Oxlint before ESLint**: 50–100x faster first pass; ESLint keeps type-aware and architecture rules.
- **Vitest + Testing Library + MSW + Playwright**: one runner for unit/component, real network mocking, real browsers for journeys.
- **pnpm**: strict `node_modules` (undeclared imports fail), catalogs, supply-chain cooldown on by default.

## 2. State homes

| Kind of state | Home | Examples |
|---|---|---|
| Server data | TanStack Query cache | orders, profile, results |
| Shareable view state | URL search params | filters, sort, page, tab |
| Form state | React Hook Form | values, dirty, errors |
| Local UI | `useState` | open/closed, hover |
| Global client | Zustand (rare) | theme, sidebar, wizard draft |

Never copy server data into a global store; never sync derived values with `useEffect`.

## 3. Security baseline

- No secrets in the client bundle (everything `VITE_`/`NEXT_PUBLIC_` is public).
- Sessions in `HttpOnly; Secure; SameSite` cookies, never `localStorage`; CSRF token on writes.
- `dangerouslySetInnerHTML` banned except via the sanitized `SafeHtml` component (DOMPurify); Trusted Types where supported.
- Headers at the CDN:
  ```text
  Content-Security-Policy: default-src 'self'; script-src 'self' 'nonce-{random}' 'strict-dynamic';
    object-src 'none'; base-uri 'none'; frame-ancestors 'none'; require-trusted-types-for 'script'
  Strict-Transport-Security: max-age=63072000; includeSubDomains; preload
  X-Content-Type-Options: nosniff
  Referrer-Policy: strict-origin-when-cross-origin
  Permissions-Policy: camera=(), microphone=(), geolocation=()
  ```
- Lockfile committed, `--frozen-lockfile` in CI, Renovate weekly, Actions pinned to SHAs, secret scanning on.

## 4. Performance targets

Core Web Vitals at p75 field data, mobile and desktop separately: **LCP ≤ 2.5 s, INP ≤ 200 ms, CLS ≤ 0.1**. Team budgets: initial JS ≤ 170 KB compressed, initial CSS ≤ 30 KB, Lighthouse performance ≥ 90 on key pages. Details in `fe-performance`.

## 5. Test layers at a glance

Static checks → unit (pure logic) → component/integration (most tests; real components, cache and router, network mocked) → visual regression (design system only) → accessibility (axe) → E2E (5–15 critical journeys) → contract (API types). Details in `fe-testing`.

## 6. Git and review at a glance

Trunk-based, branches live 1–2 days, Conventional Commits, squash merge through a merge queue, PRs under ~400 lines, first review within one business day, comments labelled `issue:/suggestion:/question:/nit:/praise:`, CODEOWNERS routes reviews to owning teams. Details in `fe-team-workflow`.
