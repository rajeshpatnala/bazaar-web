# Targets, measuring and loading

Budgets, RUM code, profiling workflow and LCP fixes.

## Contents

- Targets and budgets
- Measuring
- Loading (LCP)

## Targets and budgets

The user-facing targets are Google's [Core Web Vitals](https://web.dev/articles/vitals): a page passes when the 75th percentile of real visits meets all three. The other budgets are team defaults that keep those targets reachable; tune them per product, but write the numbers down and enforce them.

**Core Web Vitals (field data, p75, mobile and desktop separately)**

| Metric | Measures | Good | Poor |
| --- | --- | --- | --- |
| LCP — Largest Contentful Paint | Loading: when the main content appears | ≤ 2.5 s | > 4.0 s |
| INP — Interaction to Next Paint | Responsiveness: delay from click/tap/key to visible response | ≤ 200 ms | > 500 ms |
| CLS — Cumulative Layout Shift | Visual stability: unexpected movement of content | ≤ 0.1 | > 0.25 |

These thresholds have been stable since INP replaced FID in 2024, and Google commits to changing them at most yearly with advance notice. Treat articles claiming new thresholds with suspicion and check web.dev.

**Team budgets (lab and build)**

| Budget | Default | Enforced by |
| --- | --- | --- |
| JavaScript for the first route (compressed) | ≤ 170 KB | `size-limit` in CI |
| CSS for the first route (compressed) | ≤ 30 KB | `size-limit` in CI |
| Any single lazy chunk (compressed) | ≤ 100 KB | Bundle visualizer, checked in review |
| Hero / LCP image | ≤ 150 KB, AVIF or WebP | Review checklist |
| Web fonts | ≤ 2 files, WOFF2, subset | Review checklist |
| Lighthouse performance score (mobile preset) | ≥ 90 on key pages | Lighthouse CI on the PR preview |
| Total Blocking Time (lab proxy for INP) | ≤ 200 ms | Lighthouse CI |
| Time to First Byte (field, p75) | ≤ 800 ms | RUM dashboard |

Why 170 KB: on a mid-range phone, a typical JavaScript bundle takes roughly as long to parse and execute as to download. Staying near this size keeps a 3–4 second interactive time achievable on 4G. Authenticated dashboards may trade some first-load size for richer features, but the budget still applies to the sign-in and landing routes.

## Measuring

Real-user monitoring (RUM) tells you **whether** the app is slow and for whom. Attribution data tells you **why**. Lab tools (Lighthouse, DevTools) let you reproduce and fix it. Use all three in that order. Never declare victory from a lab score alone.

```bash
pnpm add web-vitals
```

**`src/app/rum.ts`** — Core Web Vitals with the details needed to act on them:

```ts
import {
  onCLS,
  onINP,
  onLCP,
  type CLSMetricWithAttribution,
  type INPMetricWithAttribution,
  type LCPMetricWithAttribution,
} from 'web-vitals/attribution';
import { env } from '@/shared/config/env';
import type { AppRouter } from './router';

type Vital = CLSMetricWithAttribution | INPMetricWithAttribution | LCPMetricWithAttribution;

const SAMPLE_RATE = 1; // lower (e.g. 0.1) once traffic is high

/** The "why": which element, and which phase was slow. */
function attribution(metric: Vital): Record<string, unknown> {
  switch (metric.name) {
    case 'LCP': {
      const a = metric.attribution;
      return {
        target: a.target,
        resource: a.url,
        ttfb: a.timeToFirstByte,
        loadDelay: a.resourceLoadDelay,
        loadDuration: a.resourceLoadDuration,
        renderDelay: a.elementRenderDelay,
      };
    }
    case 'INP': {
      const a = metric.attribution;
      return {
        target: a.interactionTarget,
        type: a.interactionType,
        inputDelay: a.inputDelay,
        processing: a.processingDuration,
        presentation: a.presentationDelay,
      };
    }
    case 'CLS': {
      const a = metric.attribution;
      return { target: a.largestShiftTarget, largestShift: a.largestShiftValue };
    }
  }
}

export function startRum(router: AppRouter): void {
  if (Math.random() > SAMPLE_RATE) return;
  const currentRoute = () => router.state.matches.at(-1)?.routeId ?? 'unknown'; // '/orders/$orderId', never real ids
  const landingRoute = currentRoute();

  const send = (metric: Vital) => {
    const body = JSON.stringify({
      name: metric.name,
      value: metric.value,
      rating: metric.rating,
      id: metric.id,
      landingRoute, // LCP and CLS belong to the page the visit started on
      route: currentRoute(), // where the visit was when the value was reported
      release: env.VITE_RELEASE,
      ...attribution(metric),
    });
    if (!navigator.sendBeacon('/api/rum', body)) {
      void fetch('/api/rum', { method: 'POST', body, keepalive: true });
    }
  };

  onLCP(send);
  onINP(send);
  onCLS(send);
}
```

Call `startRum(router)` in `main.tsx` after the router is created. `fe-cicd-observability` sends the same data to your observability tool and builds the dashboards and alerts.

**Reading the data**

- Slice by **route** and **device type**; an average across the whole app hides the one slow page.
- For LCP, the four phase timings show where time goes (next section). For INP, a large `inputDelay` means the main thread was busy before the click, while large `processing` means your event handler or the re-render it triggers is slow.
- In a single-page app, LCP and CLS describe the page the visit landed on; INP covers the whole visit. That's why both routes are recorded.

**Profiling workflow** — when a metric is red:

1. **Reproduce on a slow device profile.** In Chrome DevTools, use 4x CPU slowdown and a mobile network preset, or a real mid-range Android phone.
2. **Record in the Performance panel.** Look for long tasks (red corners), the **Interactions** track for slow INP, and the LCP marker with its phase breakdown.
3. **Read React's Performance Tracks** (React 19.2+) in the same timeline to see which components rendered, and why, during the slow moment. React DevTools' Profiler adds render counts.
4. **Change one thing, re-record, compare.**
5. **Confirm in the field** a few days after release — the RUM p75 is the only number that counts.

## Loading (LCP)

Every LCP splits into four phases with no gaps between them. On a well-optimized page, almost all the time goes to the two network phases, and the two delays are close to zero ([web.dev](https://web.dev/articles/optimize-lcp)). Use the RUM attribution from the previous section to see which phase is slow before changing anything.

*Diagram: LCP breakdown · four phases and the fix for each*

Shrinking an image doesn't help if the element then waits for JavaScript to render: the saved time just moves into render delay. Fix the delays first.

**`index.html` head** — let the browser discover critical resources immediately:

```html
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Orders · Acme</title>
  <!-- Only for critical cross-origin hosts (CDN, API); each costs a connection. -->
  <link rel="preconnect" href="https://cdn.example.com" crossorigin />
  <!-- The one font used above the fold. -->
  <link rel="preload" href="/fonts/inter-var-latin.woff2" as="font" type="font/woff2" crossorigin />
  <!-- No synchronous <script> tags: Vite injects a module script, which is deferred. -->
</head>
```

**Images**

```html
<!-- The LCP image: discoverable in HTML, high priority, never lazy, sized to prevent layout shift. -->
<img
  src="/img/hero-1200.avif"
  srcset="/img/hero-600.avif 600w, /img/hero-1200.avif 1200w, /img/hero-2000.avif 2000w"
  sizes="(min-width: 64rem) 50vw, 100vw"
  width="1200" height="800"
  fetchpriority="high"
  alt="Team reviewing an order dashboard"
/>

<!-- Everything below the fold: native lazy loading. -->
<img src="/img/feature-800.webp" width="800" height="600" loading="lazy" decoding="async" alt="…" />
```

- Serve AVIF or WebP at several widths, generated at build time or by an image CDN on your own domain (a third-party domain costs an extra connection).
- Use `fetchpriority="high"` on one or two images at most; marking everything high helps nothing.
- In a client-rendered app, the LCP element doesn't exist until JavaScript runs. For public, content-heavy pages, server rendering (Next.js) removes that delay entirely.

**Fonts** — self-hosted WOFF2, subset to the scripts you need, with a fallback whose metrics match, so the swap doesn't shift layout:

```css
@font-face {
  font-family: 'Inter';
  src: url('/fonts/inter-var-latin.woff2') format('woff2');
  font-weight: 100 900;
  font-display: swap; /* text is visible immediately in the fallback */
}
@font-face {
  font-family: 'Inter Fallback';
  src: local('Arial');
  size-adjust: 107%; /* compute these per font with a tool such as Fontaine or Capsize */
  ascent-override: 90%;
  descent-override: 22%;
  line-gap-override: 0%;
}
:root {
  --font-family-sans: 'Inter', 'Inter Fallback', system-ui, sans-serif;
}
```

**Caching headers** — set at the CDN:

| Resource | `Cache-Control` | Why |
| --- | --- | --- |
| `index.html` | `no-cache` | Revalidated every visit, so a deploy is live immediately |
| `/assets/*` (hashed JS, CSS, fonts, images) | `public, max-age=31536000, immutable` | The file name changes when content changes, so cache forever |
| `config.json` (runtime config) | `no-cache` | Must reflect the current environment |
| API `GET` responses | `private, no-cache` with `ETag` | Revalidation returns a tiny `304` when nothing changed |

Also enable Brotli compression and HTTP/2 or HTTP/3 at the CDN, and avoid redirect chains on entry URLs. Each redirect adds a full round trip before the first byte.
