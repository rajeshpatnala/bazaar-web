# Architecture, scalability, maintainability lens

Question for every finding: **will this still be right in 5-20 years with 10x pages, products and editors?**

## 1. Hardcoding hunt (grep, then classify)
Search for: prices/currency symbols, `₹|\$|€` literals, phone/email/address/social URLs, company name, tax/shipping rates, thresholds (free-shipping amount), API base URLs, feature flags, copy repeated across pages, hex colors/px magic numbers repeated, `localhost`, `DUMMY|TODO|FIXME|lorem`, IDs/SKUs inside markup or logic.
Classify each: **data** (move to JSON/CMS/API), **config** (one `config.js`/env per environment), **design token** (CSS custom property), **copy** (content file / i18n), **legitimate constant**.
Target: adding the 50th product, a new size, a new currency, a new language or a rebrand = data/token edit, zero code change.

## 2. Single source of truth
- Duplicate components (nav, footer, cards, buttons, head tags) copied per page = drift. Consolidate into shared CSS/partials/web components/templates. Without a build step: native `<template>` + small JS include, Web Components, or a tiny static generator (11ty/Astro) is better than copy-paste. Recommend the least tooling that removes duplication.
- Tokens in `:root` (color, space, type scale, radius, shadow, z-index scale, breakpoints as container/`clamp()` logic). No repeated magic numbers; no `!important` outside deliberate override layers.
- One module owns each concern: money formatting, API client, cart store, analytics, storage access, DOM helpers.

## 3. Layers and seams (know the changes coming)
```
UI components (dumb, no fetch/storage)
  → state/store (cart, session, UI state; subscribe/notify)
    → services (api client, payments adapter, analytics)  ← swap Shopify/custom/mock here
      → config + schema (validated at startup)
```
- Define an interface per replaceable backend (`CartService`, `CatalogService`, `CheckoutService`) with a mock implementation and a real one; UI imports only the interface. Demo → production becomes one adapter swap.
- Money: integers in minor units + currency code, formatted with `Intl.NumberFormat`. Never floats.
- Dates/locale via `Intl`; copy keyed for i18n even if single-language today.
- Validate every external payload at the boundary (schema/zod/hand-rolled), not deep inside UI.

## 4. State and data
- Server is source of truth for money/stock/orders. Client state is a cache. Version stored data (`{v:2,...}`) and migrate; stale `localStorage` from old releases breaks users silently.
- Avoid globals; use ES modules, `type="module"`, explicit exports. Event delegation over per-node listeners. Passive scroll listeners, `IntersectionObserver` over scroll math, no layout thrash (batch reads/writes).

## 5. Scalability of the codebase (people + size)
- Folder by feature/domain, not by file type, once >~15 files. Predictable names; one export per file for components.
- Lint + format (ESLint, Prettier/Biome, stylelint), `.editorconfig`, pre-commit, CI running lint + tests + build + audit. Types via TypeScript or JSDoc `// @ts-check` (cheap, durable).
- Tests: unit for pure logic (money, cart, validation), e2e smoke (Playwright) for critical flows: browse → add → cart → checkout. Visual regression for shared components.
- Docs that rot slowest: README (run/build/deploy), ADRs/`plans/` for *why*, CONTRIBUTING, a component catalogue.
- Dependency policy: prefer platform features (CSS grid, `dialog`, `popover`, container queries, `:has()`, View Transitions, `fetch`, `URL`, `Intl`) over libraries. Add a dependency only if it clearly pays for maintenance; pin, audit, and know its exit cost.

## 6. Performance as architecture
Budgets (e.g. LCP < 2.5s, CLS < 0.1, INP < 200ms, JS < 100KB gz on content pages). Critical CSS minimal; defer non-critical JS; responsive images (`srcset`/`sizes`, AVIF/WebP, explicit `width`/`height`); `loading=lazy` below fold, `fetchpriority` for LCP; font `display=swap` + subset + self-host; HTTP caching with hashed filenames; no render-blocking third parties.

## 7. Refactor method (safe, incremental)
1. Characterize: list pages, flows, shared pieces; take baseline screenshots / write smoke tests.
2. Strangle, don't rewrite: extract one shared piece at a time behind the same markup, verify on all pages, delete the copies.
3. Mechanical changes via script + grep verification; diff the selector/function sets before and after any slice-and-remove so nothing disappears silently.
4. Small reviewable commits; each leaves the site working.
5. Record decisions (what, why, rejected alternative) in `plans/` or ADRs.
