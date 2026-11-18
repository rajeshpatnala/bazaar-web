# Feature flags, observability and releases

Flag registry and provider, Sentry setup, dashboards, SLO alerts, runbook template and release checklist.

## Contents

- Feature flags
- Observability
- Release checklist

## Feature flags

Flags separate **deploying** code from **releasing** a feature. Unfinished work merges daily behind an off flag, features roll out to a percentage of users, and a misbehaving feature is switched off in seconds without a deploy. Use the vendor-neutral OpenFeature API, so changing flag vendors (LaunchDarkly, Unleash, GrowthBook, Flagsmith) touches one file.

```bash
pnpm add @openfeature/react-sdk
# plus your vendor's OpenFeature web provider
```

**`src/shared/config/flags.ts`** — the registry: every flag, its owner, and when it must be removed:

```ts
export const FLAGS = {
  'checkout.new-flow': { owner: '@acme/payments-web', removeBy: '2026-12-15', default: false },
  'orders.bulk-cancel': { owner: '@acme/orders-web', removeBy: '2027-01-31', default: false },
  'ops.realtime-kill-switch': { owner: '@acme/web-platform', removeBy: 'never', default: false }, // permanent kill switch
} as const;

export type FlagKey = keyof typeof FLAGS;
```

**`src/app/flags.ts`**

```ts
import { InMemoryProvider, OpenFeature } from '@openfeature/react-sdk';
import { FLAGS } from '@/shared/config/flags';

/** Local and test fallback: every flag at its registered default. */
function defaultsProvider(): InMemoryProvider {
  const config = Object.fromEntries(
    Object.entries(FLAGS).map(([key, flag]) => [
      key,
      { variants: { on: true, off: false }, defaultVariant: flag.default ? 'on' : 'off', disabled: false },
    ]),
  );
  return new InMemoryProvider(config);
}

export async function initFlags(clientKey: string | undefined): Promise<void> {
  const provider = clientKey ? createVendorProvider(clientKey) : defaultsProvider(); // vendor provider: one line, per vendor docs
  await OpenFeature.setProviderAndWait(provider); // first render already sees the right variants
}

/** Call after sign-in so targeting rules (percentage, plan, internal users) apply. */
export async function setFlagUser(user: { id: string; plan?: string }): Promise<void> {
  await OpenFeature.setContext({ targetingKey: user.id, ...(user.plan ? { plan: user.plan } : {}) });
}
```

Wrap the app in `<OpenFeatureProvider>` (in `providers.tsx`, `fe-app-architecture`), then read flags with a typed hook:

```tsx
import { useBooleanFlagValue } from '@openfeature/react-sdk';
import { FLAGS, type FlagKey } from '@/shared/config/flags';

export function useFlag(key: FlagKey): boolean {
  return useBooleanFlagValue(key, FLAGS[key].default); // typos are compile errors
}

// In a page:
const newCheckout = useFlag('checkout.new-flow');
return newCheckout ? <NewCheckout /> : <Checkout />;
```

**Flag rules**

| Rule | Why |
| --- | --- |
| Every flag is in the registry with an owner and a `removeBy` date | A CI check (or a weekly bot) lists overdue flags, so they get deleted |
| The default (`off`) is the safe, existing behaviour | If the flag service is down, users get the known-good path |
| Delete the flag and the old code path within two sprints of reaching 100% | Each live flag doubles the paths to test and reason about |
| Test both variants of a flagged feature | The off path is what runs when you switch the feature off in an incident |
| Never use flags for permissions or security | Flag values are visible and editable in the browser; authorization belongs on the server |
| Kill switches for risky dependencies (real-time, third-party widgets) are permanent and documented in the runbook | On-call can disable a failing subsystem without a deploy |
| Flag names are `area.feature` in kebab-case | Easy to search and to group by owning team |

## Observability

In production you need to answer three questions within minutes: **is something broken, for whom, and since which release?** Errors go to an error tracker (Sentry in these examples), real-user performance goes through the RUM endpoint from `fe-performance` to your analytics store, and browser traces connect to backend traces. Everything carries the release id, environment and route.

```bash
pnpm add @sentry/react
pnpm add -D @sentry/vite-plugin
```

**`src/app/observability.ts`**

```ts
import * as Sentry from '@sentry/react';
import type { RuntimeConfig } from '@/shared/config/runtime-config';
import { env } from '@/shared/config/env';
import type { AppRouter } from './router';

export function initObservability(config: RuntimeConfig): void {
  if (!config.sentryDsn) return; // local development: errors stay in the console
  Sentry.init({
    dsn: config.sentryDsn,
    environment: config.environment,
    release: env.VITE_RELEASE, // ties every error to the exact deploy
    sendDefaultPii: false,
    tracesSampleRate: config.environment === 'production' ? 0.1 : 1,
    tracePropagationTargets: [/^\/api\//], // adds trace headers to our API only, never third parties
    beforeSend(event) {
      if (event.request?.url) event.request.url = event.request.url.split('?')[0]; // drop query strings (tokens, emails)
      return event;
    },
  });
}

/** After the router exists: performance traces named by route pattern ('/orders/$orderId'). */
export function enableRouteTracing(router: AppRouter): void {
  Sentry.addIntegration(Sentry.tanstackRouterBrowserTracingIntegration(router));
}

/** After sign-in. An opaque id only, never email or name. */
export function setObservabilityUser(userId: string | null): void {
  Sentry.setUser(userId ? { id: userId } : null);
}
```

Call `enableRouteTracing(router)` in `main.tsx` right after `createAppRouter`.

**`src/shared/lib/errors.ts`** — `reportError` (`fe-app-architecture`) now reaches the tracker. Route errors, boundaries, query and mutation failures, and React 19's root error hooks all flow through it.

```ts
import * as Sentry from '@sentry/react';

export function reportError(error: unknown): void {
  if (isExpected(error)) return; // 4xx outcomes aren't bugs
  if (import.meta.env.DEV) console.error(error);
  Sentry.captureException(error); // no-op until Sentry.init runs
}
```

**Source maps: uploaded privately, then deleted** — add as the last plugin in `vite.config.ts`. It only runs in CI, where the token exists. `fe-repo-foundation` already builds with `sourcemap: 'hidden'`.

```ts
import { sentryVitePlugin } from '@sentry/vite-plugin';

// plugins: [ …existing plugins…,
process.env.SENTRY_AUTH_TOKEN
  ? sentryVitePlugin({
      org: 'acme',
      project: 'web-app',
      authToken: process.env.SENTRY_AUTH_TOKEN,
      release: { name: process.env.VITE_RELEASE },
      sourcemaps: { filesToDeleteAfterUpload: ['./dist/**/*.map'] }, // never served publicly
    })
  : null,
// ]
```

**Connect browser and server.** Trace headers on `/api` calls let one trace show the click, the request and the database query. Also attach the `X-Request-Id` that `http.ts` sends to `HttpError`, so support can quote it and find the server logs for a user's failed request.

**Dashboards** — one per audience, all filterable by release, route, browser and device:

| Dashboard | Shows | Source |
| --- | --- | --- |
| Release health | Crash-free sessions, new issues, error rate: current versus previous release | Error tracker |
| Web Vitals | p75 LCP, INP, CLS per route and device; attribution breakdown | RUM endpoint (`fe-performance`) |
| Key journeys | Funnel success for sign-in, create order, checkout | Product analytics |
| Client-seen API health | 4xx/5xx rate and latency as seen by browsers, per endpoint | RUM + traces |

**Service-level objectives and alerts**

| SLO (30-day window) | Target | Alert when | Severity → route |
| --- | --- | --- | --- |
| Crash-free sessions | ≥ 99.9% | Below 99.5% over 1 hour | Page the on-call engineer |
| New errors after a release | — | A new issue in the latest release exceeds 50 events in 10 minutes | Release channel + PR author |
| Canary health | Error rate ≤ current release | Canary error rate > 2× current | **Automatic rollback** + release channel |
| Web Vitals (top 5 routes) | p75 good for LCP, INP, CLS | Crosses "good" for 2 days (`fe-performance`) | Owning team's channel |
| Checkout success | ≥ baseline − 5% | 20% below the same hour last week | Page the on-call engineer |

Every alert links to a runbook. Alerts that fire without anyone needing to act get fixed or deleted within a week, because noisy alerts teach people to ignore all alerts.

**Runbook template** (`docs/runbooks/<alert-name>.md`):

```markdown
# Alert: Crash-free sessions below 99.5%

## What it means
More than 0.5% of sessions in the last hour hit an unhandled error.

## First 5 minutes
1. Open Release health → is the spike tied to the latest release?
2. Yes → roll back: `./scripts/deploy.sh production <previous-release>`. Then investigate.
3. No → check the top issue's stack trace and browser breakdown; check the API health dashboard.

## Kill switches
`ops.realtime-kill-switch` turns off real-time updates if the socket layer is failing.

## Escalation
Owning team: @acme/web-platform. Backend on-call if API health is also red.
```

## Release checklist

With the pipeline above, most releases need no manual steps. Use this list for releases that change something risky: auth, payments, data migrations, a large new feature, or infrastructure.

**Before**

- [ ] Risky changes are behind a feature flag that defaults to off, and the off path is tested.
- [ ] Full E2E suite is green on staging with this exact artifact.
- [ ] Dashboards and alerts exist for any new journey (release health, funnel step).
- [ ] The runbook covers the new failure modes, including which flag or kill switch to use.
- [ ] Backend changes are deployed first and are backward-compatible with the current front end (the old and new front ends run side by side during canary).
- [ ] The previous release id is noted for rollback.

**During**

- [ ] Canary at 5%: compare error rate, crash-free sessions and Web Vitals with the current release for 30 minutes.
- [ ] Widen to 50%, then 100%, only when the comparison is flat.
- [ ] Turn the feature flag on gradually (internal users → 5% → 50% → 100%), watching the same dashboards.

**After**

- [ ] Post the release id, notable changes (generated from PR titles) and the flag rollout plan in the release channel.
- [ ] Check the field Web Vitals for affected routes after 2–3 days.
- [ ] Schedule the flag clean-up PR for its `removeBy` date.
