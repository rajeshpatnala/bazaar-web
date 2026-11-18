---
name: fe-testing
description: "Defines and implements the front-end test strategy: which layer to test at, writing user-centric Testing Library tests, factories and per-feature MSW mocks, controlling time and randomness, Vitest Browser Mode visual regression, Playwright E2E with fixtures, tags and sharding, OpenAPI contract checks, flaky-test quarantine, coverage policy and the test review checklist. Use whenever writing, fixing or reviewing tests, test infrastructure, mocks, coverage or CI test jobs, or when a test is flaky."
---

# Testing

Owns the final test configuration and the rules every other playbook's tests follow. Load `fe-standards` alongside it.

## Best practices

Binding rules; the reference files show how to apply them.

**What to test**

1. **Test behaviour users can observe, not implementation** — tests that click and read the screen survive refactors; tests that check hook calls or state break on every one.
2. **Put most effort into component/integration tests** — they exercise real components, real cache and routing with only the network mocked, which gives the most confidence per test.
3. **Unit-test pure logic exhaustively** — business rules in `model/` are cheap to test with tables of inputs and outputs.
4. **Keep E2E for the 5–15 journeys that make or lose money** — sign-in, checkout, core create and edit. E2E tests are slow and costly to maintain, so each one must earn its place.
5. **Every bug fix ships with a test that fails without the fix** — the same bug never returns.

**How to write tests**

6. **Find elements the way users do: by role, label and text** — this also proves the UI is accessible; `data-testid` is the last resort.
7. **Simulate real interaction with `userEvent` and wait with `findBy*`** — no `fireEvent` shortcuts, no manual `sleep`.
8. **One behaviour per test, named as a sentence** (`'shows an error when the card is declined'`) — a failure explains itself.
9. **Each test sets up exactly what it needs** — factories with overrides, no shared mutable fixtures, no test depending on another's order.
10. **No large snapshots** — nobody reads a 300-line diff; assert the specific thing that matters.

**Keeping the suite trustworthy**

11. **Tests are deterministic** — network mocked, time frozen, random values seeded. Unhandled requests fail the test.
12. **A flaky test is a P1 bug** — quarantine it within a day and fix it within a week; never "just re-run".
13. **Coverage is a floor against drops, not a target** — nobody writes tests to move a percentage.
14. **Visual regression only for the design system** — screenshots of whole pages break on every content change.
15. **The suite must stay fast** — unit and component tests finish in minutes and E2E is sharded, so engineers keep running them.

## Workflow

1. Pick the cheapest layer that catches the bug → `references/strategy-and-writing.md` (What to test at which layer).
2. Apply the writing rules and the what-to-mock table; add jsdom stubs → `strategy-and-writing.md` (Writing good tests).
3. `factory<T>()`, per-feature `*.mocks.ts` with scenarios, aggregated handlers, fake timers, `TZ=UTC` → `references/test-data.md`.
4. Split `vitest.config.ts` into `unit` and `vrt` projects; add visual tests for design-system components only → `references/browser-and-e2e.md` (Browser Mode).
5. Playwright fixtures, `@smoke` tags, sharding → `browser-and-e2e.md` (End-to-end).
6. `api:types` / `api:check` contract scripts → `references/contracts-and-policy.md`.
7. Apply the flaky-test process, coverage policy and CI budgets; use the review checklist on every PR → `contracts-and-policy.md`.

## Files this playbook owns (Build mode)

`vitest.config.ts` (final, `fe-design-system` adds the Storybook project), `src/test/**`, `src/**/*.mocks.ts` conventions, `src/test/factories.ts` helper, `src/mocks/browser.ts`, `e2e/fixtures.ts`, `e2e/*.spec.ts` conventions, `scripts` entries `api:types` / `api:check`, screenshot baselines (generated in CI only).

## Done when

`pnpm test:ci` runs the unit and vrt projects green; an unhandled request fails the test; tests pass alone and in random order; screenshots are produced in CI only; `pnpm api:check` fails when the OpenAPI spec drifts from the Zod schemas; E2E smoke runs on PRs and the full sharded suite in the merge queue.

## Reference files

Read only the file the current step needs. Paths are relative to this skill's folder (`.claude/skills/<skill>/`).

- `references/strategy-and-writing.md` — Layer table, anti-patterns, what to mock, jsdom stubs and example tests.
- `references/test-data.md` — Factories, per-feature mocks and scenarios, aggregated handlers, dev MSW worker, fake timers and stable ids.
- `references/browser-and-e2e.md` — Vitest projects, screenshot tests and rules, Playwright fixtures, tags and sharding.
- `references/contracts-and-policy.md` — OpenAPI type generation and checks, flaky-test process, coverage policy, CI budgets and the test review checklist.

## Sources

- [Visual regression testing — Vitest docs](https://vitest.dev/guide/browser/visual-regression-testing)
- [Announcing Vitest 4.0 — VoidZero](https://voidzero.dev/posts/announcing-vitest-4)
- [What to look for in a code review — Google eng-practices](https://google.github.io/eng-practices/review/reviewer/looking-for.html)
