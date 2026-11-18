# 4. API client and requests

(From `bz-web-ui` SKILL.md; open when the task touches this area.)


One client in `api/client.ts`, created with `createClient<paths>({ baseUrl: location.origin, credentials: "same-origin" })` and called with full paths (`api.GET("/api/v1/recommendations")`, because the generated `paths` include the prefix; an origin base also works under jsdom) and middleware via `client.use(...)`. Same origin in dev (Vite proxy) and in `just run` (`web/dist` served by the server on 127.0.0.1:8787), so there is no CORS and no absolute server URL in code.

- **CSRF:** middleware `onRequest` adds the CSRF header to every non-GET/HEAD request. Header name and how the token is delivered are defined in `bz-web-api` `references/api-contract.md`; keep the token in a module variable (memory), never in storage. A missing token on a mutation is a thrown error, not a silent send.
- **401:** there is no login page. Middleware `onResponse` on 401 clears the query cache, closes the socket and shows the session-ended screen; `SessionGate` does the same on first load when `GET /auth/me` fails. 403 with a CSRF problem refreshes the token once (from `/auth/me`), then surfaces the error.
- **Errors** are `application/problem+json`: show `title`/`detail` (user-safe), never the raw body or stack. Map to one `ApiError` type in `client.ts`.
- **Side effects:** `retry: false` for mutations and for `order-intents`/`approvals`. `staleTime` per screen (EOD screens minutes, live screens `0` plus WebSocket).
- Wrap each call in a `queryOptions` factory next to its feature; `api/keys.ts` holds the key tree (see reference). Components never call `fetch` directly.
