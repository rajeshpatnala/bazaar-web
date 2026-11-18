# Test setup and verification

Vitest config, MSW server and handlers, render helper, first tests, Playwright config, smoke spec, and the verify table.

## Contents

- Test setup
- Verify the setup

## Test setup

Unit and component tests run in Vitest with jsdom, with every network call answered by MSW; E2E tests run the production build in real browsers with Playwright. Unhandled requests fail the test, so no test silently hits a real server. (Vitest Browser Mode for visual-regression tests is added in `fe-testing`.)

**`vitest.config.ts`** — reuses the Vite config so aliases and plugins match the app:

```ts
import { defineConfig, mergeConfig } from 'vitest/config';
import viteConfig from './vite.config';

export default mergeConfig(
  viteConfig,
  defineConfig({
    test: {
      environment: 'jsdom',
      globals: true,
      include: ['src/**/*.test.{ts,tsx}'],
      setupFiles: ['./src/test/setup.ts'],
      env: { VITE_API_BASE_URL: 'http://localhost/api' }, // Node's fetch needs absolute URLs
      restoreMocks: true,
      unstubEnvs: true,
      coverage: {
        provider: 'v8',
        include: ['src/**/*.{ts,tsx}'],
        exclude: ['src/**/*.test.{ts,tsx}', 'src/**/*.stories.tsx', 'src/test/**', 'src/main.tsx'],
        // Enable once real features exist; a floor against big drops, not a goal:
        // thresholds: { lines: 70, branches: 65 },
      },
    },
  }),
);
```

**`src/test/setup.ts`**

```ts
import '@testing-library/jest-dom/vitest';
import { cleanup } from '@testing-library/react';
import { server } from './server';

beforeAll(() => server.listen({ onUnhandledRequest: 'error' }));
afterEach(() => {
  server.resetHandlers(); // drop per-test overrides
  cleanup();
});
afterAll(() => server.close());
```

**`src/test/server.ts`** and **`src/test/handlers.ts`** — default happy-path responses; tests override them with `server.use(...)` for error cases:

```ts
// server.ts
import { setupServer } from 'msw/node';
import { handlers } from './handlers';

export const server = setupServer(...handlers);
```

```ts
// handlers.ts — the leading * matches any origin
import { http, HttpResponse } from 'msw';

export const handlers = [
  http.get('*/api/me', () => HttpResponse.json({ id: 'u_1', name: 'Test User' })),
];
```

**`src/test/render.tsx`** — renders a component with the same providers the app uses (`fe-app-architecture` adds the router):

```tsx
import { render, type RenderOptions } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ReactElement, ReactNode } from 'react';

export function createTestQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: Infinity }, mutations: { retry: false } },
  });
}

export function renderWithProviders(ui: ReactElement, options?: RenderOptions) {
  const queryClient = createTestQueryClient(); // fresh cache per test: no leaks between tests
  function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>;
  }
  return { queryClient, ...render(ui, { wrapper: Wrapper, ...options }) };
}
```

**`src/shared/lib/http.test.ts`** — the first test, proving the setup works end to end:

```ts
import { http as mock, HttpResponse } from 'msw';
import { server } from '@/test/server';
import { http, HttpError } from './http';

test('returns parsed JSON', async () => {
  server.use(mock.get('*/api/ping', () => HttpResponse.json({ ok: true })));
  await expect(http.get('/ping')).resolves.toEqual({ ok: true });
});

test('throws HttpError with status and body', async () => {
  server.use(mock.get('*/api/ping', () => HttpResponse.json({ code: 'declined' }, { status: 402 })));
  const error = await http.get('/ping').catch((e: unknown) => e);
  expect(error).toBeInstanceOf(HttpError);
  expect(error).toMatchObject({ status: 402, body: { code: 'declined' } });
});
```

**`playwright.config.ts`**

```ts
import { defineConfig, devices } from '@playwright/test';

const CI = Boolean(process.env.CI);
const baseURL = process.env.E2E_BASE_URL ?? 'http://localhost:4173';

export default defineConfig({
  testDir: './e2e',
  fullyParallel: true,
  forbidOnly: CI, // a stray test.only fails CI
  retries: CI ? 2 : 0, // retried passes are reported as flaky — track and fix them
  reporter: CI ? [['github'], ['html', { open: 'never' }]] : 'list',
  use: { baseURL, trace: 'on-first-retry', screenshot: 'only-on-failure' },
  projects: [
    { name: 'chromium', use: { ...devices['Desktop Chrome'] } },
    { name: 'firefox', use: { ...devices['Desktop Firefox'] } },
    { name: 'webkit', use: { ...devices['Desktop Safari'] } },
    { name: 'mobile', use: { ...devices['Pixel 7'] } },
  ],
  // Test the production build locally; against a preview URL in CI, set E2E_BASE_URL instead.
  webServer: process.env.E2E_BASE_URL
    ? undefined
    : { command: 'pnpm build && pnpm preview --port 4173', url: baseURL, reuseExistingServer: !CI },
});
```

**`e2e/smoke.spec.ts`**

```ts
import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';

test('home page loads and is accessible', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
  const { violations } = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag22aa']).analyze();
  expect(violations).toEqual([]);
});
```

Run `pnpm exec playwright install --with-deps` once per machine to download the browsers.

## Verify the setup

The foundation is done when these commands pass on a clean clone:

| Command | Expected result |
| --- | --- |
| `pnpm install` | Installs; Husky hooks are created |
| `pnpm verify` | Types, lint, format, 2 passing tests with coverage, production build — all green |
| `pnpm exec playwright install --with-deps && pnpm e2e` | Smoke test passes in 4 browser projects |
| `git commit -m "update stuff"` | Rejected by commitlint (no type or scope) |
| `git commit -m "chore(repo): add tooling"` | Accepted after lint-staged runs |
| Import `@/features/x/components/y` from another feature | `pnpm lint` fails with the boundary message |

Prove the gates catch failures, not only that they pass: break one thing at a time (a type error, an `any`, a failing assertion) and confirm the right command fails.

**Next in the series:** `fe-app-architecture` — App architecture (app shell, router, TanStack Query config, error boundaries, one complete feature module); `fe-auth-realtime` — Auth and real-time; `fe-design-system` — Design system and CSS; `fe-testing` — Testing in depth; `fe-performance` — Performance; `fe-cicd-observability` — CI/CD, deployment and observability.
