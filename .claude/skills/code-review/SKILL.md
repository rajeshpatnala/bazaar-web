---
name: code-review
description: Code review procedure for this repo - scope, red-flag scan, generic and stack-specific checklists, severity labels, and the findings file format sent to the tech lead. Use when reviewing a branch, PR, or diff.
---

# Code Review

Reviewers **do not edit source code**. Findings go to `.agents/reviews/<feature-id>/code-review.md`
(template `.agents/templates/review-findings.md`); a short summary goes to the manager, who passes it
to the tech lead for triage. Security issues you notice are recorded too, tagged `security`, and the
security reviewer owns the deep check.

Reviews run **per branch as soon as its developer hands off** (or on a draft PR at a checkpoint), in parallel with other developers still building. Review only what's on that branch.

## 1. Procedure

1. Read the feature plan (`.agents/plans/<feature-id>.md`) - acceptance criteria and slice scope.
2. Get the diff only: `git fetch -q && git diff --stat origin/$(just base)...<branch>` then `git diff origin/$(just base)...<branch> -- <files>`. Don't read unrelated files; use `.agents/codemap.md` to locate context.
3. Confirm the gate result from the developer's handoff; if missing, ask the manager for a build-runner check instead of running long builds yourself.
4. Read tests first, then code from the core outwards (domain/business logic first, integrations and entry points last).
5. Run the red-flag scan on changed files.
6. Walk the checklist for the areas touched.
7. Write findings; return a <= 10-line summary.

## 2. Red-flag scan

```bash
rg -n 'innerHTML|outerHTML|insertAdjacentHTML|document\.write|dangerouslySetInnerHTML|eval\(|new Function|javascript:|postMessage|localStorage|sessionStorage|: any\b|as any|@ts-ignore|@ts-expect-error|eslint-disable|console\.log|(VITE|NEXT_PUBLIC|PUBLIC)_[A-Z_]*(SECRET|PRIVATE|TOKEN|PASSWORD)|useEffect\(\(\) => \{\s*set' <changed files>
```

Each hit must be justified or becomes a finding.

## 3. Checklist by area

- **Correctness:** meets acceptance criteria; edge cases (empty, max, negative, duplicate, null vs missing); time zones; external input.
- **Errors:** handled at the right layer; no swallowed errors; no leaked internals to callers; logged once.
- **Security basics:** no secrets in code/logs; parameterised queries; input validated at the edge (deep check: security reviewer).
- **Tests:** unit tests present for every new behaviour and error path (missing = blocker); deterministic; mocks only.
- **Records:** a feature PR adds or updates `docs/features/F<n>-*.md` with what it built; a `fix/B<n>` PR has a regression test that fails without the fix and a `docs/bugs/B<n>-*.md` with root cause (missing or wrong = major). Never accept secrets or real customer data in a record.
- **Architecture:** layering/dependency rules from `fe-app-architecture` respected; code in the right module; ADR for irreversible choices.
- **Readability:** names, small functions, docs on public items, comments explain why.

### Stack-specific checklist

- **Playbooks:** the change follows the Best practices of every `fe-*` skill it touches; load the matching skill when the diff needs it (`fe-team-workflow` -> `references/reviewing.md` has the front-end review checklist).
- **Behaviour:** loading, empty and error states; races and stale data; URL holds shareable state (filters, tabs).
- **Data:** every API response parsed with Zod at the boundary; server data in TanStack Query, not copied into global stores; no `any`.
- **UI:** design-system components and tokens only (no raw colours or spacing); accessible names, roles, keyboard and focus; user-facing strings as messages when the app is translated.
- **Security:** no unsanitized HTML or URL sinks, no secrets behind public env prefixes, CSRF on cookie writes (`fe-security`).
- **Performance:** no heavy dependency or widget without lazy loading; budgets in `fe-performance` still met.
- **Tests:** user-centric Testing Library tests, network mocked with MSW, no flaky waits (`fe-testing`).

## 4. Severity

| Label | Meaning | Blocks merge |
|---|---|---|
| blocker | Bug, data loss, contract break, architecture violation, security issue | Yes |
| major | Missing important tests, poor error handling, likely prod performance issue | Yes unless tech lead defers |
| minor | Readability, idiom | No |
| nit | Preference | No |
| question | Need clarification | Until answered |

Each finding: id (`CR-<feature>-<n>`), severity, file:line, issue, why it matters, concrete fix (code if short).

## 5. Summary returned to the manager

```
Review <feature-id> <branch>: REQUEST CHANGES | APPROVE | APPROVE WITH MINORS
blockers: CR-12-1, CR-12-3 | major: CR-12-2 | minor: 4 | nit: 2
findings: .agents/reviews/<feature-id>/code-review.md
```

On re-review, update each finding's status (`fixed` / `not fixed` / `deferred by tech lead`) instead of writing a new file.
