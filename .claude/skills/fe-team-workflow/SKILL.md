---
name: fe-team-workflow
description: "Defines how a 10+ engineer front-end team works: trunk-based git with short-lived branches and stacked PRs, Conventional Commits, PR sizing and descriptions, self-review, reviewing in priority order with labelled comments, verdicts and disagreement handling, CODEOWNERS and reviewer load balancing, AI-assisted coding rules and AGENTS.md, ADRs/RFCs, onboarding, tech debt, postmortems and flow metrics. Use whenever opening, describing, splitting or reviewing a PR, writing commit messages, setting up CODEOWNERS or AGENTS.md, or deciding team process."
---

# Team workflow, PRs & code review

Process rules for humans and agents. Every agent follows the authoring and self-review rules on every PR it opens. Load `fe-standards` alongside it.

## Best practices

Binding rules; the reference files show how to apply them.

**Changes**

1. **Small PRs, one purpose each** (aim for under ~400 changed lines) — small changes get reviewed faster and more carefully, and are easy to revert.
2. **Short-lived branches, merged at least daily** — long branches turn into painful merges and surprise conflicts.
3. **Refactors and behaviour changes go in separate PRs** — reviewers can verify "nothing changed" and "this changed" separately.
4. **Unfinished work merges behind a feature flag, not on a branch** — integration problems surface in days, not months.
5. **Authors review their own diff before asking anyone else** — the cheapest reviewer is you, five minutes before you click "Ready".

**Reviews**

6. **Review at your next natural break, and always within one business day** — team velocity matters more than individual velocity, but don't break your own focus mid-task to do it ([Google](https://google.github.io/eng-practices/review/reviewer/speed.html)).
7. **Review in priority order: design, behaviour, complexity, tests, then naming** — the most expensive problems get caught first.
8. **Tools enforce style; humans review substance** — formatting and lint comments are banned in review.
9. **Label comment intent** (`issue:`, `suggestion:`, `nit:`, `question:`, `praise:`) — the author knows what blocks the merge and what doesn't.
10. **Approve once the change improves the codebase, even if it isn't perfect; approve with comments for minor items** — perfection-blocking slows everyone and teaches people to avoid review.
11. **Critique code, never people; explain the why** — psychological safety is what makes honest review possible.
12. **Move to a call after two rounds of back-and-forth** — then summarize the outcome in the PR for the record.

**Team**

13. **Every folder has an owning team, and CODEOWNERS routes reviews automatically** — no PR waits because nobody feels responsible.
14. **Decisions that are expensive to reverse get an ADR** — new engineers can read why, instead of re-arguing it.
15. **Measure the flow (cycle time, review wait, PR size), never individuals** — metrics are for fixing the process, not ranking people.
16. **AI tools can draft code and do first-pass review, but a human owns every line that merges** — accountability doesn't transfer to a tool.

## Workflow

**Every PR:** follow `references/git-and-authoring.md` — branch and commit naming, size guide (< ~400 lines), splitting strategies, PR description, self-review checklist.

**Every review:** follow `references/reviewing.md` — priority order, comment labels, verdicts, disagreement ladder.

**Repo setup (once):** git config, CODEOWNERS, auto-assignment, `AGENTS.md`, PR template → `references/ownership-and-ai.md` and `git-and-authoring.md`.

**Team process:** ADR/RFC choice and templates, onboarding plan, tech-debt handling, postmortems, metrics → `references/team-process.md`.

## Files this playbook owns (Build mode)

`.github/CODEOWNERS`, `.github/pull_request_template.md`, `AGENTS.md`, `docs/adr/*`, `docs/rfc/*` templates, `CONTRIBUTING.md`.

## Done when

CODEOWNERS covers every folder; the PR template contains description, test steps and checklist sections; `AGENTS.md` lists commands, architecture, style, tests and never-do rules; branch protection requires owner review; every PR an agent opens is under ~400 lines, single-purpose, conventionally titled and self-reviewed.

## Reference files

Read only the file the current step needs. Paths are relative to this skill's folder (`.claude/skills/<skill>/`).

- `references/git-and-authoring.md` — PR lifecycle, git config and daily flow, branch/commit rules, stacked PRs, PR size, description example and self-review.
- `references/reviewing.md` — Review workflow, priority table, comment labels, good and bad comments, verdicts and the disagreement ladder.
- `references/ownership-and-ai.md` — CODEOWNERS, reviewer assignment, roles, AI rules and an AGENTS.md example.
- `references/team-process.md` — ADR/RFC guidance and templates, onboarding plan, tech debt, postmortem template and flow metrics.

## Sources

- [Speed of code reviews — Google eng-practices](https://google.github.io/eng-practices/review/reviewer/speed.html)
- [What to look for in a code review — Google eng-practices](https://google.github.io/eng-practices/review/reviewer/looking-for.html)
