---
paths:
  - "**/*.test.ts"
  - "**/*.test.tsx"
  - "**/*.spec.ts"
  - "e2e/**"
  - "**/*.stories.tsx"
---
# Test files

- Follow `fe-testing`: user-centric Testing Library queries, network mocked with MSW, factories for data, no fixed waits.
- Never point tests at production or real backends; E2E runs against previews, staging or a local server with mocks.
- No secrets or personal data in fixtures.
