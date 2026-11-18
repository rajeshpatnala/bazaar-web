# Decisions, onboarding, debt, incidents and metrics

ADR/RFC guidance and templates, onboarding plan, tech debt, postmortem template and flow metrics.

## Decisions, onboarding, tech debt, incidents and metrics

The practices that keep a 10-person team effective as it grows to 30: write decisions down, get new people productive in days, pay down debt continuously, learn from incidents without blame, and watch the flow of work.

**Writing decisions down**

| Use | When | Outcome |
| --- | --- | --- |
| ADR (`docs/adr/NNNN-title.md`, template in `fe-repo-foundation`'s repo hygiene) | A decision that is expensive to reverse: state library, rendering strategy, styling approach | One page: context, decision, alternatives, consequences |
| RFC | A proposal affecting several teams or needing feedback first (a new shared package, a migration) | Discussed for about a week, decided in the fortnightly front-end architecture review, then recorded as an ADR |
| Lint rule or codemod | A decision that can be checked by a machine | Nobody has to remember it; the deprecated pattern can't come back |

**RFC template** (`docs/rfcs/NNNN-title.md`):

```markdown
# RFC: Move server state from Redux to TanStack Query
Author: … · Status: Draft | In review | Accepted | Rejected · Decision by: 2026-11-20

## Problem — what hurts today, with evidence (incidents, metrics, PR friction)
## Proposal — what changes, with a small example
## Alternatives — including "do nothing"
## Migration — steps, who does what, how long; can it be incremental?
## Risks and open questions
```

**Onboarding plan** — a new engineer should merge a real change in their first two days:

| When | Goal |
| --- | --- |
| Day 1 | Laptop set up from the README in under 15 minutes; read the best-practices sections of the playbook; meet their buddy |
| Day 2 | First merged PR from a `good-first-issue` ticket (small, real, reviewed like any other) |
| Week 1 | Pair with the buddy on a feature behind a flag; review two PRs alongside a senior reviewer |
| Week 2–4 | Join the review rotation; ship a small feature end to end; shadow an on-call shift |
| Day 30 | Retro with their lead: what in the docs or tooling was confusing? Fix it. Each new hire improves onboarding for the next |

**Managing tech debt**

- Keep a debt register: issues labelled `tech-debt`, each with the cost it causes today ("every new form duplicates 80 lines of validation") and a rough size.
- Spend 15–20% of each sprint on it, prioritized by cost, not by annoyance.
- Boy-scout rule: leave touched code slightly better, in a separate PR if the clean-up is non-trivial.
- Deprecate with tools: mark the old module with a `no-restricted-imports` lint rule (new usage fails CI), migrate existing usages with a codemod, then delete it.
- Upgrades are continuous (weekly Renovate PRs, `fe-repo-foundation`), so no framework migration grows into a quarter-long project.

**Blameless postmortems** — after any incident that affected users:

```markdown
# Postmortem: Checkout errors for 5% of users, 2026-11-04
## Impact — who was affected, how many, for how long
## Timeline — detection, response, mitigation, resolution (with times)
## Root cause — the system conditions that allowed it, not the person who pushed it
## What went well / what went badly
## Action items — each with an owner and a due date (tests, alerts, guardrails, docs)
```

The question is always "how did our system let this happen?" — never "who did this?". People who fear blame hide problems.

**Team metrics** — reviewed monthly by the team, to improve the process. Never used to rank individuals.

| Metric | What it tells you | Healthy target |
| --- | --- | --- |
| Deployment frequency (DORA) | How small and routine releases are | Multiple per day |
| Lead time for changes (DORA) | Merge-ready idea to production | Under 1 day |
| Change failure rate (DORA) | Share of deploys causing an incident or rollback | Under 15% |
| Time to restore (DORA) | How fast you recover | Under 1 hour (rollback makes this minutes) |
| Time to first review | Whether reviews are blocking people | Median under 4 working hours, max 1 day |
| PR cycle time (open → merged) | Overall flow | Median under 1 day |
| PR size | Whether splitting is a habit | Median under 200 changed lines |
