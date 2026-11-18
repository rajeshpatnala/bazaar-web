# JavaScript size, INP and CLS

Bundle reduction, size-limit, yielding, deferred rendering, virtualization, workers and layout-shift rules.

## Contents

- JavaScript size
- Responsiveness (INP)
- Visual stability (CLS)

## JavaScript size

Every kilobyte of JavaScript is downloaded, parsed, compiled and executed on the user's device, often a mid-range phone. The cheapest code is code you don't ship. Split by route, load heavy features only when used, and make every new dependency justify its weight.

**Splitting strategy**

| What | How | Already in place |
| --- | --- | --- |
| Each page | `lazyRouteComponent` in the router | Yes (`fe-app-architecture`) |
| Heavy widgets inside a page (charts, rich-text editor, maps, PDF viewer) | `lazy()` + `<Suspense>`, loaded when visible or on first interaction | Add per feature |
| Rarely used flows (admin, export wizard) | Separate routes | Yes |
| Dev-only tools (devtools, mock worker) | Dynamic `import()` behind `import.meta.env.DEV` | Yes (`fe-app-architecture` and `fe-testing`) |

**Lazy-load a heavy widget**

```tsx
import { lazy, Suspense } from 'react';
import { Skeleton } from '@/shared/ui/skeleton';

// The chart library lands in its own chunk; mapping keeps the team's named-export rule.
const RevenueChart = lazy(() => import('./revenue-chart').then((m) => ({ default: m.RevenueChart })));

export function RevenuePanel() {
  return (
    <Suspense fallback={<Skeleton lines={6} />}>
      <RevenueChart />
    </Suspense>
  );
}
```

(`lazy()` needs a default export, so `revenue-chart.tsx` adds `export default RevenueChart` alongside its named export.)

**Choosing dependencies** — before adding a package, check its compressed size and tree-shakeability (bundlephobia.com or pkg-size.dev), and whether the platform already does the job:

| Instead of | Use | Saves (approx., compressed) |
| --- | --- | --- |
| `moment` | `Intl.DateTimeFormat`, `Intl.RelativeTimeFormat`, or `date-fns` (per-function imports) | ~70 KB |
| All of `lodash` | Native array/object methods, or `lodash-es` per-function imports | ~25 KB |
| `axios` | `fetch` via `http.ts` (`fe-repo-foundation`) | ~13 KB |
| `uuid` | `crypto.randomUUID()` | ~2 KB |
| A component library for one widget | Native `<dialog>`, `popover`, `<details>` (`fe-design-system`) | Varies, often 50 KB+ |

**Rules that keep bundles small**

- No barrel files that re-export a whole library; import the specific module.
- Prefer packages that publish ES modules and declare `"sideEffects": false`, so unused exports are dropped.
- Never import a server-only or Node package into client code. Lint catches most cases, and the size check catches the rest.
- Inspect the bundle whenever a dependency changes. Run a visualizer (for example `rollup-plugin-visualizer`, which works as a Vite plugin) and look for duplicates, whole libraries pulled in for one function, and locale data.

**Size budgets in CI** — `size-limit` fails the PR when the build grows past budget and comments the difference:

```bash
pnpm add -D size-limit @size-limit/file
```

```json
// .size-limit.json
[
  { "name": "Initial JS", "path": "dist/assets/index-*.js", "limit": "170 kB" },
  { "name": "Initial CSS", "path": "dist/assets/index-*.css", "limit": "30 kB" }
]
```

Add `"size": "size-limit"` to `package.json` and run it after `pnpm build` in CI (`fe-cicd-observability`). Raising a limit requires a PR comment explaining what the extra size buys. size-limit sums every file a pattern matches, so check the 100 KB per-lazy-chunk budget in the visualizer report rather than with a wildcard entry.

## Responsiveness (INP)

INP measures the time from a click, tap or keypress to the next frame the user sees. The browser can't paint while JavaScript runs, so the rule is simple: never occupy the main thread for long stretches. Respond to the input immediately, then do the heavy work in small pieces, in the background, or not at all.

**Diagnose by phase** (from the RUM attribution):

| Slow phase | Meaning | Typical fix |
| --- | --- | --- |
| Input delay | The main thread was already busy when the user clicked | Break up long tasks running at that time (hydration, analytics, timers) |
| Processing | Your event handler, or the re-render it triggers, is slow | Make the urgent update small; defer the rest with a transition; virtualize |
| Presentation delay | Rendering the next frame is expensive | Smaller DOM, `content-visibility`, avoid forced layout |

**Yield to the main thread during long work** — `src/shared/lib/yield.ts`:

```ts
/** Lets the browser handle pending input and paint before continuing. */
export function yieldToMain(): Promise<void> {
  const scheduler = (globalThis as { scheduler?: { yield?: () => Promise<void> } }).scheduler;
  if (scheduler?.yield) return scheduler.yield(); // keeps task priority where supported
  return new Promise((resolve) => setTimeout(resolve, 0));
}

/** Processes items in ~8 ms slices, so no single task gets near the 50 ms long-task mark. */
export async function processInChunks<T>(items: readonly T[], work: (item: T) => void, budgetMs = 8): Promise<void> {
  let deadline = performance.now() + budgetMs;
  for (const item of items) {
    work(item);
    if (performance.now() > deadline) {
      await yieldToMain();
      deadline = performance.now() + budgetMs;
    }
  }
}
```

**Keep input instant; let heavy results lag** — `useDeferredValue` renders the expensive list in the background, and React interrupts that render if the user keeps typing:

```tsx
import { useDeferredValue, useState } from 'react';

export function ProductSearch({ products }: { products: readonly Product[] }) {
  const [query, setQuery] = useState('');
  const deferredQuery = useDeferredValue(query);
  const results = filterProducts(products, deferredQuery); // the React Compiler memoizes this

  return (
    <>
      <input type="search" aria-label="Search products" value={query} onChange={(e) => setQuery(e.target.value)} />
      <ProductList items={results} aria-busy={query !== deferredQuery} />
    </>
  );
}
```

Use `startTransition` for the same effect when the slow update follows a click (switching tabs, applying a filter), not typing.

**Render only visible rows** — virtualize lists and tables longer than a few hundred rows:

```bash
pnpm add @tanstack/react-virtual
```

```tsx
import { useRef, type ReactNode } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';

interface VirtualListProps<T> {
  items: readonly T[];
  rowHeight: number;
  renderRow: (item: T) => ReactNode;
}

export function VirtualList<T>({ items, rowHeight, renderRow }: VirtualListProps<T>) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => rowHeight,
    overscan: 8, // render a few rows beyond the viewport for smooth scrolling
  });

  return (
    <div ref={scrollRef} style={{ blockSize: '60vh', overflow: 'auto' }}>
      <div style={{ blockSize: virtualizer.getTotalSize(), position: 'relative' }}>
        {virtualizer.getVirtualItems().map((row) => {
          const item = items[row.index];
          if (item === undefined) return null;
          return (
            <div key={row.key} style={{ position: 'absolute', inset: '0 0 auto 0', transform: `translateY(${row.start}px)` }}>
              {renderRow(item)}
            </div>
          );
        })}
      </div>
    </div>
  );
}
```

If rows render stale with the React Compiler enabled, add a `"use no memo"` directive to this one component; some table and virtualization libraries return mutable objects the compiler can't track. Note too that virtualized content isn't found by the browser's find-in-page, so prefer pagination for lists users search visually.

**Move heavy computation off the main thread** — parsing a large CSV, aggregating reports, or diffing documents belongs in a Web Worker. Vite bundles workers natively:

```ts
const worker = new Worker(new URL('./aggregate.worker.ts', import.meta.url), { type: 'module' });
worker.postMessage(rows);
worker.onmessage = (event: MessageEvent<Summary>) => setSummary(event.data);
```

For more than one or two message types, wrap the worker with Comlink, so calling it looks like calling an async function.

**Cheap wins**

- Debounce search-as-you-type requests (300 ms, `fe-app-architecture`) and throttle scroll and resize work to animation frames.
- Never read layout (`offsetHeight`, `getBoundingClientRect`) right after writing styles in a loop; batch reads, then writes.
- Give long, off-screen sections `content-visibility: auto; contain-intrinsic-size: auto 600px;` so the browser skips rendering them until they're scrolled near.
- Show feedback within the same frame as the click (pressed state, spinner), even when the result takes longer.

## Visual stability (CLS)

Layout shift happens when something appears or changes size after the content around it has rendered: an image without dimensions, a late banner, a font swap, a skeleton smaller than the real content. The cure is always the same: **know the final size before the content arrives.**

| Cause | Fix |
| --- | --- |
| Images and video without dimensions | Always set `width` and `height` (or CSS `aspect-ratio`); the browser reserves the box before loading |
| Skeletons smaller than the real content | Size skeletons to match the loaded layout: same row count and heights |
| Banners, cookie notices or ads inserted above content | Reserve their space up front, or show them as overlays (`position: fixed`, `<dialog>`, `popover`) that don't push content |
| Web font swap changes text size | Metric-matched fallback font (`size-adjust`, see Loading) |
| Content added above the reading position (new items, "load more" at the top) | Append below, or show a "3 new items" button the user clicks |
| Animating `top`, `height` or `margin` | Animate `transform` and `opacity` only; they never trigger layout |
| Embeds (video, maps, social posts) | A fixed-size container, or a lightweight placeholder that loads the real embed on click |

**Reserve space for late content**

```css
.adSlot {
  min-block-size: 250px; /* the tallest ad size you serve */
  contain: layout; /* its contents can't resize the page around it */
}

.videoEmbed {
  aspect-ratio: 16 / 9;
  inline-size: 100%;
}
```

Shifts within 500 ms of a user's own click or keypress don't count towards CLS, so expanding an accordion is fine. Unprompted movement is what's penalized. To debug, enable **Layout Shift Regions** in Chrome DevTools' Rendering panel, and check the `largestShift` element the RUM attribution reports.
