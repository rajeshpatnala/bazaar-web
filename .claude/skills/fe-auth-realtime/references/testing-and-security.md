# Testing and security checklist

Tests for auth and real-time failure paths, E2E auth setup and the security review checklist.

## Contents

- Testing auth and real-time
- Security checklist

## Testing auth and real-time

Test the failure paths, because that's where auth and real-time bugs live: concurrent `401`s, failed refresh, open redirects, malformed events, reconnect timing. The pure functions (`safeRedirect`, `backoffDelay`, `applyOrderEvent`) are tested without a browser or socket.

**`src/test/handlers.ts`** — replaces `fe-repo-foundation`'s version; tests are signed in by default:

```ts
import { http, HttpResponse } from 'msw';

export const testSession = {
  user: { id: 'u_1', name: 'Test User', email: 'test@example.com' },
  permissions: ['orders:read', 'orders:create', 'orders:cancel'],
};

export const handlers = [http.get('*/api/auth/me', () => HttpResponse.json(testSession))];

/** server.use(...signedOut) for a test that starts signed out. */
export const signedOut = [
  http.get('*/api/auth/me', () => new HttpResponse(null, { status: 401 })),
  http.post('*/api/auth/refresh', () => new HttpResponse(null, { status: 401 })),
];
```

**`src/shared/lib/http.test.ts`** — add:

```ts
import { delay, http as mock, HttpResponse } from 'msw';
import { server } from '@/test/server';
import { authEvents, http } from './http';

test('refreshes once for concurrent 401s, then retries each request', async () => {
  let refreshCalls = 0;
  let signedIn = false;
  server.use(
    mock.get('*/api/data', () => (signedIn ? HttpResponse.json({ ok: true }) : new HttpResponse(null, { status: 401 }))),
    mock.post('*/api/auth/refresh', async () => {
      refreshCalls += 1;
      await delay(50); // keep the refresh in flight while the other 401s arrive
      signedIn = true;
      return new HttpResponse(null, { status: 204 });
    }),
  );

  const results = await Promise.all([http.get('/data'), http.get('/data'), http.get('/data')]);

  expect(results).toEqual([{ ok: true }, { ok: true }, { ok: true }]);
  expect(refreshCalls).toBe(1);
});

test('announces session-expired when refresh fails', async () => {
  server.use(
    mock.get('*/api/data', () => new HttpResponse(null, { status: 401 })),
    mock.post('*/api/auth/refresh', () => new HttpResponse(null, { status: 401 })),
  );
  const onExpired = vi.fn();
  authEvents.addEventListener('session-expired', onExpired);

  await expect(http.get('/data')).rejects.toMatchObject({ status: 401 });
  expect(onExpired).toHaveBeenCalledOnce();
  authEvents.removeEventListener('session-expired', onExpired);
});

test('sends the CSRF token on writes only', async () => {
  document.cookie = 'csrf_token=abc123';
  const seen: (string | null)[] = [];
  server.use(
    mock.all('*/api/echo', ({ request }) => {
      seen.push(request.headers.get('X-CSRF-Token'));
      return HttpResponse.json({});
    }),
  );

  await http.get('/echo');
  await http.post('/echo', {});
  expect(seen).toEqual([null, 'abc123']);
});
```

**`features/auth/model/session.test.ts`**

```ts
import { safeRedirect } from './session';

test.each([
  ['/orders?page=2', '/orders?page=2'],
  [undefined, '/'],
  ['https://evil.com', '/'],
  ['//evil.com', '/'],
  ['/\\evil.com', '/'],
  ['javascript:alert(1)', '/'],
])('safeRedirect(%s) → %s', (input, expected) => {
  expect(safeRedirect(input)).toBe(expected);
});
```

**`src/app/auth-guard.test.tsx`**

```tsx
import { screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { server } from '@/test/server';
import { signedOut, testSession } from '@/test/handlers';
import { renderRoute } from '@/test/render-route';

test('sends signed-out users to login, then back to the page they wanted', async () => {
  server.use(
    ...signedOut,
    http.post('*/api/auth/login', () => {
      server.use(http.get('*/api/auth/me', () => HttpResponse.json(testSession))); // now signed in
      return HttpResponse.json(testSession);
    }),
    http.get('*/api/orders', () => HttpResponse.json({ items: [], totalItems: 0, page: 1, pageSize: 20 })),
  );
  const user = userEvent.setup();
  const { router } = await renderRoute('/orders');

  expect(await screen.findByRole('heading', { name: 'Sign in' })).toBeInTheDocument();
  expect(router.state.location.search).toMatchObject({ redirect: expect.stringContaining('/orders') });

  await user.type(screen.getByLabelText('Email'), 'test@example.com');
  await user.type(screen.getByLabelText('Password'), 'correct-horse');
  await user.click(screen.getByRole('button', { name: 'Sign in' }));

  expect(await screen.findByRole('heading', { name: 'Orders' })).toBeInTheDocument();
});
```

**`src/shared/lib/realtime.test.ts`**

```ts
import { backoffDelay } from './realtime';

test.each([
  [0, 1_000],
  [1, 2_000],
  [4, 16_000],
  [5, 30_000], // capped
  [20, 30_000],
])('attempt %i waits between 0 and %i ms', (attempt, max) => {
  expect(backoffDelay(attempt, 30_000, () => 0)).toBe(0);
  expect(backoffDelay(attempt, 30_000, () => 0.999_999)).toBeLessThanOrEqual(max);
});
```

**`features/orders/api/orders-realtime.test.ts`**

```ts
import { createTestQueryClient } from '@/test/render';
import { buildOrder } from '@/test/factories';
import { orderDetailQuery } from './orders-api';
import { applyOrderEvent } from './orders-realtime';

test('writes a valid order event into the cache', () => {
  const queryClient = createTestQueryClient();
  const order = buildOrder({ id: 'ord_1', status: 'shipped' });

  expect(applyOrderEvent(queryClient, { type: 'order.updated', payload: order })).toBe(true);
  expect(queryClient.getQueryData(orderDetailQuery('ord_1').queryKey)).toEqual(order);
});

test('ignores malformed payloads and other event types', () => {
  const queryClient = createTestQueryClient();
  vi.spyOn(console, 'error').mockImplementation(() => undefined);

  expect(applyOrderEvent(queryClient, { type: 'order.updated', payload: { id: 'ord_1' } })).toBe(true);
  expect(queryClient.getQueryData(orderDetailQuery('ord_1').queryKey)).toBeUndefined();
  expect(applyOrderEvent(queryClient, { type: 'chat.message', payload: {} })).toBe(false);
});
```

**E2E: sign in once, reuse the session.** Add a setup project to `playwright.config.ts`, so only one test exercises the login form and the rest start signed in:

```ts
// projects in playwright.config.ts
{ name: 'setup', testMatch: /auth\.setup\.ts/ },
{ name: 'chromium', use: { ...devices['Desktop Chrome'], storageState: 'e2e/.auth/user.json' }, dependencies: ['setup'] },
```

```ts
// e2e/auth.setup.ts — add e2e/.auth/ to .gitignore
import { test as setup, expect } from '@playwright/test';

setup('sign in', async ({ page }) => {
  await page.goto('/login');
  await page.getByLabel('Email').fill(process.env.E2E_USER_EMAIL ?? '');
  await page.getByLabel('Password').fill(process.env.E2E_USER_PASSWORD ?? '');
  await page.getByRole('button', { name: 'Sign in' }).click();
  await expect(page).toHaveURL('/');
  await page.context().storageState({ path: 'e2e/.auth/user.json' });
});
```

Use a dedicated test account per environment; credentials come from CI secrets, never the repo.

**Keep sockets out of component tests.** Add this to `src/test/setup.ts`, so rendering a signed-in route doesn't open a real WebSocket. Real-time behaviour is covered by the unit tests above and by E2E.

```ts
vi.mock('@/app/realtime', () => ({
  startRealtime: () => () => undefined,
  useRealtimeStatus: () => 'idle',
}));
```

## Security checklist

Review this list in any PR that touches auth, cookies or sockets. Most items are server-side, but front-end engineers should check they're in place, because the front-end design above depends on them.

- [ ] Session cookies are `HttpOnly; Secure`, `access` is short-lived, `refresh` is path-scoped and rotated on use.
- [ ] No token, password or session data in `localStorage`, `sessionStorage`, URLs or logs.
- [ ] Every write checks the `X-CSRF-Token` header against the cookie; `SameSite` is a second layer, not the only one.
- [ ] Login returns one generic error and is rate-limited per account and per IP.
- [ ] `redirect` parameters pass through `safeRedirect` — no open redirects.
- [ ] Logout revokes the refresh token server-side and the client reloads, clearing all cached data.
- [ ] The WebSocket upgrade checks the `Origin` header (cookies are sent automatically, so without this any site can open a socket as the user) and re-validates the session.
- [ ] Every real-time payload is validated with the feature's Zod schema before it touches the cache.
- [ ] UI permission checks (`useCan`) are mirrored by server authorization on every endpoint.
- [ ] A strict Content Security Policy (`fe-security`) limits `connect-src` to your API and socket origins.
