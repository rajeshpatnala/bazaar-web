---
name: fe-performance
description: "Makes and keeps the front end fast: Core Web Vitals targets and JS/CSS budgets, real-user monitoring with web-vitals attribution, LCP loading (preload, fetchpriority, fonts, caching), bundle size (lazy loading, dependency swaps, size-limit), INP responsiveness (yielding, useDeferredValue, virtualization, workers), CLS, data and third-party scripts, and Lighthouse CI enforcement. Use whenever a page is slow, bundle size grows, adding a dependency or third-party script, touching index.html, images or fonts, or reviewing performance-sensitive code."
---

# Performance

Sets the budgets, measures real users and fixes LCP, INP and CLS. Load `fe-standards` alongside it.

## Best practices

Binding rules; the reference files show how to apply them.

**Measure before optimizing**

1. **Judge performance by real users at the 75th percentile, on mobile and desktop separately** — lab scores on a fast laptop hide what most users experience.
2. **Collect attribution, not just numbers** — knowing which element was the LCP, or which click was slow, turns a red dashboard into a ticket.
3. **Profile before changing code** — guessed optimizations add complexity and usually miss the real bottleneck.

**Loading**

4. **Make the largest element load first** — preload or prioritize the hero image or font, never lazy-load it, and keep it out of client-only JavaScript.
5. **Ship less JavaScript** — split by route, load heavy features on demand, and treat every dependency as a cost someone must justify.
6. **Cache aggressively and safely** — hashed assets cached for a year, HTML never cached, so deploys take effect instantly and repeat visits are free.
7. **Size every image and self-host fonts** — modern formats, responsive sizes, explicit dimensions, and a metric-matched fallback font.

**Responsiveness**

8. **Never block the main thread for more than about 50 ms** — split long work, yield between chunks, and move heavy computation to a Web Worker.
9. **Keep typing and clicking instant; let results catch up** — mark heavy re-renders as non-urgent with transitions.
10. **Render only what's on screen** — virtualize lists beyond a few hundred rows.

**Stability**

11. **Reserve space for everything that loads later** — images, ads, embeds and skeletons get their final size up front, so nothing jumps.

**Data and third parties**

12. **Fetch early and in parallel, never in waterfalls** — route loaders and prefetch-on-hover make most navigations instant.
13. **Third-party scripts load last** — after the page is usable, behind consent, and each one has an owner who can justify its cost.

**Keeping it fast**

14. **Budgets are enforced in CI** — a PR that pushes JavaScript size or lab metrics over budget fails like a failing test.
15. **Regressions in real-user metrics alert the owning team** — speed erodes one small change at a time unless someone is watching.

## Workflow

1. Adopt the targets and budgets → `references/measure-and-load.md` (Targets and budgets).
2. Add `rum.ts` (web-vitals attribution, sendBeacon) and follow the profiling workflow → `measure-and-load.md` (Measuring).
3. Fix loading: `index.html` head, LCP image priority, fonts with metric fallbacks, caching headers → `measure-and-load.md` (Loading).
4. Cut JavaScript: lazy widgets, dependency swaps, `.size-limit.json` → `references/runtime.md` (JavaScript size).
5. Fix responsiveness and stability: `yield.ts`, deferred values, virtual lists, workers, CLS rules → `runtime.md`.
6. Data and third-party rules, `loadScriptWhenIdle` → `references/third-party-and-enforcement.md`.
7. Lighthouse CI, CI steps, RUM alerts, PR checklist → `third-party-and-enforcement.md`. Send CI steps to the `fe-cicd-observability` owner.

## Files this playbook owns (Build mode)

`src/app/rum.ts`, `src/shared/lib/yield.ts`, `src/shared/ui/virtual-list.tsx`, `src/shared/lib/load-script.ts`, `.size-limit.json`, `lighthouserc.json`, performance-related parts of `index.html` (preloads, fonts). CI steps are change requests to `fe-cicd-observability`.

## Done when

Initial JS ≤ 170 KB and CSS ≤ 30 KB compressed (size-limit passes); no lazy chunk over 100 KB in the visualizer; Lighthouse CI passes on key pages; RUM reports LCP, INP and CLS with attribution per route; field p75 is within LCP ≤ 2.5 s, INP ≤ 200 ms, CLS ≤ 0.1, or a ticket exists for each miss.

## Reference files

Read only the file the current step needs. Paths are relative to this skill's folder (`.claude/skills/<skill>/`).

- `references/measure-and-load.md` — Budgets, RUM code, profiling workflow and LCP fixes.
- `references/runtime.md` — Bundle reduction, size-limit, yielding, deferred rendering, virtualization, workers and layout-shift rules.
- `references/third-party-and-enforcement.md` — Data-fetching and third-party script rules, Lighthouse CI, CI steps, alerts and the performance PR checklist.

## Sources

- [Web Vitals — web.dev](https://web.dev/articles/vitals)
- [Optimize Largest Contentful Paint — web.dev](https://web.dev/articles/optimize-lcp)
