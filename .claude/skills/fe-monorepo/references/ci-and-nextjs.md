# CI, deploys, Next.js app and checklist

Affected runs, remote cache, per-app deploys, Next.js 16 app setup and the new package/app checklist.

## Contents

- CI and deploys
- Adding a Next.js app for public pages
- Checklist: adding a package or app

## CI and deploys

CI cost should scale with the size of the change, not the size of the repo. On PRs, Turborepo runs tasks only for the packages a change touches and everything that depends on them. A shared remote cache replays results that any machine has already computed. Each app then deploys on its own, and only when it changed.

**CI changes** (the jobs from `fe-cicd-observability`, adapted):

```yaml
- uses: actions/checkout@v5
  with:
    fetch-depth: 0 # --affected compares against the base branch, so it needs history
- uses: pnpm/action-setup@v4
- uses: actions/setup-node@v5
  with:
    node-version-file: .nvmrc
    cache: pnpm
- run: pnpm install --frozen-lockfile
- run: pnpm format:check
- name: Affected checks (PRs)
  if: github.event_name == 'pull_request'
  run: pnpm turbo run typecheck lint test build --affected
  env:
    TURBO_TOKEN: ${{ secrets.TURBO_TOKEN }} # remote cache
    TURBO_TEAM: ${{ vars.TURBO_TEAM }}
- name: Everything (merge queue) — fast anyway thanks to the cache
  if: github.event_name == 'merge_group'
  run: pnpm turbo run typecheck lint test build
  env:
    TURBO_TOKEN: ${{ secrets.TURBO_TOKEN }}
    TURBO_TEAM: ${{ vars.TURBO_TEAM }}
```

**Remote cache** — use Vercel's hosted cache or a self-hosted one (any server implementing the Turborepo remote-cache API, backed by S3 or similar). Give CI a read-write token and engineers read-only tokens, so a laptop can't poison the shared cache.

**Debugging cache misses** — `pnpm turbo run build --dry=json` prints each task's inputs, environment variables and hash without running anything. Compare two runs to see exactly what changed. The usual culprits are an environment variable missing from `env`, or a generated file that isn't excluded from `inputs`.

**Deploy only what changed** — `.github/workflows/deploy.yml` on push to `develop` (staging) and `main` (production, reached only by promotion):

```yaml
jobs:
  changes:
    runs-on: ubuntu-latest
    outputs:
      dashboard: ${{ steps.dashboard.outputs.changed }}
      marketing: ${{ steps.marketing.outputs.changed }}
    steps:
      - uses: actions/checkout@v5
        with:
          fetch-depth: 2
      # turbo-ignore exits 0 when nothing affecting the app changed since the previous commit
      - id: dashboard
        run: npx turbo-ignore dashboard && echo "changed=false" >> "$GITHUB_OUTPUT" || echo "changed=true" >> "$GITHUB_OUTPUT"
      - id: marketing
        run: npx turbo-ignore marketing && echo "changed=false" >> "$GITHUB_OUTPUT" || echo "changed=true" >> "$GITHUB_OUTPUT"

  deploy-dashboard:
    needs: changes
    if: needs.changes.outputs.dashboard == 'true'
    # `fe-cicd-observability`'s build → staging → canary → production flow, building with:
    #   pnpm turbo run build --filter=dashboard

  deploy-marketing:
    needs: changes
    if: needs.changes.outputs.marketing == 'true'
    # Next.js deploy: your platform's integration, or a container built from `output: 'standalone'`
```

A change to `packages/ui` correctly marks **both** apps as changed, because both depend on it.

**Local development**

| Task | Command |
| --- | --- |
| Run one app | `pnpm turbo run dev --filter=dashboard` |
| Run Storybook for the design system | `pnpm --filter @acme/ui storybook` |
| Check everything a change affects before pushing | `pnpm turbo run typecheck lint test --affected` |
| Add a dependency to one package | `pnpm --filter dashboard add date-fns` |

## Adding a Next.js app for public pages

Public pages — marketing, pricing, docs, blog, product listings — need search-engine visibility and fast first loads on unknown devices. Server rendering delivers both, because the HTML arrives complete and the LCP element is in it. Use Next.js for these. Keep the signed-in product as the Vite SPA, where SEO doesn't matter and client-side navigation shines. Both apps share `@acme/ui` and `@acme/api-client`.

| Page type | App | Rendering |
| --- | --- | --- |
| Marketing, pricing, docs, blog | `marketing` (Next.js) | Static, regenerated on a schedule |
| Public product or listing pages | `marketing` (Next.js) | Server-rendered, with cached data |
| Signed-in dashboard and tools | `dashboard` (Vite SPA) | Client-rendered (`fe-repo-foundation`, `fe-app-architecture`, `fe-auth-realtime`, `fe-design-system`, `fe-testing`, `fe-performance`, `fe-cicd-observability`, `fe-team-workflow`) |

**`apps/marketing/next.config.ts`** — Next.js 16, where Turbopack is the default bundler and the React Compiler option is stable:

```ts
import type { NextConfig } from 'next';

const nextConfig: NextConfig = {
  reactCompiler: true, // needs babel-plugin-react-compiler as a devDependency
  transpilePackages: ['@acme/ui', '@acme/api-client'], // compile our TypeScript-source packages
  poweredByHeader: false,
  images: { formats: ['image/avif', 'image/webp'] },
  output: 'standalone', // only if you deploy as a container
};

export default nextConfig;
```

**`apps/marketing/app/layout.tsx`**

```tsx
import type { Metadata } from 'next';
import type { ReactNode } from 'react';
import '@acme/ui/styles.css'; // the same tokens and base styles as the dashboard

export const metadata: Metadata = {
  metadataBase: new URL('https://www.acme.com'),
  title: { default: 'Acme', template: '%s · Acme' },
  description: 'Order management for growing teams.',
};

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
```

**`apps/marketing/app/pricing/page.tsx`** — a server component: data is fetched and validated on the server, cached, and refreshed hourly:

```tsx
import type { Metadata } from 'next';
import { Plans } from '@acme/api-client';
import { BillingToggle } from './billing-toggle';

export const metadata: Metadata = { title: 'Pricing', description: 'Simple plans for every team size.' };

async function getPlans() {
  const res = await fetch(`${process.env.API_URL}/plans`, { next: { revalidate: 3600 } });
  if (!res.ok) throw new Error(`GET /plans failed: ${res.status}`);
  return Plans.parse(await res.json()); // same schema the dashboard uses
}

export default async function PricingPage() {
  const plans = await getPlans();
  return (
    <main>
      <h1>Pricing</h1>
      <BillingToggle plans={plans} /> {/* the only interactive part ships JavaScript */}
    </main>
  );
}
```

```tsx
// apps/marketing/app/pricing/billing-toggle.tsx
'use client';

import { useState } from 'react';
import { Button } from '@acme/ui';
import type { Plan } from '@acme/api-client';

export function BillingToggle({ plans }: { plans: readonly Plan[] }) {
  const [yearly, setYearly] = useState(false);
  // … render plans with monthly or yearly prices
  return <Button variant="secondary" onClick={() => setYearly((y) => !y)}>{yearly ? 'Show monthly' : 'Show yearly'}</Button>;
}
```

**Rules for the Next.js app**

- **Server components by default; `'use client'` only on interactive leaves.** Every client component ships its JavaScript, so keep them small and low in the tree.
- **Mark `@acme/ui` components that use state, effects or refs with `'use client'`** at the top of their file. Vite ignores the directive, and it makes those components usable from Next.js server components.
- **Keep secrets on the server:** import `server-only` in modules that read secret environment variables, so an accidental client import fails the build. Only `NEXT_PUBLIC_` variables reach the browser.
- **`params` and `searchParams` are promises in Next.js 16;** `await` them in pages and layouts.
- **SEO basics are code, not afterthoughts:** `generateMetadata` for dynamic pages, `app/sitemap.ts` and `app/robots.ts`, canonical URLs, and Open Graph images.
- **Security headers** go in `next.config.ts` `headers()` or in `proxy.ts`, which replaces `middleware.ts` in Next.js 16. Use the same CSP as `fe-security`.
- **The same quality gates apply:** shared ESLint and TypeScript configs, Vitest for components, Playwright for journeys, and Lighthouse CI on key public pages (`fe-performance`). The budgets matter even more here, because these pages are many visitors' first impression.

## Checklist: adding a package or app

**New shared package**

- [ ] At least two apps need it today (otherwise keep the code in the one app that does).
- [ ] Lives in `packages/<name>`, named `@acme/<name>`, `private: true`.
- [ ] Public API declared in `exports`; nothing else importable.
- [ ] Ships TypeScript source unless a Node process or npm publish consumes it.
- [ ] Extends `@acme/config-typescript` and `@acme/config-eslint`; has `typecheck`, `lint` and `test` scripts.
- [ ] React and other singletons are `peerDependencies` using `catalog:` versions.
- [ ] Respects the allowed-dependency table; added to CODEOWNERS with an owning team.
- [ ] Added to `transpilePackages` in Next.js apps that use it.

**New app**

- [ ] Rendering choice recorded in an ADR (SPA or server-rendered, and why).
- [ ] Lives in `apps/<name>` with its own CODEOWNERS entry, runtime config and deploy job gated by `turbo-ignore`.
- [ ] Uses `@acme/ui`, `@acme/api-client` and the shared configs; no code copied from another app.
- [ ] Has its own `build`, `typecheck`, `lint`, `test` and `dev` scripts, wired into `turbo.json` via the shared task names.
- [ ] Every build-time environment variable is listed in `turbo.json` `env`.
- [ ] Has its own Sentry project or environment, RUM tagging and alerts (`fe-cicd-observability`), and Lighthouse CI on key pages (`fe-performance`).
