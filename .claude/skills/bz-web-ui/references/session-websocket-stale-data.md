# 5. Session, WebSocket, stale data

(From `bz-web-ui` SKILL.md; open when the task touches this area.)


**Session** (`app/SessionGate`): the owner gets in through the one-time link that `just run` prints and opens (`/auth?token=...`, handled by the server, see `bz-security`); the UI has no password or login form. On start the gate loads `GET /auth/me` (CSRF token kept in memory, mode, live flag). Without a session it renders the session-ended screen: plain text saying the server was restarted or the one-time link was already used, and to restart the server (`just run`), which prints a fresh link. No retry loop, no token field, nothing stored.

**WebSocket** (`api/ws.ts`, one socket per app, opened once `GET /auth/me` succeeds, at `/api/v1/ws` with `ws://` derived from `location`):

- Messages are `{ type, data }` typed as `components["schemas"]["WsOut"]` (generated): `tick`, `data_status`, `order_update`, `gate_alert`, `job_progress` (updates the backtest job query cache), `resync` (invalidates the query groups it names, e.g. after the Mac wakes); parse once at the edge, ignore unknown types.
- Reconnect with exponential backoff and jitter (for example 1s doubling to 30s, reset after a stable connection); after any abnormal close, call `GET /api/v1/auth/me` first: 401 stops reconnecting and shows the session-ended screen, anything else keeps backing off (there is no auth close code). Pause while `document.hidden` and reconnect on `visibilitychange`.
- On every open, resubscribe to what the screens currently need (subscription set lives in the store, not in components).
- Status machine `connecting | open | reconnecting | closed` drives a shell-level `DataDelayedBadge`: shown when disconnected, or when `now - asOf` exceeds the threshold supplied by the server status, never a constant in the UI. While disconnected, prices are labelled stale, not hidden.
- Ticks write into a zustand `tickStore` keyed by symbol (latest only); components select a single symbol. Never one React update per raw tick. Chart updates are throttled: coalesce to at most one `series.update` per animation frame (or about 250 ms), and always apply the latest bar.
- `order_update` and `gate_alert` invalidate the approval and positions queries and raise an alert; they never trigger an approval.
