# Data, third parties and enforcement

Data-fetching and third-party script rules, Lighthouse CI, CI steps, alerts and the performance PR checklist.

## Contents

- Data and third-party scripts
- Enforcing budgets
- Performance PR checklist

## Data and third-party scripts

The fastest request is the one already made. `fe-app-architecture` and `fe-auth-realtime` already have the data layer start requests early (route loaders), run them in parallel, prefetch on hover and cache results. What remains is keeping payloads small and keeping other companies' code from slowing yours.

**Data**

- **No waterfalls.** A component that fetches, renders a child, which then fetches again, serializes round trips. Start all of a page's requests in its loader with `Promise.all` or parallel `ensureQueryData` calls.
- **Tune `staleTime` per query.** Reference data (countries, plans) can be `Infinity`; dashboards 30–60 s; live prices use real-time updates (`fe-auth-realtime`) instead of polling.
- **Paginate on the server.** Never download thousands of rows to filter them in the browser.
- **Ask the API for what the screen needs.** A list endpoint returns list fields, not full objects with nested history. Agree on lean list and detail shapes with the backend.
- **Prefetch the likely next step.** `defaultPreload: 'intent'` (`fe-app-architecture`) already loads a page's code and data on hover or focus, so most clicks feel instant.

**Parallel loader example**

```ts
loader: ({ context, params }) =>
  Promise.all([
    context.queryClient.ensureQueryData(orderDetailQuery(params.orderId)),
    context.queryClient.ensureQueryData(customerQuery(params.orderId)), // runs at the same time, not after
  ]),
```

**Third-party scripts** — analytics, chat widgets, A/B testing, tag managers and embeds are a leading cause of slow INP and LCP, and the team doesn't control their code.

| Rule | Why |
| --- | --- |
| Every third-party script has a named owner and a written reason | Scripts nobody owns are never removed |
| Load after the page is usable (`load` event) or on first interaction, never in `<head>` | Keeps them out of LCP and early input delay |
| Load only after consent where required | Privacy law, and it means fewer scripts for many users |
| Replace heavy embeds with a facade (a static preview that loads the real player or map on click) | A video embed can cost hundreds of kilobytes before anyone presses play |
| Avoid A/B tools that hide the page until they decide | They add their whole load time to LCP; prefer server-side or edge experiments |
| Review all third parties quarterly against RUM data | Their cost grows silently as vendors ship updates |

**Load a script after the page is ready**

```ts
export function loadScriptWhenIdle(src: string): void {
  const inject = () => {
    const script = document.createElement('script');
    script.src = src;
    script.async = true;
    document.body.append(script);
  };
  if (document.readyState === 'complete') setTimeout(inject, 0);
  else window.addEventListener('load', () => setTimeout(inject, 0), { once: true });
}
```

Add every allowed third-party origin to the Content Security Policy (`fe-security`), so a compromised tag manager can't load arbitrary scripts.

## Enforcing budgets

Three automatic checks catch regressions at different stages. `size-limit` catches bundle growth in seconds on every PR. Lighthouse CI catches loading and blocking-time regressions on the PR's build. Real-user alerts catch whatever slips through, within days of a release.

**`lighthouserc.json`** — median of three runs on key public pages, mobile emulation (the default):

```json
{
  "ci": {
    "collect": {
      "url": ["http://localhost:4173/", "http://localhost:4173/login"],
      "startServerCommand": "pnpm preview --port 4173",
      "numberOfRuns": 3
    },
    "assert": {
      "assertions": {
        "categories:performance": ["error", { "minScore": 0.9 }],
        "categories:accessibility": ["error", { "minScore": 0.95 }],
        "largest-contentful-paint": ["error", { "maxNumericValue": 2500 }],
        "total-blocking-time": ["error", { "maxNumericValue": 200 }],
        "cumulative-layout-shift": ["error", { "maxNumericValue": 0.1 }]
      }
    },
    "upload": { "target": "filesystem", "outputDir": ".lighthouseci" }
  }
}
```

Use the `filesystem` target and upload the folder as a CI artifact; the public temporary-storage option would publish reports of a private app. For signed-in pages, add a `puppeteerScript` that signs in with the E2E test account first.

**CI steps** (added to the `verify` job in `fe-cicd-observability`):

```yaml
- run: pnpm build
- run: pnpm size # size-limit: fails if over budget
- run: pnpm dlx @lhci/cli autorun # Lighthouse CI against the built app
```

**Real-user alerts** (set up in `fe-cicd-observability`'s observability tool):

| Alert | Condition | Goes to |
| --- | --- | --- |
| Core Web Vital regression | p75 for a route crosses "good" for 2 consecutive days | Team owning the route (CODEOWNERS) |
| Release regression | p75 LCP or INP up more than 10% versus the previous release | The release channel, with the release id |
| Third-party slowdown | INP attribution points to a third-party script for over 5% of slow interactions | The script's owner |

## Performance PR checklist

- [ ] No new dependency without a size check and a reason; heavy libraries are lazy-loaded.
- [ ] `size-limit` and Lighthouse CI pass, or the PR explains why a budget changes.
- [ ] Images have `width`/`height`, use AVIF or WebP, and only the LCP image has `fetchpriority="high"` (never `loading="lazy"`).
- [ ] New lists over a few hundred rows are paginated or virtualized.
- [ ] Event handlers do the minimum synchronously; heavy updates use a transition, chunking or a worker.
- [ ] Content that loads late has reserved space; no layout shift on load.
- [ ] New pages fetch in the route loader, in parallel, with an appropriate `staleTime`.
- [ ] Any new third-party script has an owner, loads after the page is usable, and is in the CSP.
