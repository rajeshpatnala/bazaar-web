# API client, query keys and WebSocket hook

Open when creating `web/src/api/`, adding an endpoint to a screen, or touching the socket. Endpoints, CSRF header name and WebSocket message shapes are owned by `bz-web-api` `references/api-contract.md`; this file only shows how the UI consumes them. openapi-fetch docs: https://openapi-ts.dev/openapi-fetch/ (middleware: `/openapi-fetch/middleware-auth`).

## 1. Generating types

`just openapi` exports the server's OpenAPI document and runs `openapi-typescript` into `web/src/api/schema.d.ts` (committed, never hand-edited). CI regenerates and fails on a diff, so server and UI cannot drift. Use the generated names everywhere:

```ts
import type { components } from "./schema";
export type Recommendation = components["schemas"]["RecommendationDto"];
```

## 2. Client with middleware

```ts
// api/client.ts
import createClient, { type Middleware } from "openapi-fetch";
import type { paths } from "./schema";

let csrf: string | null = null;                       // memory only, never storage
export const setCsrfToken = (t: string | null) => { csrf = t; };

const security: Middleware = {
  onRequest({ request }) {
    if (!["GET", "HEAD", "OPTIONS"].includes(request.method)) {
      if (!csrf) throw new Error("missing CSRF token");
      request.headers.set(CSRF_HEADER, csrf);         // name from api-contract.md
    }
    return request;
  },
  onResponse({ request, response }) {
    if (response.status === 401) {
      onSessionEnded();                               // clear query cache, close socket, show session-ended screen
    }
    return response;
  },
};

export const api = createClient<paths>({ baseUrl: location.origin, credentials: "same-origin" });   // paths include /api/v1
api.use(security);
```

Notes: the generated `paths` keys include `/api/v1`, so `baseUrl` is the page origin and calls use full paths; this works behind the Vite proxy, from `web/dist` and under jsdom (Node's `Request` rejects relative URLs). Middleware may need to clone a body before reading it; do not read `response.body` in middleware. Calls return `{ data, error }`: a query function throws `error` (mapped to `ApiError` from the problem+json body) so TanStack Query sees failures. Check the installed version's types for the exact middleware argument shape (it changed between 0.x releases).

## 3. Query keys and options

One key tree in `api/keys.ts` so invalidation is by prefix:

```ts
export const keys = {
  auth: { me: ["auth", "me"] as const },
  status: ["status"] as const,
  recommendations: { all: ["recommendations"] as const, one: (id: string) => ["recommendations", id] as const },
  approvals: { one: (id: string) => ["approvals", id] as const },
  candles: (symbol: string, tf: string) => ["candles", symbol, tf] as const,
  positions: ["positions"] as const,
};
```

```ts
// features/scanner/queries.ts
export const recommendationsQuery = () =>
  queryOptions({
    queryKey: keys.recommendations.all,
    queryFn: async ({ signal }) => {
      const { data, error } = await api.GET("/api/v1/recommendations", { signal });
      if (error) throw toApiError(error);
      return data;
    },
    staleTime: 60_000,
  });
```

Mutations (`order-intents`, `approve`, `reject`, `kill-switch`) set `retry: false`, disable the trigger while pending and invalidate by prefix on success. A 401 clears the whole cache.

## 4. WebSocket client

A plain class plus a React hook; no library needed. Backoff, resubscribe and status are in the class so they are unit-testable with a fake `WebSocket` and fake timers.

```ts
// api/ws.ts (shape, not complete)
type Status = "connecting" | "open" | "reconnecting" | "closed";

export class LiveSocket {
  private ws: WebSocket | null = null;
  private attempt = 0;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private subs = new Set<string>();
  constructor(private url: string, private onMessage: (m: WsMessage) => void, private onStatus: (s: Status) => void) {}

  connect() {
    this.onStatus(this.attempt ? "reconnecting" : "connecting");
    const ws = (this.ws = new WebSocket(this.url));
    ws.onopen = () => { this.attempt = 0; this.onStatus("open"); this.resubscribe(); };
    ws.onmessage = (e) => { const m = parse(e.data); if (m) this.onMessage(m); };   // unknown types ignored
    ws.onclose = (e) => {
      if (e.wasClean && e.code === 1000) return;                                   // we closed it
      void this.onAbnormalClose();   // GET /api/v1/auth/me: 401 -> session-ended screen and stop; else back off below
      const delay = Math.min(30_000, 1_000 * 2 ** this.attempt++) * (0.5 + Math.random() / 2);
      this.onStatus("reconnecting");
      this.timer = setTimeout(() => this.connect(), delay);
    };
  }
  subscribe(sym: string) { this.subs.add(sym); this.send({ type: "subscribe", data: { symbol: sym } }); }
  private resubscribe() { for (const s of this.subs) this.send({ type: "subscribe", data: { symbol: s } }); }
  close() { clearTimeout(this.timer); this.ws?.close(); }
}
```

The subscribe message shape comes from `api-contract.md`; if the server pushes a fixed set, drop the subscription code. URL: `` `${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/api/v1/ws` ``.

Hook and store rules:

- `useLiveStatus()` returns the status; the shell shows `DataDelayedBadge` unless it is `open` and the last `data_status`/tick is fresh per the server threshold.
- Incoming `tick` messages update a zustand `tickStore` (Map keyed by symbol, latest value). `useTick(symbol)` selects one symbol with a selector so unrelated symbols do not re-render.
- Chart components subscribe to the store outside React rendering (`tickStore.subscribe`) and push to the series through a throttle: keep the latest bar, flush once per `requestAnimationFrame` or about 250 ms.
- Create the socket in one provider after `auth/me` succeeds; close it on logout, on 401 and on unmount (React StrictMode mounts twice in dev: make `close()` idempotent and ignore events from a stale instance).
- Test with a fake `WebSocket` class injected into the provider, or with MSW's WebSocket interception (`ws.link`) as set up in `bz-testing` `references/ui-mocked-api.md`.
