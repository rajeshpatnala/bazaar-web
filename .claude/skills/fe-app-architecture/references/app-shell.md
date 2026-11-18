# App shell, providers and routing

What gets built, data flow, entry point, providers, query policy, router and root layout.

## Contents

- What this playbook builds
- Data flow at a glance
- Entry point, providers and query policy
- Routing

## What this playbook builds

A working app shell plus one complete feature (`orders`: list with filters and pagination, detail, cancel, create). Every new feature copies this shape. It builds on the files from `fe-repo-foundation` (`env.ts`, `http.ts`, test setup).

**Core rules**

- The **URL** owns shareable view state (filters, page, tab). The **query cache** owns server data. **Components** own only transient UI state.
- Route **loaders** start data fetching before the page component renders, so there are no request waterfalls.
- Every response is **parsed with Zod** inside the feature's `api/` folder. Nothing else in the app sees unvalidated data.

**Packages**

```bash
pnpm add @tanstack/react-router react-error-boundary zustand @hookform/resolvers
pnpm add -D @tanstack/react-query-devtools @tanstack/react-router-devtools
```

**Files added**

```text
src/
├─ main.tsx
├─ app/
│  ├─ query-client.ts      # cache and retry policy
│  ├─ router.tsx           # route tree + createAppRouter()
│  ├─ providers.tsx        # QueryClientProvider + RouterProvider
│  └─ root-layout.tsx      # header, <main>, skip link, route error UI
├─ pages/
│  ├─ orders-page.tsx
│  └─ order-detail-page.tsx
├─ features/orders/
│  ├─ api/orders-api.ts    # schemas, fetchers, queryOptions, mutations
│  ├─ model/order.ts       # pure business rules
│  ├─ components/          # orders-table, order-filters, create-order-form, status-badge
│  └─ index.ts             # public API
├─ shared/
│  ├─ lib/errors.ts        # error → user message
│  ├─ stores/ui-store.ts   # Zustand (client-only preferences)
│  └─ ui/                  # error-fallback, pagination, skeleton
└─ test/render-route.tsx   # render any URL in tests
```

## Data flow at a glance

*Diagram: data flow · read path and write path through the query cache*

Every screen reads server data from the query cache, which the route loader fills before render. Every write updates the cache optimistically, then invalidates related keys so lists refetch the confirmed data.

## Entry point, providers and query policy

One `QueryClient` and one router are created at startup and passed down. Retry, caching and error-reporting rules live in one file, so no component decides them ad hoc.

**`src/shared/lib/errors.ts`** — one place that turns any error into a user message and decides what to report:

```ts
import { HttpError } from './http';

/** 4xx responses are expected outcomes (validation, not found), not bugs. */
export function isExpected(error: unknown): boolean {
  return error instanceof HttpError && error.status < 500;
}

export function toUserMessage(error: unknown): string {
  if (error instanceof HttpError) {
    if (error.status === 401) return 'Your session expired. Please sign in again.';
    if (error.status === 403) return "You don't have access to this.";
    if (error.status === 404) return "We couldn't find that.";
    if (error.status === 429) return 'Too many requests. Wait a moment and try again.';
    if (error.status >= 500) return 'Something went wrong on our side. Please try again.';
  }
  if (error instanceof DOMException && error.name === 'TimeoutError') {
    return 'The request timed out. Check your connection and try again.';
  }
  return 'Something went wrong. Please try again.';
}

export function reportError(error: unknown): void {
  if (isExpected(error)) return;
  console.error(error); // `fe-cicd-observability` replaces this with Sentry.captureException(error)
}
```

Zod parse failures are not `HttpError`s, so a backend contract break is always reported.

**`src/app/query-client.ts`**

```ts
import { MutationCache, QueryCache, QueryClient } from '@tanstack/react-query';
import { HttpError } from '@/shared/lib/http';
import { reportError } from '@/shared/lib/errors';

const MAX_RETRIES = 2;
const RETRYABLE_4XX = new Set([408, 429]);

function shouldRetry(failureCount: number, error: unknown): boolean {
  const clientError = error instanceof HttpError && error.status < 500 && !RETRYABLE_4XX.has(error.status);
  return !clientError && failureCount < MAX_RETRIES; // retrying a 404 never helps
}

export function createQueryClient(): QueryClient {
  return new QueryClient({
    queryCache: new QueryCache({ onError: reportError }),
    mutationCache: new MutationCache({ onError: reportError }),
    defaultOptions: {
      queries: {
        staleTime: 30_000, // treat data as fresh for 30 s: no refetch on every mount
        gcTime: 5 * 60_000, // keep unused data 5 min for instant back-navigation
        retry: shouldRetry,
      },
      mutations: { retry: false }, // never repeat a write automatically
    },
  });
}
```

Tune `staleTime` per query, not globally: a stock price might use 0, a country list `Infinity`.

**`src/app/providers.tsx`**

```tsx
import { lazy, Suspense } from 'react';
import { QueryClientProvider, type QueryClient } from '@tanstack/react-query';
import { RouterProvider } from '@tanstack/react-router';
import type { AppRouter } from './router';

// Devtools are code-split and never shipped to production.
const QueryDevtools = import.meta.env.DEV
  ? lazy(() => import('@tanstack/react-query-devtools').then((m) => ({ default: m.ReactQueryDevtools })))
  : () => null;

interface AppProvidersProps {
  queryClient: QueryClient;
  router: AppRouter;
}

export function AppProviders({ queryClient, router }: AppProvidersProps) {
  return (
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
      <Suspense>
        <QueryDevtools />
      </Suspense>
    </QueryClientProvider>
  );
}
```

**`src/main.tsx`**

```tsx
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { createQueryClient } from '@/app/query-client';
import { createAppRouter } from '@/app/router';
import { AppProviders } from '@/app/providers';
import { reportError } from '@/shared/lib/errors';
import '@/shared/styles/global.css';

const queryClient = createQueryClient();
const router = createAppRouter({ queryClient });

const container = document.getElementById('root');
if (!container) throw new Error('#root element missing from index.html');

createRoot(container, {
  onUncaughtError: reportError, // React 19: errors no boundary caught
  onCaughtError: reportError, // errors a boundary caught and rendered a fallback for
}).render(
  <StrictMode>
    <AppProviders queryClient={queryClient} router={router} />
  </StrictMode>,
);
```

## Routing

TanStack Router gives fully typed paths, params and search params. A typo in a link or a wrong search-param type is a compile error, not a broken page. Each route validates its URL, starts its data loading and lazy-loads its page code.

**`src/app/router.tsx`**

```tsx
import {
  createRootRouteWithContext,
  createRoute,
  createRouter,
  lazyRouteComponent,
  type RouterHistory,
} from '@tanstack/react-router';
import type { QueryClient } from '@tanstack/react-query';
import { OrdersSearch, orderDetailQuery, ordersListQuery } from '@/features/orders';
import { PageSkeleton } from '@/shared/ui/skeleton';
import { NotFound, RootLayout, RouteError } from './root-layout';

interface RouterContext {
  queryClient: QueryClient;
}

const rootRoute = createRootRouteWithContext<RouterContext>()({
  component: RootLayout,
  errorComponent: RouteError,
  notFoundComponent: NotFound,
});

const homeRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/',
  component: lazyRouteComponent(() => import('@/pages/home-page'), 'HomePage'),
});

const ordersRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/orders',
  validateSearch: OrdersSearch, // bad or missing params fall back to defaults
  loaderDeps: ({ search }) => search, // reload when filters change
  loader: ({ context, deps }) => context.queryClient.ensureQueryData(ordersListQuery(deps)),
  component: lazyRouteComponent(() => import('@/pages/orders-page'), 'OrdersPage'),
});

const orderDetailRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/orders/$orderId',
  loader: ({ context, params }) => context.queryClient.ensureQueryData(orderDetailQuery(params.orderId)),
  component: lazyRouteComponent(() => import('@/pages/order-detail-page'), 'OrderDetailPage'),
});

const routeTree = rootRoute.addChildren([homeRoute, ordersRoute, orderDetailRoute]);

interface CreateAppRouterOptions {
  queryClient: QueryClient;
  history?: RouterHistory; // tests pass a memory history
}

export function createAppRouter({ queryClient, history }: CreateAppRouterOptions) {
  return createRouter({
    routeTree,
    context: { queryClient },
    ...(history ? { history } : {}),
    defaultPreload: 'intent', // hover or focus on a link prefetches its code and data
    defaultPreloadStaleTime: 0, // TanStack Query decides freshness, not the router
    defaultPendingComponent: PageSkeleton,
    defaultPendingMinMs: 500, // once shown, keep the skeleton 500 ms so it never flashes
    scrollRestoration: true,
  });
}

export type AppRouter = ReturnType<typeof createAppRouter>;

declare module '@tanstack/react-router' {
  interface Register {
    router: AppRouter; // makes every Link, useSearch and useParams type-safe app-wide
  }
}
```

**Loader rules**

| Data | In the loader | In the component |
| --- | --- | --- |
| Needed to render the page at all | `await ensureQueryData(...)` | `useSuspenseQuery(...)` — data is already cached |
| Secondary (side panel, recommendations) | `void queryClient.prefetchQuery(...)` (don't await) | `useSuspenseQuery` inside its own `<Suspense>` |
| Only after user action | nothing | `useQuery` with `enabled` or a mutation |

The loader and the component use the **same `queryOptions` object**, so their keys can't drift apart.

**`src/app/root-layout.tsx`** — the shell every page renders inside:

```tsx
import { lazy, Suspense, useEffect } from 'react';
import { Link, Outlet, useRouter, type ErrorComponentProps } from '@tanstack/react-router';
import { useQueryErrorResetBoundary } from '@tanstack/react-query';
import { ErrorFallback } from '@/shared/ui/error-fallback';

const RouterDevtools = import.meta.env.DEV
  ? lazy(() => import('@tanstack/react-router-devtools').then((m) => ({ default: m.TanStackRouterDevtools })))
  : () => null;

export function RootLayout() {
  const router = useRouter();

  // Accessibility: after navigation, move focus to <main> so screen readers announce the new page.
  useEffect(
    () =>
      router.subscribe('onResolved', ({ pathChanged }) => {
        if (pathChanged) document.getElementById('main')?.focus();
      }),
    [router],
  );

  return (
    <>
      <a href="#main" className="skip-link">Skip to content</a>
      <header>
        <nav aria-label="Main">
          <Link to="/">Home</Link>
          <Link to="/orders">Orders</Link>
        </nav>
      </header>
      <main id="main" tabIndex={-1}>
        <Outlet />
      </main>
      <Suspense>
        <RouterDevtools />
      </Suspense>
    </>
  );
}

export function RouteError({ error, reset }: ErrorComponentProps) {
  const router = useRouter();
  const { reset: resetQueryErrors } = useQueryErrorResetBoundary();
  useEffect(() => resetQueryErrors(), [resetQueryErrors]);

  return (
    <ErrorFallback
      error={error}
      onRetry={() => {
        reset();
        void router.invalidate(); // re-run loaders
      }}
    />
  );
}

export function NotFound() {
  return (
    <section>
      <h1>Page not found</h1>
      <Link to="/">Go to the home page</Link>
    </section>
  );
}
```

**URL state rules**

- Anything a user would expect to survive a refresh or a shared link — filters, sort, page, selected tab, open item — goes in search params, validated by a Zod schema with defaults.
- Update search params with `navigate({ search: (prev) => ({ ...prev, page: 2 }) })`, never by building query strings by hand.
- Typing into a search box replaces history (`replace: true`); discrete choices (page, tab) push a new entry so Back works.
