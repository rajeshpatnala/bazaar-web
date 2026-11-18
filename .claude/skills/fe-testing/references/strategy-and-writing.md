# Strategy and writing good tests

Layer table, anti-patterns, what to mock, jsdom stubs and example tests.

## Contents

- What to test at which layer
- Writing good tests

## What to test at which layer

Choose the cheapest layer that can catch the bug. A wrong discount calculation needs a unit test, not an E2E run. A broken checkout journey needs E2E, because no unit test sees the real backend.

| Layer | Tool | Tests | Speed | Rough share of tests |
| --- | --- | --- | --- | --- |
| Static | TypeScript, ESLint, Oxlint | Typos, wrong types, unsafe patterns, a11y lint, import boundaries | Seconds | — (always on) |
| Unit | Vitest (Node/jsdom) | Pure functions in `model/`, formatters, `safeRedirect`, reducers | ~1 ms each | 30% |
| Component / integration | Vitest + Testing Library + MSW | A route or feature as a user sees it: render, interact, assert; real cache and router | ~50–300 ms each | 55% |
| Visual regression | Vitest Browser Mode `toMatchScreenshot` | Design-system components in every variant and theme | ~1 s each | Design system only |
| Accessibility | axe in Storybook and Playwright | WCAG violations on components and key pages | Runs alongside the above | Every story, key pages |
| End-to-end | Playwright | 5–15 critical journeys against a real deployed backend | 5–30 s each | ~10% |
| Contract | Generated API types + CI check | Front end and API agree on shapes | Seconds | One check |

**Where each kind of bug is caught**

| Bug | Caught by |
| --- | --- |
| `order.total` renamed by the backend | Contract check (types fail) or Zod parse in a component test |
| Cancel button shown for shipped orders | Unit test of `canCancel` |
| Filter doesn't update the URL | Component test through the route |
| Button loses its focus ring after a CSS refactor | Visual regression or a Storybook a11y check |
| Login works locally but cookies fail behind the CDN | E2E against the preview deployment |

As of October 2026, Vitest 5 is the current major and the examples in this playbook use APIs from its documentation. `fe-repo-foundation` pins Vitest 4.1; upgrade by following the official migration guide.

## Writing good tests

A good test reads like a short user story: **arrange** the situation, **act** like a user, **assert** what the user would see. If it needs comments to explain what it's checking, the test itself is too clever.

**How to find elements, in order of preference**

| Priority | Query | Use for |
| --- | --- | --- |
| 1 | `getByRole('button', { name: 'Pay' })` | Almost everything; also proves the element is accessible |
| 2 | `getByLabelText('Email')` | Form fields |
| 3 | `getByText(/no orders/i)` | Non-interactive text |
| 4 | `getByTestId('chart-canvas')` | Last resort, when nothing user-visible identifies it |

Use `getBy*` for things that must already be there, `findBy*` for things that appear after async work, and `queryBy*` only to assert something is **absent**.

**Anti-patterns and their fixes**

1. **Testing implementation instead of behaviour**

```tsx
// ❌ Breaks on any refactor, proves nothing a user cares about
expect(useCancelOrder).toHaveBeenCalledWith('ord_1');
expect(queryClient.getQueryState(['orders'])?.status).toBe('success');

// ✅ What the user experiences
await user.click(screen.getByRole('button', { name: 'Cancel order' }));
expect(await screen.findByText('Cancelled')).toBeInTheDocument();
```

2. **Fake events and manual waiting**

```tsx
// ❌ fireEvent skips focus, keyboard and pointer events real users trigger
fireEvent.change(input, { target: { value: 'Ada' } });
await new Promise((r) => setTimeout(r, 500));

// ✅ Real interaction; wait for the result, not for a guessed duration
const user = userEvent.setup();
await user.type(input, 'Ada');
expect(await screen.findByRole('status')).toHaveTextContent('Saved');
```

3. **Mocking your own code**

```tsx
// ❌ The test passes even if OrdersTable is broken
vi.mock('../components/orders-table', () => ({ OrdersTable: () => <div>table</div> }));

// ✅ Mock only the boundary (the network); render everything else for real
server.use(http.get('*/api/orders', () => HttpResponse.json(pageOf([buildOrder()]))));
```

4. **Several behaviours in one test**

```tsx
// ❌ When it fails, which behaviour broke?
test('orders page', async () => { /* filters, paginates, cancels, shows errors… */ });

// ✅ One behaviour each, named as a sentence
test('keeps the status filter in the URL', …);
test('shows the empty state when no orders match', …);
```

5. **Snapshots of large DOM trees**

```tsx
// ❌ A 300-line diff nobody reviews; gets updated blindly
expect(container).toMatchSnapshot();

// ✅ Assert the thing that matters
expect(screen.getByRole('row', { name: /ord_1/ })).toHaveTextContent('$49.99');
```

**What to mock and what not to**

| Mock | Don't mock |
| --- | --- |
| The network (MSW) | Your components and hooks |
| Time (`vi.useFakeTimers`, `vi.setSystemTime`) | TanStack Query or the router (use the real ones with test settings) |
| Randomness and generated ids | Zod schemas and business rules |
| Browser APIs jsdom lacks (`matchMedia`, `IntersectionObserver`) | The design-system components |
| Third-party SDKs (analytics, payments widget) | CSS modules |

**`src/test/setup.ts`** — add stubs for browser APIs that jsdom doesn't implement:

```ts
Object.defineProperty(window, 'matchMedia', {
  writable: true,
  value: (query: string) => ({
    matches: false,
    media: query,
    addEventListener: () => undefined,
    removeEventListener: () => undefined,
    onchange: null,
    dispatchEvent: () => false,
  }),
});

class NoopObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
  takeRecords() { return []; }
}
vi.stubGlobal('IntersectionObserver', NoopObserver);
vi.stubGlobal('ResizeObserver', NoopObserver);
```
