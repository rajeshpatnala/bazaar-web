---
name: delivery-process
description: How the manager and tech lead run work with sub-agents - preflight, intake lanes, S/M/L sizing, lifecycle, reuse over cold starts, dispatch format, reviews, scripted triage, waiting and merging by script, owner rules. Details (records and bugs, previews and verification tiers, budget and reports, files and modes) are in references/. Preload for manager and tech lead.
---

# Delivery Process (manager + tech lead)

**Rule of thumb:** before launching an agent, ask whether a script, an agent that is already warm, or your own command
can do the step. Launch a fresh agent only when the step needs judgment no warm agent has.

## 0. Preflight (once per session)
`just status` first (checkout, work mode, budget, board, open PRs with CI, bugs). If a branch is missing or worktrees
fail, `just preflight` prints the fixes; offer them to the user. **Never swap a kit agent for a general-purpose agent**
to get around a failure. Work modes, the state files and approvals: `references/files-and-modes.md`.

## 2. Intake: lane, then size
| Lane | Signs | What happens |
|---|---|---|
| Question | what/how/why/does it | answer from codemap, `just feature`, records; no agent unless it needs investigation |
| Bug | broken, wrong, error | §7 |
| **Preview first** | judged by look or feel, or a matter of taste | §2b: no slices until the owner picks |
| Work | everything else | size it (§2a), then §3 |

Tell the user lane, size and token estimate in one line before planning ("M, one developer, ~250k; plan next?").

### 2a. Size ladder
| Size | Rule of thumb | Plan | Developers | Reviews |
|---|---|---|---|---|
| **S** | <= `small_max_files` files, one area | manager's 5-line brief | 1 | what `review-plan` lists |
| **M** | one feature, one area | ONE plan file <= 40 lines | 1, continued between steps | at the end, per PR |
| **L** | several areas, parallel pays off | full plan with slices | one per independent area (worktrees mode) | per PR, pipelined |
When unsure, pick the smaller size. A plan costing more than ~3x what the visible change needs gets shrunk or justified.

### 2b. Preview first
One question at most, answerable by pointing; then `prototyper` makes <= 3 previews (cap 150k); the owner picks; the
pick becomes owner rules and acceptance criteria. Details: `references/preview-and-verify.md`.

## 3. Lifecycle
```
1 Intake    lane + size + estimate; `just budget-task <id>`
2 Plan      S: manager brief. M/L: tech lead, ONE plan file (L adds slices) incl. feature record draft.
            Architect only for new services/stores, cross-cutting or irreversible choices
3 Approve   plan summary + `just budget-check` to the user (if plan approval = yes)
4 Build     M: one developer, continued. L: one developer per independent slice. T0+T1 per step (§4)
5 Review    at each hand-off: small PR -> `just envelope <pr>` first: every required review in one
            tool-free call, findings written to the usual files (re-reviews too). Exit 3 or a big PR ->
            `just review-plan <pr>` and launch exactly the reviewers it names, each with the output path
            `review-pack` prints. Docs-only PRs: `light` (no reviewer). The tester still runs when required
6 Triage    `just triage <id> --ref <pr-N|branch>` applies the defaults (§8). Fixes go straight to the
            developer; the tech lead only for what it lists under "needs tech lead"
7 Fix loop  continue the SAME developer with finding ids; re-check by the same reviewers; one push per round
8 Verify    after the last slice: ONE tester runs T2 for the whole feature
9 Merge     tech lead (continued, not relaunched): `just ship <pr>` - waits for CI in the script,
            merge-check, merge, updates the integration branch and codemap
10 Release  user asks -> promotion PR -> `just promote` (asks the user) -> release PR -> tag
11 Report   fixed format (references/budget-and-reports.md); retrospective for L, over-budget or rework
```
Slicing (L): independently mergeable, owns distinct files, <= `max_slice_changed_lines`; slice 0 = shared contracts.
Assign by dominant skill: `dev-data`, `dev-quant`, `dev-backtest`, `dev-execution`, `dev-api`, `dev-ui`, `dev-core`, `cicd`, or an agent added later.

## 4. Verification tiers
T0 every step (gate hook), T1 one quick look at the changed thing, T2 the full matrix once per feature by the tester.
Slices never run T2; never re-run a tier that already passed on unchanged code. Stack specifics: `references/preview-and-verify.md`.

## 5. Dispatch (the only format)
```
Task: <id> <one-line goal>      Size: S|M|L
Plan: <ABS_REPO>/.agents/plans/<id>.md (slice <n>)   | S: 5-line brief here
Branch: just new-branch feat/<id>-<slug>
Scope: files/modules you may change
Acceptance: <= 3 bullets          Verify: T0 | T0+T1
Output: <exact path from review-pack>   (reviewers only)
Return: verdict or handoff <= 15 lines, blockers, file path, "Where to look"
```
Owner rules, work mode, skills and CLAUDE.md reach every agent already: never restate them. Point to paths.

**Reuse over cold start.** Every fresh agent re-pays its whole start (Claude Code's prompt, CLAUDE.md, preloaded skills,
memory) - often more than the work. Continue the existing agent (SendMessage to its id) for fix rounds, the next step
in the same area, triage questions and command-only steps (`just ship`, rebase). The tech lead who planned or merged is
the one you continue. A hook blocks the first fresh launch of a `reuse_roles` agent that already ran this session.
Fresh launches are for parallel work or a different area. `just budget-report <id>` shows start vs work per run.

**Hand-backs:** read once; pass on the verdict and the path, never restate or relay the text; don't re-read files you
just wrote. **Checkpoints:** developers commit on their branch whenever T0 passes, so undo = `git revert`.

## 6. Owner rules
A correction that could recur: one line in `.agents/owner-rules.md` (max 20; a hook gives it to every agent), search
the whole product for the same pattern once, fix every instance in one dispatch.

## 7. Records and bugs
Features `F<n>`, bugs `B<n>`; feature records in `docs/features/`, bug records in `docs/bugs/`, both required by
merge-check. Bug intake: `just bug-new "<title>" --feature F<n> --severity <...>`; fix = failing regression test first.
Feature questions: `just feature F<n>` first. Full procedure: `references/records-and-bugs.md`.

## 8. Triage defaults (applied by `just triage`)
| Finding | Default |
|---|---|
| blocker, critical, high, major, medium; failing acceptance test; missing unit tests | fix now (blocks merge) |
| minor, nit, low, info | fix now if cheap; the developer may propose defer |
| disputed, conflicting fixes for one spot, no clear fix, unknown severity | **needs the tech lead** |
Deferrals and rejections are the tech lead's call and go into the feature record.

**Status is checked, never remembered:** "green / ready" only from `merge-check`, `pr-wait`, `ship` or `status` run
just now. `PENDING` = CI not finished, never a failure. **Waiting is a script's job:** never poll CI or chain merge
steps with agent turns. Developers push once per hand-off or fix round (every push restarts CI).

## 9. Budget
`just budget-task <id>` at the start; usage is measured automatically. Before a batch:
`just budget-check <id> dev-data:2 code-reviewer:2` (WARN: tell the user; OVER LIMIT: ask). Report
`agents X + manager Y` from `just budget-report <id>`. More: `references/budget-and-reports.md`.
