# Ownership and AI-assisted work

CODEOWNERS, reviewer assignment, roles, AI rules and an AGENTS.md example.

## Contents

- Ownership and reviewer assignment
- AI-assisted coding and review

## Ownership and reviewer assignment

At around 10 engineers, "anyone can review anything" turns into "nobody feels responsible for reviewing this". Give every path an owning **team** (never a single person, who becomes a bottleneck and a single point of failure), let GitHub assign reviewers automatically, and require a second, specialist review only where mistakes are expensive.

**`.github/CODEOWNERS`** — the last matching pattern wins, so the catch-all goes first:

```text
# Fallback owner for anything not listed below
*                              @acme/web-platform

# Features: owned by the team that builds them
/src/features/orders/          @acme/orders-web
/src/features/checkout/        @acme/payments-web
/src/features/auth/            @acme/identity-web

# Shared foundations
/src/shared/ui/                @acme/design-system
/src/shared/styles/            @acme/design-system
/design-tokens/                @acme/design-system
/src/shared/lib/http.ts        @acme/web-platform

# Build, CI and dependencies
/.github/                      @acme/web-platform
/vite.config.ts                @acme/web-platform
/eslint.config.js              @acme/web-platform
/package.json                  @acme/web-platform
/pnpm-lock.yaml                @acme/web-platform
```

When a line lists several owners, an approval from **any one** of them satisfies the rule. For paths that need two kinds of review (auth, payments, the HTTP client), require the specialist team as well. Use a ruleset rule for specific file paths if your GitHub plan supports it; otherwise use a small required CI check that looks for an approval from `@acme/security-reviewers` when those files change.

**Automatic assignment** — in each GitHub team's settings, under *Code review*:

- Enable auto-assignment with the **load balance** algorithm and **1** reviewer per PR, so work spreads evenly instead of always landing on the most senior person.
- Exclude the current on-call engineer and anyone on leave.
- Keep "notify the entire team" off, so one person owns each review rather than everyone assuming someone else has it.

**Roles**

| Role | Responsibility |
| --- | --- |
| Author | Small, self-reviewed PR; drives it to merge; responds to every comment |
| Code-owner reviewer | Responds within one business day; approves on behalf of the owning team |
| Specialist reviewer (security, design system, platform) | Required only on their paths; focuses on their area |
| Tech lead | Breaks ties, owns ADRs, watches review load and PR metrics across the team |
| Web-platform team | Owns tooling, CI, shared libraries and this playbook; reviews changes to them |

**Cross-team and cross-time-zone etiquette**

- Before opening a PR in another team's area, talk to them, or open an issue. Surprise PRs to someone else's code get slow reviews.
- Make sure the author gets your review before their working day ends, or before it starts if they've already finished. Approving with comments avoids losing a full day to a time-zone gap.
- Rotate review duty (each team member reviews outside their own feature at least once a sprint), so knowledge spreads and no area has only one expert.

## AI-assisted coding and review

AI coding assistants and review bots are now part of most teams' workflow. Used well, they speed up boilerplate, tests and first-pass review. Used carelessly, they produce large, plausible-looking PRs that nobody fully understands. The standard doesn't change: **the author owns and understands every line that merges, and a human code owner approves it.**

**Rules**

| Rule | Why |
| --- | --- |
| The author can explain every line of an AI-assisted change in review | "The assistant wrote it" is not an answer to a reviewer's question, or in an incident |
| AI-generated code meets the same bar: tests, size limits, lint, review | Volume is cheap to generate and expensive to review; the size rule matters more, not less |
| An AI review bot comments first; it never counts as an approval | Bots catch mechanical issues quickly; humans judge design, product fit and trade-offs |
| No secrets, credentials or customer data in prompts; use only tools approved by the company | Prompts may leave your environment |
| Mention substantial generated code in the PR description | Reviewers adjust their attention (generated code tends to look right while hiding subtle bugs) |
| Repository conventions live in an `AGENTS.md` file the tools read | Assistants follow the same architecture and style rules humans do |

**`AGENTS.md`** (repository root) — a short, factual brief for coding assistants, kept in sync with this playbook:

```markdown
# Conventions for AI coding assistants

## Commands
- Install: `pnpm install` · Everything CI runs: `pnpm verify` · Unit/component tests: `pnpm test:unit`

## Architecture
- Features live in `src/features/<name>/{model,api,components}` and are imported only via their `index.ts`.
- Server data uses TanStack Query with the feature's `queryOptions`; components never call `fetch`.
- Every API response is parsed with the feature's Zod schema.
- Shareable view state (filters, page, tab) lives in validated URL search params.

## Code style
- Strict TypeScript: no `any`, no `enum`, named exports only, kebab-case file names.
- UI comes from `src/shared/ui`; styles are CSS modules using semantic tokens only.

## Tests
- Every change includes tests. Query by role or label; mock only the network (MSW), time and randomness.

## Never
- Add a dependency without asking · edit generated files · disable a lint rule · weaken a test to make it pass · commit secrets.
```

**Getting value from AI review**

- Configure the bot to comment with the same intent labels as humans (`issue:`, `suggestion:`, `nit:`), and to skip style issues the linters already enforce.
- Treat its comments as a checklist the author clears **before** requesting human review. That keeps human reviewers on design and behaviour.
- Track false positives. If most bot comments are dismissed, tune its instructions or turn it off; noise trains people to ignore all comments.
