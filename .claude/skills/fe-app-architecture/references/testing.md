# Testing routes and features

Route render helper, factories, model tests and page tests.

## Testing routes and features

Test a feature through its real route: render a URL, let the real loaders and query cache run against MSW, and assert what the user sees. Pure rules in `model/` get plain unit tests. `fe-testing` covers the full strategy.

**`src/test/render-route.tsx`**

```tsx
import { render } from '@testing-library/react';
import { createMemoryHistory } from '@tanstack/react-router';
import { createAppRouter } from '@/app/router';
import { AppProviders } from '@/app/providers';
import { createTestQueryClient } from './render';

export async function renderRoute(path: string) {
  const queryClient = createTestQueryClient();
  const router = createAppRouter({ queryClient, history: createMemoryHistory({ initialEntries: [path] }) });
  await router.load(); // run loaders first, exactly like a real navigation
  const view = render(<AppProviders queryClient={queryClient} router={router} />);
  return { ...view, router, queryClient };
}
```

**`src/test/factories.ts`** — valid objects with overridable fields, so each test states only what matters:

```ts
import type { Order } from '@/features/orders';

let seq = 0;

export function buildOrder(overrides: Partial<Order> = {}): Order {
  seq += 1;
  return {
    id: `ord_${seq}`,
    customerName: 'Ada Lovelace',
    totalCents: 4999,
    currency: 'USD',
    status: 'pending',
    createdAt: '2026-10-01T10:00:00.000Z',
    ...overrides,
  };
}
```

**`features/orders/model/order.test.ts`**

```ts
import { canCancel, formatTotal } from './order';

test.each([
  ['pending', true],
  ['paid', true],
  ['shipped', false],
  ['cancelled', false],
] as const)('canCancel(%s) is %s', (status, expected) => {
  expect(canCancel({ status })).toBe(expected);
});

test('formats cents as currency', () => {
  expect(formatTotal({ totalCents: 4999, currency: 'USD' }, 'en-US')).toBe('$49.99');
});
```

**`src/pages/orders-page.test.tsx`**

```tsx
import { screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { delay, http, HttpResponse } from 'msw';
import { server } from '@/test/server';
import { renderRoute } from '@/test/render-route';
import { buildOrder } from '@/test/factories';

const pageOf = (items = [buildOrder()]) => ({ items, totalItems: items.length, page: 1, pageSize: 20 });

test('filters by status and keeps the filter in the URL', async () => {
  const requests: URL[] = [];
  server.use(
    http.get('*/api/orders', ({ request }) => {
      requests.push(new URL(request.url));
      return HttpResponse.json(pageOf());
    }),
  );
  const user = userEvent.setup();
  const { router } = await renderRoute('/orders');

  await user.selectOptions(await screen.findByLabelText('Status'), 'Paid');

  await vi.waitFor(() => expect(router.state.location.search).toMatchObject({ status: 'paid', page: 1 }));
  expect(requests.at(-1)?.searchParams.get('status')).toBe('paid');
});

test('shows the empty state', async () => {
  server.use(http.get('*/api/orders', () => HttpResponse.json(pageOf([]))));
  await renderRoute('/orders');
  expect(await screen.findByText(/no orders match/i)).toBeInTheDocument();
});

test('shows a friendly error when the API fails', async () => {
  server.use(http.get('*/api/orders', () => HttpResponse.json({}, { status: 500 })));
  await renderRoute('/orders');
  expect(await screen.findByRole('alert')).toHaveTextContent(/something went wrong on our side/i);
  expect(screen.getByRole('button', { name: 'Try again' })).toBeInTheDocument();
});

test('rolls back an optimistic cancel when the server refuses', async () => {
  server.use(
    http.get('*/api/orders/:id', () => HttpResponse.json(buildOrder({ id: 'ord_1', status: 'paid' }))),
    http.post('*/api/orders/:id/cancel', async () => {
      await delay(50);
      return HttpResponse.json({ code: 'already_shipped' }, { status: 409 });
    }),
  );
  const user = userEvent.setup();
  await renderRoute('/orders/ord_1');

  await user.click(await screen.findByRole('button', { name: 'Cancel order' }));
  expect(screen.getByText('Cancelled')).toBeInTheDocument(); // optimistic update
  expect(await screen.findByRole('alert')).toBeInTheDocument(); // server refused
  expect(screen.getByText('Paid')).toBeInTheDocument(); // rolled back
});
```

These four tests cover the riskiest behaviour — URL sync, the empty state, error recovery and rollback — without testing implementation details like hook calls or cache keys.
