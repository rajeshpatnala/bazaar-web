# Reviewing a PR

Review workflow, priority table, comment labels, good and bad comments, verdicts and the disagreement ladder.

## Reviewing a PR

A reviewer's job is to keep the codebase healthy while keeping teammates moving. Respond quickly, look at the most important things first, be explicit about what blocks the merge, and approve once the change makes things better. [Google's guidance](https://google.github.io/eng-practices/review/reviewer/speed.html) sets one business day as the maximum response time, and notes that most complaints about code review go away when reviews get faster.

**How to work through a PR**

1. **Read the description and ticket.** Should this change exist at all, and is this the right approach? Say so now, before line-level comments.
2. **Find the core file** (the author's "start here" note helps) and review it first. If its design is wrong, stop and say so; the rest may change anyway.
3. **Open the preview for any UI change.** Use the keyboard, resize to phone width, switch to dark mode.
4. **Go through the remaining files,** tests included. Tests are code that must be maintained.
5. **Submit one batch of comments** with a clear verdict, not a trickle of single comments.

If a PR is too large to review well, ask for it to be split. If it truly can't be, comment on the overall design first, so the author is never left waiting with nothing to act on.

**What to check, in priority order** (adapted from [Google's code-review guide](https://google.github.io/eng-practices/review/reviewer/looking-for.html))

| Priority | Area | Front-end questions |
| --- | --- | --- |
| 1 | Design | Right layer and feature? Respects import boundaries? Is a new dependency justified? Solving today's problem, not a speculative one? |
| 2 | Behaviour | Loading, empty, error and slow-network states handled? Race conditions (double submit, out-of-order responses)? Works when the user navigates away mid-request? |
| 3 | State and data | State in its one home (URL, query cache, component)? Responses validated with Zod? Correct cache keys and invalidation after writes? |
| 4 | Accessibility | Semantic elements, labels, keyboard path, focus management, contrast, announcements for async changes? |
| 5 | Security | No unsanitized HTML, secrets, tokens in storage or URLs? Redirects validated? Authorization still enforced server-side? |
| 6 | Performance | Bundle impact of new imports? Heavy work off the critical path? Lists virtualized or paginated? Images sized? |
| 7 | Complexity | Can a teammate understand it quickly? Simpler alternative? Duplicated logic that belongs in `model/` or `shared/`? |
| 8 | Tests | Behaviour-focused, deterministic, covering failure paths? Would they fail if the code broke? |
| 9 | Naming, comments, docs | Clear names? Comments explain why? README, Storybook or ADR updated if behaviour changed? |

**Comment with intent labels**

| Label | Means | Blocks merge? |
| --- | --- | --- |
| `issue:` | A real problem: bug, security, accessibility, broken contract | Yes |
| `suggestion:` | A better approach, with reasoning | Author decides |
| `question:` | I don't understand; please explain (maybe in a code comment) | Until answered |
| `nit:` | Minor polish | No |
| `praise:` | Something done well | No — but do it; it teaches as much as criticism |

**Good and bad comments**

```text
❌ "This is wrong."
✅ issue: If the user clicks Pay twice, two mutations run and the card is charged twice.
   Disable the button while `isPending`, and send an Idempotency-Key header.

❌ "Why didn't you use useMemo here?"
✅ question: Is this filter expensive enough to matter? The React Compiler memoizes it already,
   so I'd drop the manual useMemo unless a profile shows a need.

❌ "Rename this."
✅ nit: `data2` → `pendingOrders`, so the JSX below reads on its own.

✅ praise: Splitting the optimistic update from the server reconciliation makes the rollback really easy to follow.
```

**Verdicts**

- **Approve** when the change improves the codebase and has no open `issue:` comments. Approving with minor comments is encouraged: say whether you need to see them addressed or trust the author to finish.
- **Request changes** only for `issue:` comments. Then re-review promptly once the author pushes; a fast second round matters more than a perfect first one.
- **Comment** when you've reviewed part of the PR (state which part) or only have questions.

**When people disagree** — resolve it in this order:

1. **Facts and data:** a benchmark, a failing test, a WCAG reference, a link to the docs.
2. **The playbook and style guide.** If it's written down, it wins; if it isn't, follow existing code.
3. **A short call** after two rounds of written back-and-forth. Then summarize the decision in the PR.
4. **The tech lead or code owner decides,** and a recurring question becomes an ADR or a lint rule, so it's never argued again.

**Protect focus** — review in two scheduled blocks a day (for example after stand-up and after lunch) rather than interrupting deep work for every notification. That meets the one-day limit without fragmenting everyone's day.
