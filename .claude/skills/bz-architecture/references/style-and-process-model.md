# 1. Style and process model

(From `bz-architecture` SKILL.md; open when the task touches this area.)


A **modular monolith**: one Cargo workspace, one server process (`bazaar-server`, axum, serving the built React UI from `web/dist` on the same origin), plus a headless `bazaar-cli` over the same crates. Hexagonal layering: pure domain and maths at the centre, I/O behind traits, two composition roots (`server`, `cli`). No microservices, no broker, no cloud services, no containers. A second process or network service needs an ADR.

```
 Browser tab (React/TS, web/) <-HTTP /api/v1 + WebSocket /api/v1/ws-> crates/server (axum) --+
                                     127.0.0.1:8787 only                                      +--> wires adapters into ports
 bazaar-cli (headless, human-run) ----------------------------------------------------------+
        scanner -> strategies -> indicators/options/regime -> core
        execution -> risk -> core        paper | broker implement Executor
        data (Kite, bhavcopy) | store (SQLite, Parquet/DuckDB) implement core ports
```

Bazaar is **local-only and single-user**: it runs on the owner's Mac, `just run` starts `bazaar-server` on `127.0.0.1:8787`, and the UI is a browser tab. The same binary and config serve analysis, paper trading and (much later) live orders; only the wired `Executor` changes. Server and UI are built and versioned together. Route, session and CSRF design: `bz-web-api` and `bz-security`; do not restate them in designs.

Process model rules (a laptop is not a server):

1. The server is up only while the Mac is awake and `just run` is running. Nothing may assume 24x7 uptime; a design that needs it is wrong.
2. **The UI is a viewer.** Closing the tab does not stop the feed, scans, paper trading, stops or jobs. Nothing safety-relevant (kill switch, daily-loss lockout, approval expiry, MIS square-off suggestions, stale-data blocking) depends on a browser being open. Alerts use the browser Notification API while a tab is open (Telegram optional); the server keeps its own persisted state regardless.
3. Several tabs are fine: every tab is a read-mostly view of server state, and approvals are single-use server-side (first approve wins, the second gets a conflict).
4. Server-side state that matters is persisted: kill switch, daily-loss lockout (scoped to the IST trading day), approvals, intents, audit chain, `job_runs`. Only sessions are in memory; a restart logs the browser out (a new launch token is printed).
5. **Binding beyond `127.0.0.1` (LAN, VPS, tunnel, reverse proxy) is out of scope.** It needs an ADR and a security redesign (TLS, real login, rate limits) before any code; the server refuses non-loopback addresses.
