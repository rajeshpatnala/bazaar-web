# Real-time

Choosing a transport, the resilient WebSocket client and connecting events to the query cache.

## Contents

- Real-time: choosing a transport
- Resilient WebSocket client
- Connecting real-time events to the query cache

## Real-time: choosing a transport

Pick the simplest transport that meets the latency need. Whichever you choose, real-time messages only **update the query cache**; components never read from a socket directly, so every screen keeps working the same way when the connection drops.

| Need | Transport | Why |
| --- | --- | --- |
| Updates every 30 s or more are fine (dashboards, reports) | Polling: `refetchInterval` on the query | No new infrastructure; pauses in background tabs |
| Server pushes events to the client (notifications, order status, live feeds) | Server-Sent Events (`EventSource`) | Plain HTTP, cookies sent automatically, built-in reconnect and resume via `Last-Event-ID` |
| Two-way and low latency (chat, presence, collaborative editing, typing indicators) | WebSocket | Full duplex; you implement reconnect and heartbeats (next section) |

**SSE in five lines** — enough for most "live status" features:

```ts
const source = new EventSource('/api/events', { withCredentials: true });
source.addEventListener('order.updated', (e) => {
  applyRealtimeEvent(queryClient, { type: 'order.updated', payload: JSON.parse(e.data) });
});
// Call source.close() on sign-out; the browser reconnects automatically after network errors.
```

**Message envelope** — agree on one shape with the backend for every transport:

```json
{ "type": "order.updated", "id": "evt_123", "payload": { "id": "ord_1", "status": "shipped" } }
```

The `type` routes the event to the owning feature, the `id` lets the client skip duplicates, and the `payload` is validated with that feature's Zod schema like any API response.

## Resilient WebSocket client

Production sockets fail in quiet ways: Wi-Fi switches, laptops sleep, proxies drop idle connections, servers restart and every client reconnects at once. The client below handles each case. It reconnects with jittered backoff, detects dead connections with heartbeats, reconnects immediately when the browser comes back online, drops duplicate events, and refreshes auth when the server closes with `4401`.

**`src/shared/lib/realtime.ts`**

```ts
import { z } from 'zod';

export const Envelope = z.object({ type: z.string(), id: z.string().optional(), payload: z.unknown() });
export type Envelope = z.infer<typeof Envelope>;
export type ConnectionStatus = 'idle' | 'connecting' | 'open' | 'reconnecting';

const UNAUTHORIZED_CLOSE_CODE = 4401; // agreed with the backend
const DEDUPE_WINDOW = 500;

/** Exponential backoff with full jitter (0–1 s, 0–2 s, 0–4 s … capped) so clients don't reconnect in lockstep. */
export function backoffDelay(attempt: number, capMs = 30_000, random: () => number = Math.random): number {
  return random() * Math.min(capMs, 1000 * 2 ** attempt);
}

function safeJson(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

interface RealtimeOptions {
  url: string;
  heartbeatMs?: number;
  onUnauthorized?: () => Promise<boolean>; // resolve true once the session is refreshed
}

export class RealtimeClient {
  private socket: WebSocket | null = null;
  private attempt = 0;
  private stopped = true;
  private reconnectTimer: ReturnType<typeof setTimeout> | undefined;
  private heartbeatTimer: ReturnType<typeof setInterval> | undefined;
  private lastMessageAt = 0;
  private currentStatus: ConnectionStatus = 'idle';
  private readonly seen = new Set<string>();
  private readonly messageListeners = new Set<(event: Envelope) => void>();
  private readonly statusListeners = new Set<() => void>();
  private readonly options: RealtimeOptions;

  constructor(options: RealtimeOptions) {
    this.options = options;
  }

  get status(): ConnectionStatus {
    return this.currentStatus;
  }

  start(): void {
    if (!this.stopped) return;
    this.stopped = false;
    window.addEventListener('online', this.reconnectNow);
    this.connect();
  }

  stop(): void {
    this.stopped = true;
    window.removeEventListener('online', this.reconnectNow);
    clearTimeout(this.reconnectTimer);
    clearInterval(this.heartbeatTimer);
    this.socket?.close(1000, 'client stop');
    this.socket = null;
    this.setStatus('idle');
  }

  onMessage(listener: (event: Envelope) => void): () => void {
    this.messageListeners.add(listener);
    return () => {
      this.messageListeners.delete(listener);
    };
  }

  /** Shaped for useSyncExternalStore. */
  subscribeStatus = (listener: () => void): (() => void) => {
    this.statusListeners.add(listener);
    return () => {
      this.statusListeners.delete(listener);
    };
  };

  send(type: string, payload: unknown): boolean {
    if (this.socket?.readyState !== WebSocket.OPEN) return false;
    this.socket.send(JSON.stringify({ type, payload }));
    return true;
  }

  private reconnectNow = (): void => {
    if (this.stopped || this.socket?.readyState === WebSocket.OPEN) return;
    clearTimeout(this.reconnectTimer);
    this.socket?.close();
    this.attempt = 0;
    this.connect();
  };

  private connect(): void {
    this.setStatus(this.attempt === 0 ? 'connecting' : 'reconnecting');
    const socket = new WebSocket(this.options.url);
    this.socket = socket;

    socket.onopen = () => {
      this.attempt = 0;
      this.lastMessageAt = Date.now();
      this.setStatus('open');
      this.startHeartbeat();
    };

    socket.onmessage = (event: MessageEvent<unknown>) => {
      this.lastMessageAt = Date.now();
      if (typeof event.data !== 'string') return;
      const parsed = Envelope.safeParse(safeJson(event.data));
      if (!parsed.success || parsed.data.type === 'pong') return;
      if (parsed.data.id && !this.remember(parsed.data.id)) return; // duplicate delivered after a reconnect
      for (const listener of this.messageListeners) listener(parsed.data);
    };

    socket.onerror = () => socket.close(); // onclose decides what happens next

    socket.onclose = (event) => {
      clearInterval(this.heartbeatTimer);
      if (this.socket !== socket || this.stopped) return; // stop() or a newer socket owns the state
      if (event.code === UNAUTHORIZED_CLOSE_CODE && this.options.onUnauthorized) {
        void this.options.onUnauthorized().then((refreshed) => {
          if (refreshed) this.scheduleReconnect();
          else this.stop();
        });
        return;
      }
      this.scheduleReconnect();
    };
  }

  private scheduleReconnect(): void {
    this.setStatus('reconnecting');
    this.reconnectTimer = setTimeout(() => this.connect(), backoffDelay(this.attempt));
    this.attempt += 1;
  }

  private startHeartbeat(): void {
    const interval = this.options.heartbeatMs ?? 25_000; // under typical 30–60 s proxy idle timeouts
    this.heartbeatTimer = setInterval(() => {
      if (Date.now() - this.lastMessageAt > interval * 2) {
        this.socket?.close(4000, 'heartbeat timeout'); // connection is silently dead
        return;
      }
      this.send('ping', null);
    }, interval);
  }

  private remember(id: string): boolean {
    if (this.seen.has(id)) return false;
    this.seen.add(id);
    if (this.seen.size > DEDUPE_WINDOW) {
      const oldest = this.seen.values().next().value;
      if (oldest !== undefined) this.seen.delete(oldest);
    }
    return true;
  }

  private setStatus(next: ConnectionStatus): void {
    if (next === this.currentStatus) return;
    this.currentStatus = next;
    for (const listener of this.statusListeners) listener();
  }
}
```

## Connecting real-time events to the query cache

Each feature owns a handler that validates its events and updates its own cache entries. The app layer routes events to handlers and refetches active queries after a reconnect, to fill anything missed while offline.

**`features/orders/api/orders-realtime.ts`** (export it from the feature's `index.ts`):

```ts
import type { QueryClient } from '@tanstack/react-query';
import type { Envelope } from '@/shared/lib/realtime';
import { reportError } from '@/shared/lib/errors';
import { Order } from '../model/order';
import { orderDetailQuery, orderKeys } from './orders-api';

/** Returns true when this feature owns the event type. */
export function applyOrderEvent(queryClient: QueryClient, event: Envelope): boolean {
  if (event.type !== 'order.updated') return false;
  const parsed = Order.safeParse(event.payload);
  if (!parsed.success) {
    reportError(parsed.error); // contract break: report, don't corrupt the cache
    return true;
  }
  queryClient.setQueryData(orderDetailQuery(parsed.data.id).queryKey, parsed.data); // detail updates instantly
  void queryClient.invalidateQueries({ queryKey: orderKeys.lists() }); // only lists on screen refetch
  return true;
}
```

**`src/app/realtime.ts`**

```ts
import { useSyncExternalStore } from 'react';
import type { QueryClient } from '@tanstack/react-query';
import { sessionQuery } from '@/features/auth';
import { applyOrderEvent } from '@/features/orders';
import { RealtimeClient, type ConnectionStatus, type Envelope } from '@/shared/lib/realtime';

const handlers = [applyOrderEvent]; // add one entry per feature

export function applyRealtimeEvent(queryClient: QueryClient, event: Envelope): void {
  const handled = handlers.some((handle) => handle(queryClient, event));
  if (!handled && import.meta.env.DEV) console.warn('Unhandled real-time event:', event.type);
}

function realtimeUrl(): string {
  const url = new URL('/api/realtime', window.location.href);
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
  return url.toString();
}

let activeQueryClient: QueryClient | null = null;

export const realtime = new RealtimeClient({
  url: realtimeUrl(),
  // Re-fetching /auth/me goes through http.ts, which refreshes the session if it can.
  onUnauthorized: async () =>
    activeQueryClient ? Boolean(await activeQueryClient.fetchQuery({ ...sessionQuery, staleTime: 0 })) : false,
});

export function startRealtime(queryClient: QueryClient): () => void {
  activeQueryClient = queryClient;
  let wasOpen = false;
  const stopStatus = realtime.subscribeStatus(() => {
    if (realtime.status !== 'open') return;
    if (wasOpen) void queryClient.invalidateQueries(); // reconnected: refetch what changed while offline
    wasOpen = true;
  });
  const stopMessages = realtime.onMessage((event) => applyRealtimeEvent(queryClient, event));
  realtime.start();

  return () => {
    stopStatus();
    stopMessages();
    realtime.stop();
    activeQueryClient = null;
  };
}

export function useRealtimeStatus(): ConnectionStatus {
  return useSyncExternalStore(realtime.subscribeStatus, () => realtime.status);
}
```

**`src/app/authenticated-layout.tsx`** — final version: connect only while signed in, and tell users when updates are paused:

```tsx
import { useEffect } from 'react';
import { Outlet } from '@tanstack/react-router';
import { useQueryClient } from '@tanstack/react-query';
import { startRealtime, useRealtimeStatus } from './realtime';

export function AuthenticatedLayout() {
  const queryClient = useQueryClient();
  useEffect(() => startRealtime(queryClient), [queryClient]); // returns the cleanup
  const status = useRealtimeStatus();

  return (
    <>
      {status === 'reconnecting' && (
        <p role="status" className="connection-banner">
          Reconnecting… new changes will appear when the connection is back.
        </p>
      )}
      <Outlet />
    </>
  );
}
```

At very high event rates (hundreds per second), batch events and invalidate once per animation frame instead of once per event.
