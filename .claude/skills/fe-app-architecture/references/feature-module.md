# A complete feature module

The `orders` feature: model, api (queries, mutations, optimistic updates), components and public index.

## A complete feature module

A feature has four parts: `model/` (domain types and pure rules), `api/` (network and cache), `components/` (UI that receives data through props) and `index.ts` (the only public entry). Dependencies flow `components → api → model`, and `model` imports nothing from React.

**`features/orders/model/order.ts`** — schemas, types and business rules:

```ts
import { z } from 'zod';

export const OrderStatus = z.enum(['pending', 'paid', 'shipped', 'cancelled']);
export type OrderStatus = z.infer<typeof OrderStatus>;

export const Order = z.object({
  id: z.string(),
  customerName: z.string(),
  totalCents: z.number().int().nonnegative(), // money in minor units: never floats
  currency: z.string().length(3),
  status: OrderStatus,
  createdAt: z.iso.datetime(),
});
export type Order = z.infer<typeof Order>;

export const OrderPage = z.object({
  items: z.array(Order),
  totalItems: z.number().int().nonnegative(),
  page: z.number().int().positive(),
  pageSize: z.number().int().positive(),
});
export type OrderPage = z.infer<typeof OrderPage>;

/** URL search params for the list page. Invalid values fall back instead of crashing. */
export const OrdersSearch = z.object({
  page: z.number().int().min(1).default(1).catch(1),
  q: z.string().trim().max(100).default('').catch(''),
  status: OrderStatus.optional().catch(undefined),
});
export type OrdersSearch = z.infer<typeof OrdersSearch>;

export const CreateOrderInput = z.object({
  customerName: z.string().trim().min(2, 'Enter at least 2 characters').max(100),
  totalCents: z.number().int().positive('Total must be more than 0'),
  currency: z.enum(['USD', 'EUR', 'INR']),
});
export type CreateOrderInput = z.infer<typeof CreateOrderInput>;

export const STATUS_LABEL: Record<OrderStatus, string> = {
  pending: 'Pending',
  paid: 'Paid',
  shipped: 'Shipped',
  cancelled: 'Cancelled',
};

const CANCELLABLE: ReadonlySet<OrderStatus> = new Set(['pending', 'paid']);

export function canCancel(order: Pick<Order, 'status'>): boolean {
  return CANCELLABLE.has(order.status);
}

export function formatTotal(order: Pick<Order, 'totalCents' | 'currency'>, locale?: string): string {
  return new Intl.NumberFormat(locale, { style: 'currency', currency: order.currency }).format(order.totalCents / 100);
}
```

**`features/orders/api/orders-api.ts`** — keys, queries and mutations:

```ts
import { queryOptions, useMutation, useQueryClient } from '@tanstack/react-query';
import { http } from '@/shared/lib/http';
import { Order, OrderPage, type CreateOrderInput, type OrdersSearch } from '../model/order';

const PAGE_SIZE = 20;

// Hierarchical keys: invalidating orderKeys.all refreshes every orders query.
export const orderKeys = {
  all: ['orders'] as const,
  lists: () => [...orderKeys.all, 'list'] as const,
  list: (search: OrdersSearch) => [...orderKeys.lists(), search] as const,
  detail: (id: string) => [...orderKeys.all, 'detail', id] as const,
};

export function ordersListQuery(search: OrdersSearch) {
  return queryOptions({
    queryKey: orderKeys.list(search),
    queryFn: async ({ signal }) => {
      const params = new URLSearchParams({ page: String(search.page), pageSize: String(PAGE_SIZE), q: search.q });
      if (search.status) params.set('status', search.status);
      return OrderPage.parse(await http.get(`/orders?${params.toString()}`, { signal }));
    },
  });
}

export function orderDetailQuery(id: string) {
  return queryOptions({
    queryKey: orderKeys.detail(id),
    queryFn: async ({ signal }) => Order.parse(await http.get(`/orders/${encodeURIComponent(id)}`, { signal })),
  });
}

export function useCreateOrder() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (input: CreateOrderInput) => Order.parse(await http.post('/orders', input)),
    onSuccess: async (order) => {
      queryClient.setQueryData(orderDetailQuery(order.id).queryKey, order); // detail page opens instantly
      await queryClient.invalidateQueries({ queryKey: orderKeys.lists() });
    },
  });
}

/** Optimistic: the UI shows "Cancelled" immediately and rolls back if the server refuses. */
export function useCancelOrder() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (id: string) =>
      Order.parse(await http.post(`/orders/${encodeURIComponent(id)}/cancel`, {})),
    onMutate: async (id) => {
      const { queryKey } = orderDetailQuery(id);
      await queryClient.cancelQueries({ queryKey }); // stop an in-flight fetch overwriting our change
      const previous = queryClient.getQueryData(queryKey);
      if (previous) queryClient.setQueryData(queryKey, { ...previous, status: 'cancelled' });
      return { previous };
    },
    onError: (_error, id, result) => {
      if (result?.previous) queryClient.setQueryData(orderDetailQuery(id).queryKey, result.previous);
    },
    onSettled: () => queryClient.invalidateQueries({ queryKey: orderKeys.all }),
  });
}
```

The `queryOptions` helper ties the key to the data type, so `getQueryData(orderDetailQuery(id).queryKey)` returns `Order | undefined` without casts.

**`features/orders/components/status-badge.tsx`** — status is shown as text, not colour alone:

```tsx
import { STATUS_LABEL, type OrderStatus } from '../model/order';
import styles from './status-badge.module.css';

export function StatusBadge({ status }: { status: OrderStatus }) {
  return <span className={`${styles.badge} ${styles[status]}`}>{STATUS_LABEL[status]}</span>;
}
```

**`features/orders/components/orders-table.tsx`**

```tsx
import { Link } from '@tanstack/react-router';
import { formatTotal, type Order } from '../model/order';
import { StatusBadge } from './status-badge';

const dateFormat = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium' });

export function OrdersTable({ orders }: { orders: readonly Order[] }) {
  if (orders.length === 0) return <p role="status">No orders match these filters.</p>;

  return (
    <table>
      <caption className="visually-hidden">Orders</caption>
      <thead>
        <tr>
          <th scope="col">Order</th>
          <th scope="col">Customer</th>
          <th scope="col">Status</th>
          <th scope="col">Total</th>
          <th scope="col">Created</th>
        </tr>
      </thead>
      <tbody>
        {orders.map((order) => (
          <tr key={order.id}>
            <td>
              <Link to="/orders/$orderId" params={{ orderId: order.id }}>{order.id}</Link>
            </td>
            <td>{order.customerName}</td>
            <td><StatusBadge status={order.status} /></td>
            <td>{formatTotal(order)}</td>
            <td><time dateTime={order.createdAt}>{dateFormat.format(new Date(order.createdAt))}</time></td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
```

**`features/orders/components/order-filters.tsx`** — controlled through props, so the page decides where state lives (here, the URL):

```tsx
import { OrderStatus, STATUS_LABEL, type OrdersSearch } from '../model/order';

interface OrderFiltersProps {
  value: Pick<OrdersSearch, 'q' | 'status'>;
  onSearchChange: (q: string) => void;
  onStatusChange: (status: OrdersSearch['status']) => void;
}

export function OrderFilters({ value, onSearchChange, onStatusChange }: OrderFiltersProps) {
  return (
    <form role="search" onSubmit={(e) => e.preventDefault()}>
      <label htmlFor="order-q">Search</label>
      <input id="order-q" type="search" defaultValue={value.q} onChange={(e) => onSearchChange(e.target.value)} />

      <label htmlFor="order-status">Status</label>
      <select
        id="order-status"
        value={value.status ?? ''}
        onChange={(e) => {
          const parsed = OrderStatus.safeParse(e.target.value);
          onStatusChange(parsed.success ? parsed.data : undefined);
        }}
      >
        <option value="">All</option>
        {OrderStatus.options.map((s) => (
          <option key={s} value={s}>{STATUS_LABEL[s]}</option>
        ))}
      </select>
    </form>
  );
}
```

**`features/orders/index.ts`** — everything else in the folder is private:

```ts
export { orderDetailQuery, ordersListQuery, useCancelOrder, useCreateOrder } from './api/orders-api';
export { OrdersSearch, canCancel, formatTotal, type Order, type OrderStatus } from './model/order';
export { CreateOrderForm } from './components/create-order-form';
export { OrderFilters } from './components/order-filters';
export { OrdersTable } from './components/orders-table';
export { StatusBadge } from './components/status-badge';
```
