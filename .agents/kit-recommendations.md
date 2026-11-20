# Kit recommendations: spend fewer tokens, keep the same results

Stack-neutral. For kit maintainers building or updating any manager + sub-agent kit.
Evidence is anonymized, from one multi-day project (about 50 agent runs per feature, 15M-token daily budget). Treat the numbers as a pattern to check, not a law.

## Principle

Tokens go to four places, in this order: (1) the orchestrator re-sending its own context on every step, (2) review work on large or repeated PRs, (3) cold starts of fresh agents, (4) rework from tool defects. Fix them in that order. Anything that a script can decide should never reach a model.

## A. Scheduler as a script, not an agent

1. **Add a deterministic "next" script, not a scheduler agent.** A new agent adds a cold start and holds a growing context like the orchestrator does. The scheduler value comes from deterministic code. One command reads board, open PRs, review files and agent context sizes, then prints one plan: who to launch, warm or fresh, who to skip, what to wait on. The orchestrator executes the plan instead of reasoning about it each turn.
2. **Batch launches.** When several PRs are ready within a short window, launch one reviewer per type with all review packs, so one start serves all. Do not launch a full reviewer set per PR.
3. **One barrier wait.** A single background script waits until every required review file for a PR (or batch) exists with a parsed verdict, then wakes the orchestrator once. Suppress the duplicate completion notice that follows a hand-back message.
4. **Skip by diff content, not path.** Security review only where I/O, parsing, auth, secrets or dependencies change. Config-only, type-only and docs-only diffs skip it. Keep domain-safety reviewers (money, capital, data loss) on their areas.
5. **Cheapest route first.** Order of preference for any task: a command the orchestrator runs itself, a tool-free scripted check, one combined reviewer, then separate reviewers. Small changes must never take the full path.

## B. Reviewer reuse policy

6. **Reuse only inside one fix cycle** (same PR, re-review). Across PRs, a warm reviewer's context grows and each step re-sends it: measured about 150k per review warm after five turns versus about 80k fresh.
7. **Reset by size, not by habit.** Replace a reused agent after about 5 turns or about 120k context. Hand over only the last findings file, never the history.
8. **Do not keep a review whose independence matters warm across PRs** (safety, money, security). A fresh read catches what a primed one skims.
9. **Reviewer starts are cheap; reviews are not.** Measure both before optimizing starts: starts were about 7k per reviewer, reviews 40-60k. Reduce review size (A4, D) before chasing start cost.

## C. Triage and merge by script

10. **Triage decides everything mechanical.** Map every reviewer's severity words to one scale. Nits, info and "deferred to slice N" are decided by the script. Only blockers and majors reach a model.
11. **Merge by script, never by an agent.** A merge costs no tokens when the orchestrator runs it once approvals exist.
12. **Merge-check prints state, not guesses.** On an already-merged PR say "already merged"; on any failure list what is missing and the exact fix in order.
13. **Lint review files.** Verify the verdict line and path before the merge gate; copy review files out of isolated worktrees automatically. Missing or malformed files cost a full reviewer round trip each.

## D. Size discipline

14. **Cap a PR at about 400 production lines** (exclude tests at half weight, lockfiles, generated code, docs). The cap is for the reviewer's single read, so flag and carry on when over; never ask "push as is?".
15. **Plan slices to the cap.** A 1000-line PR costs several fix rounds; three 350-line PRs cost less review and merge safely in parallel.
16. **Scripts and modules at about 400 lines or less.** When a script needs more, split by responsibility (read state, decide, format) with one thin entry file importing the modules. One module plus its tests per slice, so each PR stays under the cap. Do not split mechanically at a line count.
17. **Unit-test scripts with fake inputs** (fake PR states, fake review files) so decisions are verified without real PRs.

## E. Orchestrator context

18. **Restart at milestones.** After each merged wave, write state to the board and start a fresh session; the board carries resume state.
19. **No raw diffs, lockfiles or generated files in the orchestrator context.** Use stats and one-line summaries.
20. **One status script** shows all open PRs, verdicts, sign-offs, owner actions and budget in one table.
21. **Fixed short hand-back format** (id, verdict, blocker, next) with a link to the detail file. Never restate agent reports.
22. **Ask only real decisions,** batched, with a recommended default first.
23. **Say once.** Things already announced (a size flag, an owner action) are not repeated each turn.

## F. Cold starts and preloads

24. **Measure preload weight per agent** (agent file + skills + project instructions) with a script, and re-measure after each change. A tool-free review call that loads a large preload (20k+ per start) defeats its own purpose.
25. **Skill core plus references.** About 60 lines of rules, an index of which reference to open, and a done checklist; examples and tables load on demand.
26. **Role digests and a "task type -> read these files" index** stop agents opening adjacent material.
27. **Cache-friendly launching.** Launch same-type agents close together, or use a longer cache time for often-launched agents; track the cached share of starts (target above 30%).
28. **Tester once per feature plus targeted runs on risk areas,** not per PR.

## G. Budget and measurement

29. **Budget must track the owner's real allowance:** owner-set cap and used figure, manager cost included, warnings at 70, 85 and 95%. A kit budget far below the real limit hides an approaching hard stop.
30. **Book usage per agent task,** not one global current task, so parallel features do not mix rows.
31. **Label notifications with the task** ("Review PR 52"), not a reused agent's first label.
32. **Track per feature:** cold-start share (target under 25%), orchestrator context at each milestone, cost per review, fix rounds per PR. Keep only changes that move these.

## H. Tooling hygiene

33. **Recipes reject `--help` and unknown flags** instead of running with them (one helper recipe created a real PR from a help flag).
34. **Quote-safe recipes:** free-text arguments (milestone notes) must survive semicolons, colons and parentheses.
35. **A records recipe with a path allowlist** so the orchestrator can commit notes, specs and memory through a docs PR; unlisted record types must be allowed by config, not forced into manual commits.
36. **A doctor check** that every command named in agent files exists, and workflow files are linted before first push.
37. **Guards parse the real command and branch,** not words in text.
38. **Hook denials must say what to do next** (which recipe, who approves) so the orchestrator does not retry or improvise.

## I. What to keep

- Scripted triage, scripted merge and one merge gate.
- Warm developer for fix rounds and the next slice in the same area.
- One planner run per feature, with revisions on the same agent.
- A safety reviewer on the plan before code (blockers cost plan price, not code price).
- Fixed review pack, fixed output paths, finding ids scoped to the PR, in-place status updates.
- Board as resume state, owner-actions list, standing owner rules written at once and fixed in the kit.

## J. Suggested order

| order | change | effort | effect |
|---|---|---|---|
| 1 | plan slices to the cap (D14-D15) | XS | cuts review and fix-round cost directly |
| 2 | barrier wait and duplicate-notice removal (A3) | M | removes the largest orchestrator cost |
| 3 | triage by script for all severities (C10) | M | fewer expert launches |
| 4 | reviewer reset and batching (A2, B6-B7) | S-M | 30-50% of review cost |
| 5 | skip-by-content rules (A4) | M | removes low-value reviews |
| 6 | the "next" plan script (A1) | M | removes per-turn launch reasoning |
| 7 | budget tracks real allowance (G29) | M | avoids a mid-feature hard stop |
| 8 | tooling fixes (C12-C13, H33-H38) | S | removes rework |
