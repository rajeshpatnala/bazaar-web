# Auth model, HTTP client and auth feature

Backend contract, session-expiry flow, final `http.ts`, session/permission APIs and login/logout.

## Contents

- Auth model and backend contract
- Session expiry at a glance
- HTTP client with CSRF and token refresh
- Auth feature

## Auth model and backend contract

The browser never sees a token. The server issues `HttpOnly` cookies and the front end only ever knows "who is signed in" from `GET /auth/me`. An injected script therefore can't steal a session, and signing out is a server call, not a `localStorage` wipe.

This playbook builds on `fe-repo-foundation` (`http.ts`) and `fe-app-architecture` (router, query client, `features/` layout).

**Cookies the server sets**

| Cookie | Flags | Lifetime | Purpose |
| --- | --- | --- | --- |
| `access` | `HttpOnly; Secure; SameSite=Lax; Path=/api` | 15 min | Authenticates API calls |
| `refresh` | `HttpOnly; Secure; SameSite=Strict; Path=/api/auth/refresh` | 7–30 days, rotated on use | Gets a new `access` cookie |
| `csrf_token` | `Secure; SameSite=Lax` (readable by JS) | Session | Echoed in the `X-CSRF-Token` header on writes |

**Endpoints**

| Endpoint | Success | Failure |
| --- | --- | --- |
| `POST /auth/login` `{ email, password }` | `200` + session JSON, sets cookies | `401` (one generic message), `429` when rate-limited |
| `POST /auth/refresh` | `204`, rotates `access` + `refresh` | `401` → user must sign in again |
| `POST /auth/logout` | `204`, clears cookies, revokes refresh token | — |
| `GET /auth/me` | `200` `{ user, permissions }` | `401` when signed out |

Why this design: a short-lived `access` cookie limits damage if one leaks, the `refresh` cookie is only ever sent to one path, and rotation lets the server detect a stolen refresh token being reused. Serve the API on the same site as the app (or proxy it, as in `fe-repo-foundation`'s Vite config) so `SameSite` cookies work without CORS credentials.

## Session expiry at a glance

*Diagram: session expiry flow · 401, single-flight refresh, retry or sign-out*

An expired access cookie costs the user nothing: the client refreshes once, shared across all failing requests, and retries. Only when the refresh itself fails does the app clear its cache and send the user to sign in, then back to the same page.

## HTTP client with CSRF and token refresh

The client handles session expiry once, centrally. On a `401` it refreshes the session and retries the request one time. Parallel requests that fail together share a single refresh call ("single-flight"), so ten failing requests never trigger ten refreshes. If refresh fails, it announces `session-expired` and the app decides what to do (next sections).

**`src/shared/lib/http.ts`** — replaces the `fe-repo-foundation` version (the public API is unchanged):

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

/** Fired when the session can't be refreshed. The app subscribes in auth-sync.ts. */
export const authEvents = new EventTarget();

const DEFAULT_TIMEOUT_MS = 15_000;
const UNSAFE_METHODS = new Set(['POST', 'PUT', 'PATCH', 'DELETE']);
const NO_REFRESH_PATHS = new Set(['/auth/login', '/auth/refresh']);

function readCookie(name: string): string | undefined {
  const entry = document.cookie.split('; ').find((c) => c.startsWith(`${name}=`));
  return entry ? decodeURIComponent(entry.slice(name.length + 1)) : undefined;
}

function csrfHeader(method: string): Record<string, string> {
  const token = UNSAFE_METHODS.has(method) ? readCookie('csrf_token') : undefined;
  return token ? { 'X-CSRF-Token': token } : {};
}

function parseBody(text: string): unknown {
  if (!text) return null;
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

let refreshInFlight: Promise<boolean> | null = null;

/** Single-flight: concurrent callers await the same refresh request. */
function refreshSession(): Promise<boolean> {
  refreshInFlight ??= fetch(`${env.VITE_API_BASE_URL}/auth/refresh`, {
    method: 'POST',
    credentials: 'include',
    headers: csrfHeader('POST'),
  })
    .then((res) => res.ok)
    .catch(() => false)
    .finally(() => {
      refreshInFlight = null;
    });
  return refreshInFlight;
}

async function send(method: string, path: string, body: unknown, opts: RequestOptions): Promise<Response> {
  const timeout = AbortSignal.timeout(opts.timeoutMs ?? DEFAULT_TIMEOUT_MS);
  return fetch(`${env.VITE_API_BASE_URL}${path}`, {
    method,
    signal: opts.signal ? AbortSignal.any([opts.signal, timeout]) : timeout,
    credentials: 'include',
    headers: {
      Accept: 'application/json',
      ...(body === undefined ? {} : { 'Content-Type': 'application/json' }),
      'X-Request-Id': crypto.randomUUID(),
      ...csrfHeader(method),
      ...opts.headers,
    },
    body: body === undefined ? null : JSON.stringify(body),
  });
}

async function request(method: string, path: string, body: unknown, opts: RequestOptions = {}): Promise<unknown> {
  let res = await send(method, path, body, opts);

  if (res.status === 401 && !NO_REFRESH_PATHS.has(path)) {
    if (await refreshSession()) {
      res = await send(method, path, body, opts); // retry exactly once
    } else {
      authEvents.dispatchEvent(new Event('session-expired'));
    }
  }

  const data = parseBody(await res.text());
  if (!res.ok) throw new HttpError(res.status, data, `${env.VITE_API_BASE_URL}${path}`);
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

Details that matter:

- **Retry once, never loop.** A second `401` after a successful refresh is a real authorization failure and surfaces as an error.
- **Login and refresh are excluded** from the retry, or a wrong password would trigger a refresh.
- **CSRF header only on writes,** read from the non-`HttpOnly` cookie (double-submit pattern). The server compares the header with the cookie.
- **Retrying a write after refresh is safe** because the first attempt was rejected before it ran. For payments, also send an `Idempotency-Key` header so a retry can never charge twice.

## Auth feature

The session is ordinary server state: one query (`sessionQuery`) that returns the session or `null`. Every "am I signed in / may I do this" check reads from it, so there's no separate auth store to keep in sync.

**`features/auth/model/session.ts`**

```ts
import { z } from 'zod';

export const Session = z.object({
  user: z.object({ id: z.string(), name: z.string(), email: z.email() }),
  permissions: z.array(z.string()), // unknown future permissions must not break parsing
});
export type Session = z.infer<typeof Session>;

export const LoginInput = z.object({
  email: z.email('Enter a valid email'),
  password: z.string().min(1, 'Enter your password'),
});
export type LoginInput = z.infer<typeof LoginInput>;

/** Permissions the front end knows about. The server remains the enforcer. */
export type Permission = 'orders:read' | 'orders:create' | 'orders:cancel' | 'admin:access';

export function can(session: Session | null | undefined, permission: Permission): boolean {
  return session?.permissions.includes(permission) ?? false;
}

/** Only same-origin relative paths: blocks open redirects like ?redirect=https://evil.com or //evil.com. */
export function safeRedirect(target: string | undefined, fallback = '/'): string {
  if (!target?.startsWith('/') || target.startsWith('//') || target.startsWith('/\\')) return fallback;
  return target;
}
```

**`features/auth/api/auth-api.ts`**

```ts
import { queryOptions, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { http, HttpError } from '@/shared/lib/http';
import { broadcastAuth } from '@/shared/lib/auth-channel';
import { can, Session, type LoginInput, type Permission } from '../model/session';

export const sessionQuery = queryOptions({
  queryKey: ['session'],
  queryFn: async ({ signal }): Promise<Session | null> => {
    try {
      return Session.parse(await http.get('/auth/me', { signal }));
    } catch (error) {
      if (error instanceof HttpError && error.status === 401) return null; // signed out is a state, not an error
      throw error;
    }
  },
  staleTime: 5 * 60_000,
});

export function useSession(): Session | null {
  return useQuery(sessionQuery).data ?? null;
}

export function useCan(permission: Permission): boolean {
  return can(useSession(), permission);
}

export function useLogin() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (input: LoginInput) => Session.parse(await http.post('/auth/login', input)),
    onSuccess: (session) => {
      queryClient.setQueryData(sessionQuery.queryKey, session);
      broadcastAuth('login');
    },
  });
}

export function useLogout() {
  return useMutation({
    mutationFn: () => http.post('/auth/logout', {}),
    onSettled: () => {
      broadcastAuth('logout');
      // A full reload wipes every in-memory cache and store: no previous user's data can leak.
      window.location.assign('/login');
    },
  });
}
```

**`src/shared/lib/auth-channel.ts`** — tells other open tabs about sign-in and sign-out:

```ts
export type AuthBroadcast = 'login' | 'logout';

const channel = typeof BroadcastChannel === 'undefined' ? null : new BroadcastChannel('auth');

export function broadcastAuth(event: AuthBroadcast): void {
  channel?.postMessage(event);
}

export function onAuthBroadcast(handler: (event: AuthBroadcast) => void): () => void {
  const listener = (e: MessageEvent<unknown>) => {
    if (e.data === 'login' || e.data === 'logout') handler(e.data);
  };
  channel?.addEventListener('message', listener);
  return () => channel?.removeEventListener('message', listener);
}
```

**`features/auth/index.ts`**

```ts
export { sessionQuery, useCan, useLogin, useLogout, useSession } from './api/auth-api';
export { LoginInput, can, safeRedirect, type Permission, type Session } from './model/session';
export { LoginForm } from './components/login-form';
```

**Permission checks in the UI** hide what the user can't do; they are not security. The API must reject the call anyway.

```tsx
const mayCancel = useCan('orders:cancel');
{mayCancel && canCancel(order) && <button type="button" onClick={() => cancel.mutate(order.id)}>Cancel order</button>}
```
