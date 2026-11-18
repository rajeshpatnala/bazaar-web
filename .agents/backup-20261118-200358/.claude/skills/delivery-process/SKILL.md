---
name: delivery-process
description: How the manager and tech lead run work with sub-agents - preflight and work modes, intake lanes (question, bug, preview-first, work), the S/M/L size ladder, planning and slicing, verification tiers, dispatch prompts, reviews, triage and fix loops, owner rules, measured token budget, merge and release gates, feature and bug records, the code map, retrospectives and reports. Preload for manager and tech lead.
---

# Delivery Process (manager + tech lead)

## 0. Preflight (once per session, before the first work)

`just preflight` checks: integration and release branches exist, remote and `gh` work, kit files are committed,
git worktrees can be created. It prints the recommended **work mode** and fixes. The session-start hook runs a cached
version and prints a line only when something is wrong.
- Branch missing: ask the user which branch their team uses (set it in `team.md` -> Branches) or run `just setup-branches` with their OK.
- Worktrees fail: fix the cause, or `just work-mode in-place` with the user's OK.
- No `gh`/remote: reviews use `--branch`/`--diff`, merges use `just merge --branch <b>`.
- **Never swap a kit agent for a general-purpose agent** to get around a failure (drops skills, hooks, memory): stop and tell the user.

| Work mode | Developers | Reviews and merge |
|---|---|---|
| `worktrees` (default) | parallel, each in its own worktree | PR: `review-pack <pr>`, `merge <pr>`; no gh: `--branch <b>` |
| `in-place` | **one at a time** in the main checkout, on its own feature branch | `review-pack --diff <base>..<branch>`, `merge-check --branch <b>`, `merge --branch <b>` |

## 1. Files that hold state (keep them short)

| File | Owner | Purpose |
|---|---|---|
| `.agents/project.md` | manager | Project profile (<= 60 lines). If the repo has its own docs, a digest that links them |
| `.agents/owner-rules.md` | manager | Owner's standing rules; **injected into every agent by a hook** |
| `.agents/team.md` | user/manager | Every setting: agents, work mode, review policy, budget, branches, approvals |
| `.agents/board.md` | manager | Every task: id, size, agent, branch/PR, status, blocker. Source of truth for resume |
| `.agents/budget.csv` | hooks | Measured token use per agent run (automatic) |
| `.agents/codemap.md` | generated | Modules, routes, config keys, features, open bugs, in-flight branches |
| `.agents/plans/<id>.md` | tech lead | ONE plan file per feature (M or L) |
| `.agents/reviews/<id>/*.md` | reviewers/testers | Findings with ids, status, tech-lead decision |
| `.agents/reports/<date>-<id>.md` | manager | Report; L work adds a retrospective |
| `docs/features/F<n>-*.md`, `docs/bugs/B<n>-*.md` | developers via PR | **Committed** feature and bug records |

Committed: `project.md`, `owner-rules.md`, `team.md`, `decisions.md`, `templates/`, `docs/`. Local: board, budget,
codemap, plans, reviews, reports, bug drafts. Agents in worktrees don't have local files: give **absolute paths**.

## 2. Intake: pick the lane, then the size

| Lane | Signs | What happens |
|---|---|---|
| Question | "what/how/why/does it" | Answer from codemap, `just feature`, records. No agents unless it needs investigation |
| Bug | "broken, wrong, error" | §7 bug intake |
| **Preview first** | judged by look or feel, or a matter of taste: "modern, premium, smooth, feels, redesign, nicer"; also any request where the owner can't judge a written plan | §2b. **No slices until the owner picks** |
| Work | everything else | size it (§2a), then the lifecycle (§3) |

Before planning, tell the user the lane, the size and a token estimate in one line
("M, one developer, ~250k; plan next?"). The owner can redirect before anything is spent.

### 2a. Size ladder

| Size | Rule of thumb | Plan | Developers | Verification | Reviews |
|---|---|---|---|---|---|
| **S** | <= `small_max_files` files, one area | none: manager writes a 5-line brief in the dispatch | 1 | T0 (+T1 if visible) | code review if it changes behaviour |
| **M** | one feature, one area | ONE plan file <= 40 lines, no slices table | 1 for the whole feature, continued between steps | T0+T1 per step, T2 once at the end | at the end (per PR) |
| **L** | several areas, or parallel work pays off | full plan with slices | one per independent area, parallel (worktrees mode only) | T0+T1 per slice, T2 once | per PR, pipelined |

- In `in-place` mode L work runs one slice at a time; prefer M-style sequencing with the same developer.
- If a plan's estimate is more than ~3x what the visible change needs, the tech lead shrinks it or justifies it.
- When in doubt between two sizes, pick the smaller and grow if needed.
- **Light lane:** a change confined to `.agents/`, `docs/` or markdown (no source, CI, deps, auth, migrations, Dockerfile)
  needs no security review and no tester; code review per `light_lane` (default `skip`; `quick` = code review only).
  merge-check still checks title and CI. Review effort scales with the risk of the paths touched.

### 2b. Preview-first lane

Taste can't be settled with text questions, and a wrong guess in the real code costs ~5-10x a preview.
1. Ask at most one question that can be answered by pointing ("any product or site you like the feel of?").
2. Launch `prototyper` to make <= 3 cheap previews of clearly different directions in `previews/<id>/` (cap 150k tokens). What a preview is here:
   - a screen as a clickable page in `previews/<task-id>/` (static HTML or `just app-dev` with the MSW mock API and fake
     candles/recommendations), or a sample recommendation card + Why panel rendered on fake data.
   - for strategy ideas: a one-page backtest report on fixture data (equity curve, drawdown, trade list, stats) so the
     owner can judge by looking; made outside the product, no new dependencies, never real account data.
3. Show them and ask: "which do you keep, which do you drop, more or less of what?"
4. Turn the pick into owner rules (§6) and acceptance criteria, then size and build it normally.
Skip it only when the owner says "just build it" or the look is already fixed by a design they gave you.

## 3. Lifecycle

```
1 Intake    manager: lane + size + estimate (§2); `just budget-task <id>`; read board and budget
2 Plan      S: manager brief. M/L: tech lead, ONE plan file; L adds slices. Feature record draft
            inside the plan (id F<n> after `just features`). Architect only for new services/stores,
            cross-cutting or irreversible choices
3 Approve   manager: plan summary + budget check to the user (if plan approval = yes)
4 Build     L: one developer per independent slice at once (worktrees mode); M: one developer,
            continued between steps; tester writes the T2 plan in parallel; cicd if needed
5 Review    as each PR/branch is handed off: `just review-plan` -> the reviewers it lists, in parallel
            (by risk of the paths; each with its output path). In-place: `--diff`/`--branch`
6 Triage    tech lead: fix now / defer / reject. Deferred items go into the feature record
7 Fix loop  continue the SAME developer with finding ids; re-check by the same reviewers only;
            max rounds from team.md, then ask the user
8 Verify    after the last slice: ONE tester runs T2 for the whole feature (§4)
9 Merge     tech lead: `just ship <pr>` (or `--branch b --title T`): waits for CI, merge-check, merges,
            pulls; it refuses unless merge-check passes. Remaining branches rebase
10 Release  user asks -> promotion PR -> `just promote` (asks the user) -> release PR -> tag
11 Report   §11; L work adds a 5-line retrospective
```

**Plan reviews up front:** run `just review-plan --files a,b,c` on the planned file list before dispatch, so required
reviews and their output paths are known in advance. **One push per round:** every push re-runs CI and voids "green".
The manager's main checkout stays on the integration branch; feature branches live in agents' worktrees/branches.

### Slicing (L only)
- A slice is independently buildable, testable and mergeable, owns distinct files, <= `max_slice_changed_lines`.
- Typical cut: slice 0 = shared contracts, merged first; then parallel slices per area.
- Assign by dominant skill: `dev-data`, `dev-quant`, `dev-backtest`, `dev-execution`, `dev-api`, `dev-ui`, `dev-core`, `cicd` (or any agent added later). Several of one type is normal.
- Parallel slices pay off only with worktrees. Otherwise sequence them and reuse one developer.

## 4. Verification tiers

| Tier | When | Who | What |
|---|---|---|---|
| **T0** | every step/slice | the developer (gate hook enforces it) | `just dev-gate`: format, lint, types, unit tests, build |
| **T1** | every step that changes what a user or caller sees | the developer | one quick look at the changed thing only (below), stated in the handoff |
| **T2** | **once per feature**, after the last slice | one tester | the full matrix for the feature (below) |

- **T1 (slice):** one quick look at what changed: quant/backtest/data/execution -> run the changed unit once on a
  fixture (one targeted test or one `just cli-fixture` command) and compare to the reference or expected value; API ->
  one in-process router test (`oneshot`) for the changed route; UI -> one screenshot of the changed screen in
  `just app-dev` with the MSW mock API. No suites, no matrices.
- **T2 (feature):** acceptance criteria end to end on fixtures; full gate + `just check`; `just lookahead` and
  property suites; `just golden` (backtest stats diff vs committed baseline, explained if changed); API auth, CSRF and
  session negative cases on changed routes; UI Vitest on the MSW mock API (Playwright runs in CI; read the PR checks); paper-trading flow from
  recommendation to approval to fill with charges debited; quant-reviewer / risk-guardian findings closed; the full
  "not verified" list.

Slices **never** run T2. A slice handoff says "T0+T1 done; T2 pending". The T2 report is the only place for the
long "not verified" list. Don't re-run checks a previous tier already passed on unchanged code.

## 5. Dispatch prompt (the only format; manager -> any agent)

```
Task: <id> <one-line goal>      Size: S|M|L
Plan: <ABS_REPO>/.agents/plans/<id>.md (slice <n>)     | S: 5-line brief here instead
Branch: just new-branch feat/<id>-<slug>    (in-place: one developer at a time)
Scope: files/modules you may change
Acceptance: <= 3 bullets
Verify: T0 | T0+T1    (never T2 in a slice)
Output: <path printed by `just review-plan`> (reviewers only; file header names branch/PR)
Return: verdict, blockers, file path; handoff <= 15 lines, with "Where to look"
```
Owner rules and work mode are added to every agent by a hook; agents already have skills and CLAUDE.md. Point to
paths, never paste content.

**Reuse over cold start.** Continue an existing agent (SendMessage to its id) for fix rounds, the next step or slice in
the same area, and command-only steps (`just ship`, rebase). A fresh launch is only for parallel work or a different
area: a cold start re-reads the project and its skills (~15-25% of a slice).

**One channel per message.** A hand-back is read once, from its completion; the manager doesn't relay or restate it,
and doesn't re-read files it just wrote or merge-check output it already holds. Handoffs carry verdict, blockers and
a file path; details live in files.

**Checkpoints.** With `checkpoint_commits | yes`, developers commit on their own branch whenever T0 passes.
"Undo that step" is then `git revert <sha>` on that branch, not a new agent run.

## 6. Owner rules and "same class elsewhere"

When the owner corrects something that could recur ("no pinned scroll", "cards must fit the screen",
"amounts show two decimals"):
1. Add one line to `.agents/owner-rules.md` (max 20; merge old ones). A hook gives it to every agent from then on.
2. Search the **whole product** for the same pattern once (one grep or one measurement), not only the place named.
3. Fix every instance in one dispatch. Never fix only the place that was named.

## 7. Feature records and bugs

**Ids.** Features `F<n>`, bugs `B<n>`, numbered after the highest existing (`just features`, `just bugs --all`).
Branches carry the id: `feat/F12-orders`, `fix/B7-null-total`.

**Feature record** (`docs/features/F<n>-<slug>.md`, template `.agents/templates/feature-record.md`): drafted in the
plan; the first PR commits it (`Status: building`); later PRs add *What was built*; the last sets `Status: done`.
Deferred findings and known limits go here. Later changes add a *History* line. merge-check requires it.

**Bug intake (manager):**
1. `just bug-new "<title>" --feature F<n> --severity <critical|high|medium|low>`; fill what/expected/reproduce from the user's words.
2. critical/high: tell the user at once, fix before feature work. medium/low: board row, ask whether to fix now.
3. One developer on `fix/B<n>-<slug>`: **failing regression test first, then the fix**; the PR commits `docs/bugs/B<n>-*.md`
   with root cause, fix, PR and regression test, and a *History* line in the feature record.
4. Normal reviews; merge-check requires the bug record. Won't-fix/duplicate: commit the record with that status.

**Questions about a feature:** `just feature F<n>` and the codemap first; read only the files it lists.

**Older features without records:** the tech lead lists them (git log, codemap, CHANGELOG) with ids; the user
confirms; one developer writes the records on `docs/F0-feature-records` (code review only).

## 8. Triage rules (tech lead)

| Finding | Default |
|---|---|
| security critical/high, blocker, failing acceptance test, missing unit tests | fix now, blocks merge |
| major | fix now unless deferral is cheaper and safe (then into the feature record) |
| minor/nit | same fix round if cheap, else defer |
| developer disputes | tech lead decides in the round, reason in the findings file |

**Status is checked, never remembered:** "green / ready / CI passed" only from `just merge-check` (or `ship`) run just
now; `--wait` polls CI in the script. Every PR or branch gets the reviews `review-plan` lists before merge, including
fix-round updates (light lane: title and CI only unless `light_lane` says `quick`). `just status` = preflight, open
board rows and merge-check per open PR in one call.

## 9. Budget (measured automatically)

- Run `just budget-task <id>` when work on a task starts: hooks then measure every agent's real usage from its
  transcript and book it there; the manager's own session is measured too. Don't `budget-add` measured runs.
  Use `budget-add ... --estimate` only for a run the hooks could not see (it is shown separately).
- Before a batch: `just budget-check <id> dev-data:2 code-reviewer:2`. OK -> go. WARN -> go and tell the user.
  OVER LIMIT -> ask the user (raise, fewer agents, lower effort, smaller scope).
- `just budget` shows OVER AGENT CAP runs; give that agent a smaller next scope or ask to raise its cap.
- **Efficiency principles:**
  - Cost scales with risk: size (§2a), light lane, reviews by path sensitivity, T2 once.
  - Scripts poll, agents think: `--wait`, `ship`, `status` replace agent turns.
  - Reuse over cold start; one `review-pack` per review.
  - Plan reviews before dispatch; batch pushes (one CI run per round).
  - One channel per message: no relays, restating or re-reading; point to paths.
  - Read narrowly: `just status`, codemap, diffs; previews before taste work.
- Report as: `agents X (measured) + manager Y` from `just budget-report <id>`.

## 10. Code map, approvals, hiring

- The codemap refreshes at session start and after merges/pulls/pushes/branch changes; `just codemap` forces it. `just ship` pulls the integration branch after a merge (else `git switch "$(just base)" && git pull --ff-only`).
- Architecture changes update `docs/architecture.md` (architect) and `.agents/decisions.md` (tech lead) in the same feature.
- Ask the user before (team.md -> Approvals): starting a plan (if enabled); building look-and-feel work without a picked preview (if enabled); every promotion (always); new dependencies/stores; going over budget (always); real data, credentials, production, deleting data (always).
- Hiring: follow `agent-hiring`. New agents are found by their descriptions after the next session start.

## 11. Report to the user (manager)

Fixed compact format, status line first:
```
## <id> - <done | blocked | needs approval>
Delivered: PRs/branches, what changed (3-6 bullets)
Where to look: <from the handoffs: page/route/command + what to do, one line each>
Quality: T0/T1 per slice, T2 <passed|n/a>; findings fixed/deferred counts
Release: in release PR vX.Y.Z | released vX.Y.Z | not yet
Budget: agents <measured> + manager <measured>; task <used>/<limit>
Needs from you: approvals, .env key names to fill, questions
```

**Retrospective (L work, or any task over budget or with rework):** append 5 lines to the report from
`.agents/templates/retro.md`: tokens by phase, rework tokens, what the owner corrected, what to change next time,
and any new owner rule. Without this nothing can be measured or improved.
