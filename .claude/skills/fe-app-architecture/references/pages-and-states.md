# Pages, UI states, forms and client state

Pages, loading/empty/error states and boundaries, forms with 422 mapping, Zustand store.

## Contents

- Pages
- Loading, empty and error states
- Forms
- Client state with Zustand

## Pages

A page connects the URL to features. It reads params and search, renders feature components, and turns user actions into navigation. It holds no fetch logic and no business rules, so it stays around 50 lines.

**`src/shared/lib/use-debounced-callback.ts`**

```ts
import { useEffect, useRef } from 'react';

export function useDebouncedCallback<A extends unknown[]>(fn: (...args: A) => void, delayMs: number) {
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []); // cancel on unmount
  return (...args: A) => {
    clearTimeout(timer.current);
    timer.current = setTimeout(() => fn(...args), delayMs);
  };
}
```

**`src/shared/ui/pagination.tsx`**

```tsx
interface PaginationProps {
  page: number;
  pageSize: number;
  totalItems: number;
  onPageChange: (page: number) => void;
}

export function Pagination({ page, pageSize, totalItems, onPageChange }: PaginationProps) {
  const pageCount = Math.max(1, Math.ceil(totalItems / pageSize));
  return (
    <nav aria-label="Pagination">
      <button type="button" onClick={() => onPageChange(page - 1)} disabled={page <= 1}>Previous</button>
      <span aria-live="polite">Page {page} of {pageCount}</span>
      <button type="button" onClick={() => onPageChange(page + 1)} disabled={page >= pageCount}>Next</button>
    </nav>
  );
}
```

**`src/pages/orders-page.tsx`**

```tsx
import { useSuspenseQuery } from '@tanstack/react-query';
import { useNavigate, useRouterState, useSearch } from '@tanstack/react-router';
import { OrderFilters, OrdersTable, ordersListQuery } from '@/features/orders';
import { Pagination } from '@/shared/ui/pagination';
import { useDebouncedCallback } from '@/shared/lib/use-debounced-callback';

export function OrdersPage() {
  const search = useSearch({ from: '/orders' }); // typed: { page: number; q: string; status?: OrderStatus }
  const navigate = useNavigate({ from: '/orders' });
  const { data } = useSuspenseQuery(ordersListQuery(search)); // already cached by the loader
  const isUpdating = useRouterState({ select: (s) => s.isLoading });

  const onSearchChange = useDebouncedCallback((q: string) => {
    void navigate({ search: (prev) => ({ ...prev, q, page: 1 }), replace: true });
  }, 300);

  return (
    <section aria-labelledby="orders-heading" aria-busy={isUpdating}>
      <h1 id="orders-heading">Orders</h1>
      <OrderFilters
        value={search}
        onSearchChange={onSearchChange}
        onStatusChange={(status) => void navigate({ search: (prev) => ({ ...prev, status, page: 1 }) })}
      />
      <OrdersTable orders={data.items} />
      <Pagination
        page={data.page}
        pageSize={data.pageSize}
        totalItems={data.totalItems}
        onPageChange={(page) => void navigate({ search: (prev) => ({ ...prev, page }) })}
      />
    </section>
  );
}
```

While the next page loads, the router keeps the current results on screen and `aria-busy` tells assistive technology an update is in progress, so the table doesn't flash to a skeleton on every filter change.

**`src/pages/order-detail-page.tsx`**

```tsx
import { useSuspenseQuery } from '@tanstack/react-query';
import { useParams } from '@tanstack/react-router';
import { StatusBadge, canCancel, formatTotal, orderDetailQuery, useCancelOrder } from '@/features/orders';
import { toUserMessage } from '@/shared/lib/errors';

export function OrderDetailPage() {
  const { orderId } = useParams({ from: '/orders/$orderId' });
  const { data: order } = useSuspenseQuery(orderDetailQuery(orderId));
  const cancel = useCancelOrder();

  return (
    <article aria-labelledby="order-heading">
      <h1 id="order-heading">Order {order.id}</h1>
      <dl>
        <dt>Customer</dt>
        <dd>{order.customerName}</dd>
        <dt>Status</dt>
        <dd><StatusBadge status={order.status} /></dd>
        <dt>Total</dt>
        <dd>{formatTotal(order)}</dd>
      </dl>

      {canCancel(order) && (
        <button type="button" disabled={cancel.isPending} onClick={() => cancel.mutate(order.id)}>
          {cancel.isPending ? 'Cancelling…' : 'Cancel order'}
        </button>
      )}
      {cancel.isError && <p role="alert">{toUserMessage(cancel.error)}</p>}
    </article>
  );
}
```

In production, put destructive actions like cancel behind the confirm dialog from the design system (`fe-design-system`).

## Loading, empty and error states

Every screen has five states. Each one has exactly one owner, so no component needs `if (isLoading) … if (error) …` chains.

| State | Owner | What the user sees |
| --- | --- | --- |
| First load of a page | Route `pendingComponent` (`PageSkeleton`) | Skeleton, only if loading takes over 1 s (router default), shown at least 500 ms so it doesn't flicker |
| First load of a widget | `WidgetBoundary` → `<Suspense>` | Skeleton in that widget only |
| Refetch / filter change | Router keeps current UI | Old data stays, `aria-busy` set |
| Empty result | The feature component | A sentence plus the next action ("No orders yet — create one") |
| Error | Route `errorComponent`, `WidgetBoundary`, or inline `role="alert"` for mutations | A plain message from `toUserMessage` plus **Try again** |
| Not found | Loader throws `notFound()` | The `NotFound` page, not an error |

**`src/shared/ui/error-fallback.tsx`**

```tsx
import { toUserMessage } from '@/shared/lib/errors';

interface ErrorFallbackProps {
  error: unknown;
  onRetry?: () => void;
  title?: string;
}

export function ErrorFallback({ error, onRetry, title = 'Something went wrong' }: ErrorFallbackProps) {
  return (
    <div role="alert">
      <h2>{title}</h2>
      <p>{toUserMessage(error)}</p>
      {onRetry && <button type="button" onClick={onRetry}>Try again</button>}
    </div>
  );
}
```

**`src/shared/ui/widget-boundary.tsx`** — one failing widget never blanks the page:

```tsx
import { Suspense, type ReactNode } from 'react';
import { QueryErrorResetBoundary } from '@tanstack/react-query';
import { ErrorBoundary } from 'react-error-boundary';
import { ErrorFallback } from './error-fallback';
import { Skeleton } from './skeleton';

interface WidgetBoundaryProps {
  name: string; // "recent activity" → "Couldn't load recent activity"
  children: ReactNode;
  fallback?: ReactNode;
}

export function WidgetBoundary({ name, children, fallback }: WidgetBoundaryProps) {
  return (
    <QueryErrorResetBoundary>
      {({ reset }) => (
        <ErrorBoundary
          onReset={reset} // clears the failed query so Try again refetches
          fallbackRender={({ error, resetErrorBoundary }) => (
            <ErrorFallback title={`Couldn't load ${name}`} error={error} onRetry={resetErrorBoundary} />
          )}
        >
          <Suspense fallback={fallback ?? <Skeleton />}>{children}</Suspense>
        </ErrorBoundary>
      )}
    </QueryErrorResetBoundary>
  );
}
```

**`src/shared/ui/skeleton.tsx`**

```tsx
import styles from './skeleton.module.css';

export function Skeleton({ lines = 3 }: { lines?: number }) {
  return (
    <div className={styles.skeleton} aria-busy="true">
      <span className="visually-hidden">Loading…</span>
      {Array.from({ length: lines }, (_, i) => <div key={i} className={styles.line} aria-hidden="true" />)}
    </div>
  );
}

export function PageSkeleton() {
  return <Skeleton lines={8} />;
}
```

**404s as "not found", not errors** — update the detail route's loader in `router.tsx`:

```tsx
import { notFound } from '@tanstack/react-router';
import { HttpError } from '@/shared/lib/http';

// inside orderDetailRoute:
loader: async ({ context, params }) => {
  try {
    await context.queryClient.ensureQueryData(orderDetailQuery(params.orderId));
  } catch (error) {
    if (error instanceof HttpError && error.status === 404) throw notFound();
    throw error; // everything else goes to RouteError
  }
},
```

**Usage on a dashboard page:**

```tsx
<WidgetBoundary name="recent orders">
  <RecentOrders /> {/* uses useSuspenseQuery inside */}
</WidgetBoundary>
```

## Forms

The Zod schema validates in the browser for instant feedback. The server stays the authority: its `422` field errors are mapped back onto the same inputs, so users see one consistent error style wherever the error came from.

**API contract assumed:** a validation failure returns `422` with `{ "fieldErrors": { "customerName": ["Already exists"] } }`. Agree on one shape for every endpoint.

**`features/orders/components/create-order-form.tsx`**

```tsx
import { useForm } from 'react-hook-form';
import { zodResolver } from '@hookform/resolvers/zod';
import { z } from 'zod';
import { HttpError } from '@/shared/lib/http';
import { toUserMessage } from '@/shared/lib/errors';
import { useCreateOrder } from '../api/orders-api';
import { CreateOrderInput, type Order } from '../model/order';

// Users type money in major units; the API takes cents.
const FormSchema = CreateOrderInput.omit({ totalCents: true }).extend({
  total: z.number({ error: 'Enter an amount' }).positive('Total must be more than 0'),
});
type FormValues = z.infer<typeof FormSchema>;

const ValidationErrorBody = z.object({ fieldErrors: z.record(z.string(), z.array(z.string())) });
const SERVER_TO_FORM_FIELD: Record<string, keyof FormValues> = {
  customerName: 'customerName',
  totalCents: 'total',
  currency: 'currency',
};

export function CreateOrderForm({ onCreated }: { onCreated: (order: Order) => void }) {
  const createOrder = useCreateOrder();
  const {
    register,
    handleSubmit,
    setError,
    formState: { errors, isSubmitting },
  } = useForm<FormValues>({
    resolver: zodResolver(FormSchema),
    defaultValues: { customerName: '', currency: 'USD' },
  });

  async function onSubmit(values: FormValues) {
    try {
      const order = await createOrder.mutateAsync({
        customerName: values.customerName,
        currency: values.currency,
        totalCents: Math.round(values.total * 100),
      });
      onCreated(order);
    } catch (error) {
      const body = error instanceof HttpError && error.status === 422 ? ValidationErrorBody.safeParse(error.body) : null;
      if (body?.success) {
        for (const [serverField, messages] of Object.entries(body.data.fieldErrors)) {
          const field = SERVER_TO_FORM_FIELD[serverField];
          if (field) setError(field, { message: messages[0] });
        }
        return;
      }
      setError('root.server', { message: toUserMessage(error) });
    }
  }

  return (
    <form onSubmit={handleSubmit(onSubmit)} noValidate>
      <label htmlFor="customerName">Customer name</label>
      <input
        id="customerName"
        autoComplete="name"
        aria-invalid={Boolean(errors.customerName)}
        aria-describedby={errors.customerName ? 'customerName-error' : undefined}
        {...register('customerName')}
      />
      {errors.customerName && <p id="customerName-error">{errors.customerName.message}</p>}

      <label htmlFor="total">Total</label>
      <input
        id="total"
        type="number"
        inputMode="decimal"
        step="0.01"
        aria-invalid={Boolean(errors.total)}
        aria-describedby={errors.total ? 'total-error' : undefined}
        {...register('total', { valueAsNumber: true })}
      />
      {errors.total && <p id="total-error">{errors.total.message}</p>}

      <label htmlFor="currency">Currency</label>
      <select id="currency" {...register('currency')}>
        <option value="USD">USD</option>
        <option value="EUR">EUR</option>
        <option value="INR">INR</option>
      </select>

      {errors.root?.server && <p role="alert">{errors.root.server.message}</p>}
      <button type="submit" disabled={isSubmitting}>{isSubmitting ? 'Creating…' : 'Create order'}</button>
    </form>
  );
}
```

**Form rules**

- `noValidate` turns off browser bubbles so all errors look the same; React Hook Form focuses the first invalid field on submit.
- Validate on submit first, then on change (React Hook Form's default), so users aren't scolded while still typing.
- Disable the submit button only while submitting — never as a way to signal invalid input.
- Every error is linked to its input with `aria-describedby`, and `aria-invalid` marks the field for screen readers.
- `fe-design-system` replaces the repeated label/input/error markup with a shared `Field` component.

## Client state with Zustand

Most apps need very little global client state once server data lives in the query cache and view state lives in the URL. Use a store only when the answer to every question below is yes.

1. Is it client-only (not from the server)?
2. Should it survive navigation but **not** be shareable via a link (otherwise use the URL)?
3. Do distant components need it (otherwise use `useState` and props)?

Typical results: sidebar open/closed, table density, dismissed banners, a multi-step wizard's draft. Never put server data, form fields or auth tokens in a store.

**`src/shared/stores/ui-store.ts`**

```ts
import { create } from 'zustand';
import { createJSONStorage, persist } from 'zustand/middleware';

export type Density = 'comfortable' | 'compact';

interface UiState {
  sidebarOpen: boolean;
  density: Density;
  toggleSidebar: () => void;
  setDensity: (density: Density) => void;
}

export const useUiStore = create<UiState>()(
  persist(
    (set) => ({
      sidebarOpen: true,
      density: 'comfortable',
      toggleSidebar: () => set((s) => ({ sidebarOpen: !s.sidebarOpen })),
      setDensity: (density) => set({ density }),
    }),
    {
      name: 'ui-prefs',
      version: 1, // bump and add `migrate` when the shape changes
      storage: createJSONStorage(() => localStorage),
      partialize: (s) => ({ sidebarOpen: s.sidebarOpen, density: s.density }), // persist data, not functions
    },
  ),
);

// Export narrow selector hooks: a component re-renders only when its slice changes.
export const useDensity = () => useUiStore((s) => s.density);
export const useSidebarOpen = () => useUiStore((s) => s.sidebarOpen);
```

**Rules**

- Components subscribe through selectors (`useDensity()`), never `useUiStore()` without one — that re-renders on every change.
- State changes go through actions defined in the store, so all ways of changing a value are in one place.
- One small store per concern (UI preferences, wizard draft), not one giant app store.
- React Context is for injecting stable dependencies (a theme object, a feature-flag client), not for values that change often.
- Reset stores between tests: `afterEach(() => useUiStore.setState(useUiStore.getInitialState()))`.
