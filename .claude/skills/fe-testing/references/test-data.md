# Test data, time and randomness

Factories, per-feature mocks and scenarios, aggregated handlers, dev MSW worker, fake timers and stable ids.

## Test data, time and randomness

Every test builds its own data from factories and states only the fields it cares about. Mock API handlers live next to the feature they mock and are reused by tests, Storybook and local development, so there is one fake backend instead of three that drift apart.

**`src/test/factories.ts`** — a tiny generic helper; no library needed:

```ts
import type { Order, OrderPage } from '@/features/orders';

function factory<T>(build: (seq: number) => T) {
  let seq = 0;
  return (overrides: Partial<T> = {}): T => {
    seq += 1;
    return { ...build(seq), ...overrides };
  };
}

export const buildOrder = factory<Order>((n) => ({
  id: `ord_${n}`,
  customerName: `Customer ${n}`,
  totalCents: 4999,
  currency: 'USD',
  status: 'pending',
  createdAt: '2026-10-01T10:00:00.000Z',
}));

export function buildOrderPage(items: Order[] = [buildOrder()]): OrderPage {
  return { items, totalItems: items.length, page: 1, pageSize: 20 };
}
```

**`src/test/factories.test.ts`** — keeps factories honest. If the schema changes, this fails before any feature test misleads you:

```ts
import { Order, OrderPage } from '@/features/orders';
import { buildOrder, buildOrderPage } from './factories';

test('factories produce schema-valid data', () => {
  expect(() => Order.parse(buildOrder())).not.toThrow();
  expect(() => OrderPage.parse(buildOrderPage())).not.toThrow();
});
```

(Export `OrderPage` from the orders feature's `index.ts` for this.)

**`features/orders/api/orders.mocks.ts`** — default handlers plus named scenarios:

```ts
import { delay, http, HttpResponse } from 'msw';
import { buildOrder, buildOrderPage } from '@/test/factories';

export const orderHandlers = [
  http.get('*/api/orders', () => HttpResponse.json(buildOrderPage())),
  http.get('*/api/orders/:id', ({ params }) => HttpResponse.json(buildOrder({ id: String(params.id) }))),
];

/** Pick by name in tests (server.use(orderScenarios.empty)) and stories. */
export const orderScenarios = {
  empty: http.get('*/api/orders', () => HttpResponse.json(buildOrderPage([]))),
  serverError: http.get('*/api/orders', () => HttpResponse.json({}, { status: 500 })),
  slow: http.get('*/api/orders', async () => {
    await delay(2000);
    return HttpResponse.json(buildOrderPage());
  }),
};
```

Move `fe-auth-realtime`'s `testSession`, default `/auth/me` handler and `signedOut` list into `features/auth/api/auth.mocks.ts`, exported as `testSession`, `authHandlers` and `signedOut`.

**`src/test/handlers.ts`** — aggregates every feature's defaults:

```ts
import { orderHandlers } from '@/features/orders/api/orders.mocks';
import { authHandlers } from '@/features/auth/api/auth.mocks';

export const handlers = [...authHandlers, ...orderHandlers];
```

Exclude `**/*.mocks.ts` from coverage, and import mocks only from tests, stories and the dev worker below, so they never reach the production bundle.

**Mock the API in local development** (optional) — front-end work can start before the backend is ready, using the same handlers:

```ts
// src/test/browser-worker.ts
import { setupWorker } from 'msw/browser';
import { handlers } from './handlers';

export const worker = setupWorker(...handlers);
```

```ts
// src/main.tsx, before createRoot(...). Run `pnpm exec msw init public/` once.
if (import.meta.env.DEV && import.meta.env.VITE_MOCK_API === 'true') {
  const { worker } = await import('@/test/browser-worker'); // dynamic: never in prod builds
  await worker.start({ onUnhandledRequest: 'bypass' });
}
```

**Time** — freeze it for anything date-dependent:

```ts
test('marks an order overdue after 7 days', async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  vi.setSystemTime(new Date('2026-10-31T09:00:00Z'));
  const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
  // … render, interact, assert
});
```

Add `afterEach(() => vi.useRealTimers())` to `src/test/setup.ts`, so a forgotten fake timer never leaks into the next test.

**Timezone and locale** — put `process.env.TZ = 'UTC';` at the top of `vitest.config.ts`, and always pass an explicit locale to formatters in assertions (`formatTotal(order, 'en-US')`). Tests then behave the same on a laptop in Bengaluru and a CI runner in Virginia.

**Random values** — stub them where output depends on them:

```ts
vi.spyOn(crypto, 'randomUUID').mockReturnValue('00000000-0000-4000-8000-000000000001');
```
