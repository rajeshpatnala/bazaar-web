# Deployment

Runtime config, startup order, deploy workflow and script, promotion, version checks and chunk-error recovery.

## Deployment

The built app is static files, so it deploys to object storage behind a CDN (or a static host such as Cloudflare, Vercel or Netlify). One artifact is built per commit to `main`, tested on staging, then canaried and promoted to production unchanged. Rollback just points back to the previous artifact.

### Runtime config instead of per-environment builds

Settings that differ per environment (API URL, Sentry environment, feature-flag key) are read at startup from `/config.json`, which each environment serves with its own values. The release id is baked into the build, because it identifies the artifact itself.

**`config/staging.json`** (committed; never contains secrets):

```json
{ "environment": "staging", "apiBaseUrl": "/api", "sentryDsn": "https://public-key@o0.ingest.sentry.io/1", "flagsClientKey": "stg-client-key" }
```

**`src/shared/config/runtime-config.ts`**

```ts
import { z } from 'zod';
import { env } from './env';

const RuntimeConfig = z.object({
  environment: z.enum(['development', 'preview', 'staging', 'production']),
  apiBaseUrl: z.string().min(1),
  sentryDsn: z.url().optional(),
  flagsClientKey: z.string().optional(),
});
export type RuntimeConfig = z.infer<typeof RuntimeConfig>;

let current: RuntimeConfig | null = null;

export async function loadRuntimeConfig(): Promise<RuntimeConfig> {
  const res = await fetch('/config.json', { cache: 'no-store' });
  if (!res.ok) throw new Error(`Could not load /config.json (HTTP ${res.status})`);
  current = RuntimeConfig.parse(await res.json()); // fail fast on a bad deploy
  return current;
}

export function runtimeConfig(): RuntimeConfig {
  if (current) return current;
  if (import.meta.env.DEV || import.meta.env.MODE === 'test') {
    return { environment: 'development', apiBaseUrl: env.VITE_API_BASE_URL }; // local dev and tests
  }
  throw new Error('Runtime config used before loadRuntimeConfig()');
}
```

In `http.ts` (`fe-repo-foundation` and `fe-auth-realtime`), replace `env.VITE_API_BASE_URL` with `runtimeConfig().apiBaseUrl`. In `index.html`, add `<link rel="preload" href="/config.json" as="fetch" crossorigin>` so the config downloads in parallel with the JavaScript and costs no extra round trip.

**`src/main.tsx` startup order**

```ts
const config = await loadRuntimeConfig(); // 1. config
initObservability(config); // 2. error tracking before anything can fail
await initFlags(config.flagsClientKey); // 3. flags, so the first render uses the right variant
const queryClient = createQueryClient(); // 4. app (`fe-app-architecture`)
const router = createAppRouter({ queryClient });
startAuthSync(queryClient, router); // `fe-auth-realtime`
startRum(router); // `fe-performance`
// … createRoot(…).render(…) as in `fe-app-architecture`
```

### Deploy workflow

**`.github/workflows/deploy.yml`**

```yaml
name: deploy
on:
  push:
    branches: [develop, main] # develop -> staging; main (reached only by promotion) -> staging, then production

permissions:
  contents: read
  id-token: write # OIDC to the cloud provider

concurrency:
  group: deploy-${{ github.ref_name }}
  cancel-in-progress: false # never interrupt a deploy half-way

jobs:
  build:
    runs-on: ubuntu-latest
    outputs:
      release: ${{ steps.meta.outputs.release }}
    steps:
      - uses: actions/checkout@v5
      # … pnpm, node, install
      - id: meta
        run: echo "release=$(git rev-parse --short=12 HEAD)" >> "$GITHUB_OUTPUT"
      - run: pnpm build
        env:
          VITE_RELEASE: ${{ steps.meta.outputs.release }}
          SENTRY_AUTH_TOKEN: ${{ secrets.SENTRY_AUTH_TOKEN }} # uploads source maps, then deletes them
      - uses: actions/upload-artifact@v4
        with:
          name: web-${{ steps.meta.outputs.release }}
          path: dist

  staging:
    needs: build
    runs-on: ubuntu-latest
    environment: staging
    steps:
      - uses: actions/checkout@v5
      - uses: actions/download-artifact@v4
        with: { name: 'web-${{ needs.build.outputs.release }}', path: dist }
      - run: ./scripts/deploy.sh staging "${{ needs.build.outputs.release }}"
      # … full Playwright suite against staging (`fe-testing`)

  production:
    needs: [build, staging]
    if: github.ref_name == 'main' # only promoted code reaches production
    runs-on: ubuntu-latest
    environment: production # approval or wait timer configured in GitHub
    steps:
      - uses: actions/checkout@v5
      - uses: actions/download-artifact@v4
        with: { name: 'web-${{ needs.build.outputs.release }}', path: dist }
      - run: ./scripts/deploy.sh production "${{ needs.build.outputs.release }}" --canary
```

### Atomic deploys, canary and rollback

**`scripts/deploy.sh`** — the example uses S3 and CloudFront; the order of steps is what matters on any host:

```bash
#!/usr/bin/env bash
set -euo pipefail
ENV="$1"; RELEASE="$2"; MODE="${3:-}"
BUCKET="s3://acme-web-$ENV"

# 1. Hashed assets first. Never delete old ones: open tabs on the previous release still load their chunks.
aws s3 sync dist/assets "$BUCKET/assets" --cache-control "public,max-age=31536000,immutable"

# 2. This environment's runtime config and the version marker.
aws s3 cp "config/$ENV.json" "$BUCKET/config.json" --cache-control "no-cache"

# 3. Keep every release's HTML addressable, for canary routing and instant rollback.
aws s3 cp dist/index.html "$BUCKET/releases/$RELEASE/index.html" --cache-control "no-cache"
aws s3 cp dist/version.json "$BUCKET/releases/$RELEASE/version.json" --cache-control "no-cache"

if [[ "$MODE" == "--canary" ]]; then
  # 4a. Canary: an edge rule serves releases/$RELEASE/index.html to 5% of users (sticky by cookie).
  ./scripts/set-canary.sh "$ENV" "$RELEASE" 5
else
  # 4b. Switch: the live index.html is replaced last, in one step.
  aws s3 cp dist/index.html "$BUCKET/index.html" --cache-control "no-cache"
  aws s3 cp dist/version.json "$BUCKET/version.json" --cache-control "no-cache"
fi
```

**Promotion and rollback**

| Step | How | Gate |
| --- | --- | --- |
| Canary 5% | Edge rule routes a sticky 5% of sessions to the new release | 30 min with no error-rate or Web Vitals regression versus the current release |
| 50% | Raise the canary weight | 30 min, same comparison |
| 100% | `deploy.sh production <release>` without `--canary` | — |
| Rollback | `deploy.sh production <previous-release>`: the previous release's assets are still there, so it takes seconds | Any gate failing; automatic if the error rate doubles |

The comparison only works because every error and metric carries the release id (Observability section).

### Tell open tabs about new releases

A tab opened yesterday still runs yesterday's code. Write the release id at build time and check it periodically.

```ts
// vite.config.ts — add to plugins: emits dist/version.json
{
  name: 'version-file',
  generateBundle() {
    this.emitFile({ type: 'asset', fileName: 'version.json', source: JSON.stringify({ release: process.env.VITE_RELEASE ?? 'dev' }) });
  },
},
```

```ts
// src/app/version-check.ts — call startVersionCheck(onNewVersion) from the root layout
import { env } from '@/shared/config/env';

export function startVersionCheck(onNewVersion: () => void): () => void {
  const check = async () => {
    try {
      const res = await fetch('/version.json', { cache: 'no-store' });
      const { release } = (await res.json()) as { release: string };
      if (release !== env.VITE_RELEASE) onNewVersion(); // show "New version available — Reload"
    } catch {
      /* offline or mid-deploy: try again next time */
    }
  };
  const onVisible = () => { if (document.visibilityState === 'visible') void check(); };
  const timer = setInterval(() => void check(), 5 * 60_000);
  document.addEventListener('visibilitychange', onVisible);
  return () => { clearInterval(timer); document.removeEventListener('visibilitychange', onVisible); };
}
```

If a lazy chunk fails to load anyway (for example, very old assets were cleaned up), Vite fires `vite:preloadError`. Reload once to recover:

```ts
window.addEventListener('vite:preloadError', (event) => {
  if (sessionStorage.getItem('reloaded-for-chunk')) return; // avoid a reload loop
  sessionStorage.setItem('reloaded-for-chunk', '1');
  event.preventDefault();
  window.location.reload();
});
```

Keep old assets for at least 30 days before any cleanup job removes them.
