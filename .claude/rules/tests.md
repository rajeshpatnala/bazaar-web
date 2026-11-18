---
paths:
  - "**/tests/**"
  - "**/*.test.ts"
  - "**/*.test.tsx"
  - "fixtures/**"
  - "web/e2e/**"
  - "web/src/mocks/**"
---
# Test files

- Follow `bz-testing`. Deterministic: `FakeClock`, no sleeps, no wall clock, no real network or broker API.
- Recorded fixtures in `fixtures/` (anonymised, no account ids or tokens), wiremock for Kite REST, a local fake WebSocket server, in-process axum router tests, MSW for the UI, in-memory SQLite and temp dirs.
