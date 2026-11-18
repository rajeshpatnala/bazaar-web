# Env config and HTTP client

Zod-validated `env.ts`, `vite-env.d.ts` and the first `http.ts` with timeouts and `HttpError`.

## Typed env config and HTTP client

The app reads configuration from one validated module and talks to the network through one client; nothing else touches `import.meta.env` or calls `fetch` directly.

**`.env.example`** — committed; real `.env.local` files are git-ignored. Everything prefixed `VITE_` is public, so never put secrets here.

```bash
VITE_API_BASE_URL=/api
VITE_SENTRY_DSN=
VITE_RELEASE=dev
```

**`src/vite-env.d.ts`** — types for `import.meta.env`:

```ts
/// <reference types="vite/client" />
interface ImportMetaEnv {
  readonly VITE_API_BASE_URL?: string;
  readonly VITE_SENTRY_DSN?: string;
  readonly VITE_RELEASE?: string;
}
```

**`src/shared/config/env.ts`** — fail at startup, not on the first API call:

```ts
import { z } from 'zod';

const EnvSchema = z.object({
  MODE: z.string(),
  VITE_API_BASE_URL: z.string().min(1).default('/api'),
  VITE_SENTRY_DSN: z.url().optional(),
  VITE_RELEASE: z.string().default('dev'),
});

const parsed = EnvSchema.safeParse(import.meta.env);
if (!parsed.success) {
  throw new Error(`Invalid environment variables:\n${z.prettifyError(parsed.error)}`);
}

export const env = parsed.data;
export const isProd = env.MODE === 'production';
```

If the same build is promoted across environments ("build once, deploy many"), load these values from a `/config.json` at startup instead of baking them in; the schema stays the same. `fe-cicd-observability` covers that pattern.

**`src/shared/lib/http.ts`**

```ts
import { env } from '@/shared/config/env';

export class HttpError extends Error {
  readonly status: number;
  readonly body: unknown;
  constructor(status: number, body: unknown, url: string) {
    super(`HTTP ${status} for ${url}`);
    this.name = 'HttpError';
    this.status = status;
    this.body = body;
  }
}

export interface RequestOptions {
  signal?: AbortSignal;
  timeoutMs?: number;
  headers?: Record<string, string>;
}

const DEFAULT_TIMEOUT_MS = 15_000;

function parseBody(text: string): unknown {
  if (!text) return null;
  try {
    return JSON.parse(text);
  } catch {
    return text; // e.g. an HTML error page from a proxy
  }
}

async function request(method: string, path: string, body: unknown, opts: RequestOptions = {}): Promise<unknown> {
  const url = `${env.VITE_API_BASE_URL}${path}`;
  const timeout = AbortSignal.timeout(opts.timeoutMs ?? DEFAULT_TIMEOUT_MS);
  const signal = opts.signal ? AbortSignal.any([opts.signal, timeout]) : timeout;

  const res = await fetch(url, {
    method,
    signal,
    credentials: 'include', // session cookie; no tokens in JavaScript
    headers: {
      Accept: 'application/json',
      ...(body === undefined ? {} : { 'Content-Type': 'application/json' }),
      'X-Request-Id': crypto.randomUUID(), // correlates browser errors with server logs
      ...opts.headers,
    },
    body: body === undefined ? null : JSON.stringify(body),
  });

  const data = parseBody(await res.text());
  if (!res.ok) throw new HttpError(res.status, data, url);
  return data;
}

export const http = {
  get: (path: string, opts?: RequestOptions) => request('GET', path, undefined, opts),
  post: (path: string, body: unknown, opts?: RequestOptions) => request('POST', path, body, opts),
  put: (path: string, body: unknown, opts?: RequestOptions) => request('PUT', path, body, opts),
  patch: (path: string, body: unknown, opts?: RequestOptions) => request('PATCH', path, body, opts),
  delete: (path: string, opts?: RequestOptions) => request('DELETE', path, undefined, opts),
};
```

Design choices:

- **Returns `unknown`.** Callers must parse with a Zod schema (see the `useOrder` example in `fe-app-architecture`), so a backend change surfaces as one clear parse error.
- **Timeout plus caller cancellation.** TanStack Query passes its `signal`; leaving a page aborts in-flight requests.
- **No retries here.** TanStack Query owns retry policy, so retries never double up (`fe-app-architecture` configures it).
- **Errors carry status and body,** so the UI can show "card declined" (402) differently from "something went wrong" (500).
