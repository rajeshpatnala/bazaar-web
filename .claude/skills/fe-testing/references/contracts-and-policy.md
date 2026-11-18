# Contracts, flakiness, coverage and review

OpenAPI type generation and checks, flaky-test process, coverage policy, CI budgets and the test review checklist.

## Contents

- API contract checks
- Flaky tests, coverage and CI policy
- Test review checklist for PRs

## API contract checks

The front end and the API should disagree at compile time, not in production. Generate TypeScript types from the backend's OpenAPI spec, check every Zod schema against them, and fail CI when the generated types change without the front end being updated.

```bash
pnpm add -D openapi-typescript
```

Add these scripts to `package.json`. The spec comes from the backend: a committed file, a published artifact, or the staging API's `/openapi.json`.

```json
{
  "api:types": "openapi-typescript ./openapi/api.json -o src/shared/api/schema.d.ts",
  "api:check": "pnpm api:types && git diff --exit-code src/shared/api/schema.d.ts"
}
```

**Tie each Zod schema to the generated type** — if the backend renames or retypes a field, `tsc` fails right here:

```ts
// features/orders/model/order.ts
import type { components } from '@/shared/api/schema';

type ApiOrder = components['schemas']['Order'];

export const Order = z.object({
  id: z.string(),
  customerName: z.string(),
  totalCents: z.number().int().nonnegative(),
  currency: z.string().length(3),
  status: OrderStatus,
  createdAt: z.iso.datetime(),
}) satisfies z.ZodType<ApiOrder>;
```

**How the two layers work together**

| Check | Catches | When |
| --- | --- | --- |
| `satisfies z.ZodType<ApiOrder>` | Schema out of sync with the spec | Compile time, in every PR |
| `api:check` in CI | Spec changed but nobody regenerated types | Every PR, and when the backend publishes a new spec |
| `Order.parse()` at runtime | The deployed API returning something the spec doesn't promise | In production, reported to Sentry |

When front-end and backend teams release independently, add consumer-driven contract tests (Pact), so the backend's CI fails before it deploys a change that would break the front end.

## Flaky tests, coverage and CI policy

A test suite people don't trust is worse than none, because real failures get re-run until they pass. Treat flakiness as an incident, keep the suite fast enough that nobody skips it, and use coverage only to stop quality sliding backwards.

**Flaky-test process**

1. **Detect.** Playwright reports tests that passed only on retry as `flaky`; post them to the team channel. Vitest runs with no retries, so any intermittent failure is visible.
2. **Quarantine within one business day.** Mark the test `test.fixme(…)` with a comment linking a ticket and naming an owner. The suite goes green again without losing the test.
3. **Fix within one week.** Find the root cause, not a longer timeout.
4. **Prove the fix:** temporarily give the Vitest test `{ repeats: 50 }` (or run `playwright test <file> --repeat-each=50`). It must pass every time before it leaves quarantine.

**Common causes and fixes**

| Symptom | Usual cause | Fix |
| --- | --- | --- |
| Passes alone, fails in the full run | Shared state between tests (store, cache, module variable) | Fresh `QueryClient` per test, reset stores in `afterEach` |
| Fails around midnight or on some machines | Real time or timezone | `vi.setSystemTime`, `TZ=UTC`, explicit locale |
| "Element not found" now and then | Asserting before async work finished | `findBy*` / web-first assertions instead of `getBy*` after an action |
| Different result on each run | Unseeded randomness or generated ids | Stub `crypto.randomUUID`, use factory sequences |
| E2E fails only in CI | Shared seeded data, or a test depending on another's leftovers | Each test seeds and cleans up its own data |
| Screenshot differs slightly | Animation, font loading, or a dynamic region | Disable animations, await `document.fonts.ready`, mask dynamic content |

**Coverage policy**

- Enable thresholds once real features exist, set a few points below the current value, and raise them occasionally. They exist to catch large drops, nothing more.
- Show diff coverage on each PR (most coverage services comment it automatically). Reviewers ask "is the new logic tested?", not "is the number above 80?".
- Exclude generated code, mocks, stories and `main.tsx`.
- For critical pure logic (pricing, permissions), mutation testing (StrykerJS) shows whether tests would actually fail if the code were wrong. Run it weekly, not on every PR.

**CI time budgets** — when a stage exceeds its budget, fix it like a bug:

| Stage | Budget |
| --- | --- |
| Static checks (types, lint, format) | 2 min |
| Unit + component tests | 5 min |
| Visual regression | 3 min |
| E2E smoke on PR preview | 5 min |
| Full E2E (sharded, merge queue and nightly) | 15 min |

In a monorepo, run only affected projects (`turbo run test --affected` or `nx affected -t test`) with remote caching, so a CSS change in one app doesn't re-run every package's tests.

## Test review checklist for PRs

Reviewers check tests with the same care as production code. Copy this into the PR template's test section.

- [ ] New behaviour has tests at the cheapest layer that can catch its bugs; bug fixes include a test that fails without the fix.
- [ ] Tests assert what a user sees or does, not hook calls, internal state or cache keys.
- [ ] Elements are found by role, label or text; any `data-testid` has a reason.
- [ ] `userEvent` for interaction, `findBy*` after async work; no `fireEvent`, no `sleep`, no `waitForTimeout`.
- [ ] Only boundaries are mocked (network, time, randomness, third parties); no `vi.mock` of our own components.
- [ ] Each test builds its own data with factories and passes when run alone or in any order.
- [ ] Error, empty and loading paths are tested, not only the happy path.
- [ ] Test names are sentences describing behaviour; each test checks one behaviour.
- [ ] No new large snapshots; visual tests only for design-system components, with screenshots generated in CI.
- [ ] Would each test actually fail if the code broke? (Try breaking it once.)
