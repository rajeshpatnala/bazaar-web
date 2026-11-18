# Browser Mode, visual regression and E2E

Vitest projects, screenshot tests and rules, Playwright fixtures, tags and sharding.

## Contents

- Browser Mode and visual regression
- End-to-end tests with Playwright

## Browser Mode and visual regression

Visual tests catch what behaviour tests can't — a button that turned the wrong colour, a lost focus ring, a layout that broke in dark mode. They are noisy unless tightly controlled, so this setup follows the [Vitest guidance](https://vitest.dev/guide/browser/visual-regression-testing):

- Use them only for design-system components.
- Keep them in a separate project with a `.vrt.test.tsx` suffix.
- Fix the viewport size.
- Generate the reference screenshots only in CI.

```bash
pnpm add -D @vitest/browser-playwright playwright
```

**`vitest.config.ts`** — replaces `fe-repo-foundation`'s version with two projects (the Storybook addon from `fe-design-system` adds a third):

```ts
import { defaultExclude, defineConfig, mergeConfig } from 'vitest/config';
import { playwright } from '@vitest/browser-playwright';
import viteConfig from './vite.config';

process.env.TZ = 'UTC';
const vrtPattern = 'src/**/*.vrt.test.tsx';

export default mergeConfig(
  viteConfig,
  defineConfig({
    test: {
      globals: true,
      restoreMocks: true,
      unstubEnvs: true,
      coverage: {
        provider: 'v8',
        include: ['src/**/*.{ts,tsx}'],
        exclude: ['src/**/*.test.{ts,tsx}', 'src/**/*.stories.tsx', 'src/**/*.mocks.ts', 'src/test/**', 'src/main.tsx'],
      },
      projects: [
        {
          extends: true, // inherit plugins and aliases from the Vite config
          test: {
            name: 'unit',
            environment: 'jsdom',
            include: ['src/**/*.test.{ts,tsx}'],
            exclude: [vrtPattern, ...defaultExclude],
            setupFiles: ['./src/test/setup.ts'],
            env: { VITE_API_BASE_URL: 'http://localhost/api' },
          },
        },
        {
          extends: true,
          test: {
            name: 'vrt',
            include: [vrtPattern],
            setupFiles: ['./src/test/setup-browser.ts'],
            browser: {
              enabled: true,
              provider: playwright(),
              headless: true,
              instances: [{ browser: 'chromium', viewport: { width: 1280, height: 720 } }],
              expect: {
                toMatchScreenshot: {
                  comparatorName: 'pixelmatch',
                  comparatorOptions: { threshold: 0.2, allowedMismatchedPixelRatio: 0.01 },
                },
              },
            },
          },
        },
      ],
    },
  }),
);
```

Scripts: `"test:unit": "vitest --project unit"`, `"test:visual": "vitest run --project vrt"`.

**`src/test/setup-browser.ts`** — real styles, no animations, fonts loaded:

```ts
import '@/shared/styles/global.css';
import { unmountAll } from './mount';

const noMotion = document.createElement('style');
noMotion.textContent = '*, *::before, *::after { animation: none !important; transition: none !important; }';
document.head.append(noMotion);

beforeEach(async () => {
  await document.fonts.ready;
});
afterEach(() => unmountAll());
```

**`src/test/mount.tsx`** — renders a component on a clean page in the chosen theme:

```tsx
import type { ReactNode } from 'react';
import { createRoot, type Root } from 'react-dom/client';

let root: Root | null = null;

export function mount(ui: ReactNode, theme: 'light' | 'dark' = 'light'): void {
  document.documentElement.dataset.theme = theme;
  const container = document.createElement('div');
  container.style.padding = '16px'; // room for focus rings and shadows
  document.body.replaceChildren(container);
  root = createRoot(container);
  root.render(ui);
}

export function unmountAll(): void {
  root?.unmount();
  root = null;
}
```

**`ui/button/button.vrt.test.tsx`** — every variant, both themes, plus the keyboard-focus state:

```tsx
import { page, userEvent } from 'vitest/browser';
import { mount } from '@/test/mount';
import { Button } from './button';

const variants = ['primary', 'secondary', 'ghost', 'danger'] as const;

describe.each(['light', 'dark'] as const)('%s theme', (theme) => {
  test.each(variants)('%s button', async (variant) => {
    mount(<Button variant={variant}>Save changes</Button>, theme);
    const button = page.getByRole('button', { name: 'Save changes' });
    await expect.element(button).toBeVisible();
    await expect(button).toMatchScreenshot(`button-${variant}-${theme}`); // capture the element, not the page
  });
});

test('shows a visible focus ring for keyboard users', async () => {
  mount(<Button>Save changes</Button>);
  await userEvent.keyboard('{Tab}');
  await expect(page.getByRole('button', { name: 'Save changes' })).toMatchScreenshot('button-focus-visible');
});
```

**Reference screenshots: rules**

- They live in `__screenshots__/` next to each test, include the browser and OS in the file name, and are committed (use Git LFS once there are hundreds).
- **Generate and update them only in CI**, on the same Linux image that runs the tests. A reference made on a Mac fails on Linux because fonts render differently.
- Updates go through a manually triggered workflow (`workflow_dispatch`) that runs `vitest run --project vrt --update` on the PR branch and commits the result. Reviewers then look at the image diff in the PR.
- When a visual test fails, Vitest saves the reference, actual and diff images. Upload them as a CI artifact so reviewers can see what changed.
- Mask dynamic content (timestamps, avatars) with `screenshotOptions: { mask: [...] }` instead of loosening the threshold.

## End-to-end tests with Playwright

E2E tests prove the deployed system works end to end: real backend, real cookies, real CDN. They are the most expensive tests to run and maintain, so they cover only journeys whose failure would hurt the business. Each test creates its own data and cleans it up, so tests run in parallel and never depend on each other.

**What belongs in E2E**

| Do test | Don't test (cheaper layers cover it) |
| --- | --- |
| Sign in, sign out, session expiry | Every form validation message |
| Create, edit, cancel an order end to end | Every filter combination |
| Checkout and payment (against the provider's test mode) | Visual details of components |
| Permission boundaries (a viewer can't reach admin) | Error-message wording |

**Test data: seed through the API, never through the UI.** A test-support endpoint, enabled only in non-production environments and protected by a CI secret, creates data in milliseconds. Clicking through forms to set up a test is slow and fragile.

**`e2e/fixtures.ts`** — a fixture that creates orders and always cleans them up, even when the test fails:

```ts
import { test as base, expect } from '@playwright/test';

interface SeededOrder {
  id: string;
  customerName: string;
}

const seedHeaders = { 'X-Test-Token': process.env.E2E_SEED_TOKEN ?? '' };

export const test = base.extend<{ seedOrder: (overrides?: Partial<SeededOrder>) => Promise<SeededOrder> }>({
  seedOrder: async ({ request }, use, testInfo) => {
    const created: string[] = [];
    await use(async (overrides = {}) => {
      const res = await request.post('/api/test-support/orders', {
        headers: seedHeaders,
        data: { customerName: `E2E ${testInfo.testId}`, status: 'paid', ...overrides }, // unique per test
      });
      expect(res.ok()).toBe(true);
      const order = (await res.json()) as SeededOrder;
      created.push(order.id);
      return order;
    });
    for (const id of created) {
      await request.delete(`/api/test-support/orders/${id}`, { headers: seedHeaders });
    }
  },
});

export { expect };
```

**`e2e/orders.spec.ts`**

```ts
import { test, expect } from './fixtures';

test('support agent cancels a paid order', { tag: '@smoke' }, async ({ page, seedOrder }) => {
  const order = await seedOrder();

  await page.goto(`/orders/${order.id}`);
  await page.getByRole('button', { name: 'Cancel order' }).click();

  await expect(page.getByText('Cancelled')).toBeVisible(); // web-first assertion: retries until true
  await page.reload();
  await expect(page.getByText('Cancelled')).toBeVisible(); // persisted on the server, not just optimistic
});
```

**E2E rules**

- Use `getByRole` and `getByLabel` locators and web-first assertions (`await expect(locator).toBeVisible()`), which wait automatically. Never use `page.waitForTimeout`.
- Don't mock your own API in E2E; that turns it into a slow component test. Mock only third parties you can't control (`page.route('https://maps.example.com/**', …)`).
- Prefer fixtures and small helper functions over large page-object classes. Introduce a page object only for a screen used by many tests.
- Sign in once per run with a setup project and `storageState` (`fe-auth-realtime`). Only the auth tests use the login form.
- Tag the fastest critical tests `@smoke`. They run on every PR preview (`--grep @smoke`); the full suite runs in the merge queue and nightly.

**Shard in CI** so the full suite stays under about 15 minutes as it grows:

```yaml
e2e:
  strategy:
    fail-fast: false
    matrix:
      shard: [1, 2, 3, 4]
  steps:
    # … checkout, pnpm, node, install, playwright install --with-deps
    - run: pnpm exec playwright test --shard=${{ matrix.shard }}/4 --reporter=blob
      env:
        E2E_BASE_URL: ${{ needs.deploy-preview.outputs.url }}
        E2E_SEED_TOKEN: ${{ secrets.E2E_SEED_TOKEN }}
    - uses: actions/upload-artifact@v4
      if: ${{ !cancelled() }}
      with:
        name: blob-report-${{ matrix.shard }}
        path: blob-report
        retention-days: 7
```

A follow-up job downloads all `blob-report-*` artifacts and runs `playwright merge-reports --reporter html` to produce one report, with traces for every failure.
