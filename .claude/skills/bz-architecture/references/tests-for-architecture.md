# 14. Tests for architecture

(From `bz-architecture` SKILL.md; open when the task touches this area.)


- **Dependency test** in `crates/cli/tests` or a `xtask`: parse `cargo metadata`, assert each workspace crate's dependencies are within the section 2 table (in particular no domain crate depends on `bazaar-server`); assert `bazaar-broker` is absent from default-feature builds of the server.
- **Pipeline property tests** with `RecordingExecutor`, `FakeClock`, fixed `risk.toml` fixtures: no placement after Block, kill switch, daily-loss breach, or expired approval; every placement has an audit record first.
- **Restart, wake and scheduler tests**: rebuild the core over the same SQLite file mid-flow; kill switch still engaged, orphaned approvals expired, `job_runs` prevents double runs, catch-up follows the missed-run table under `FakeClock`; advance `FakeClock` across hours (simulated sleep) and assert the wake handler marks data stale, expires approvals, triggers a feed reconnect and keeps the kill switch.
- **Adapter contract tests**: the same suite runs against the in-memory SQLite adapter and fakes; Kite adapters only against wiremock or a local WS server with recorded frames from `fixtures/`.
- **Server and API-types tests**: in-process `tower::ServiceExt::oneshot` on temp dirs (no real port, no real Keychain); the server refuses a non-loopback bind; session, Host and Origin behaviour per `bz-security`; generated OpenAPI and `web/src/api/schema.d.ts` match the committed files (`bz-web-api`, `bz-testing`).
- No live API, no real `.env`; connection status only through `just db-check`. Unit tests are required in every PR (`bz-testing`).
