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
Plan review turns failure paths and "controls that must be disabled" into acceptance criteria, not developer
checklists: `references/plan-checklist.md`.

## 3. Lifecycle
```
1 Intake    lane + size + estimate; `just budget-task <id>`
2 Plan      S: manager brief. M/L: tech lead, ONE plan file (L adds slices) incl. feature record draft.
            Architect only for new services/stores, cross-cutting or irreversible choices
3 Approve   plan summary + `just budget-check` to the user (if plan approval = yes)
4 Build     M: one developer, continued. L: one developer per independent slice. T0+T1 per step (§4)
5 Review    `just next` says what each open PR needs. Small PR -> `just envelope <pr>` first: every required
            review in one tool-free call, findings written to the usual files (re-reviews too). Exit 4 (can't
            run here) -> one `combined-reviewer` with the prompt it prints. Exit 3 or a big PR -> launch
            exactly the reviewers `review-plan` names, ALL IN ONE MESSAGE (in parallel, each fresh, each with
            the path `review-pack` prints), so they come back together; in the background, `just review-wait
            <pr>` wakes you once. Risk/safety reviewers marked `envelope: false` always review their own
            paths. Docs-only PRs: `light`. The tester still runs when required
6 Triage    `just triage <id> --ref <pr-N|branch>` applies the defaults (§8) and approves only once every
            required review is in. The tech lead only for what it lists under "needs tech lead"
7 Fix loop  `just fix-list <pr>` IS the developer's prompt (only the fix-now findings + the fix-round rules);
            continue the SAME developer. Round 2+: `just fix-scope <pr>`, then only reviewers with open items
            re-check (fixes-only when review_rounds = fixes-only); a round with only minor/nit left from
            code/domain reviewers is closed by the tech lead reading the fix diff and `just verify-fixes`
8 Verify    after the last slice: ONE tester runs T2 for the whole feature
9 Merge     approval: `just triage` writes it when nothing needs judgment; else the tech lead decides and ends
            with `just approve <pr>`. Reviews and approvals name the commit they covered: a push after them
            means a re-review (merge-check says so). Then the MANAGER runs `just ship <pr>` in the background -
            no agent is launched just to merge. It waits for CI in the script, merge-check, merge, updates the
            integration branch and codemap
10 Release  user asks -> promotion PR -> `just promote` (asks the user) -> release PR -> tag
11 Report   fixed format (references/budget-and-reports.md); retrospective for L, over-budget or rework;
            after the feature's last merge `just efficiency F<n>` + <= 15 lines of observations (efficiency_review)
```
Slicing (L): independently mergeable, owns distinct files, `min_slice_lines`-`max_slice_changed_lines` estimated lines
(`est. lines` column; `just plan` flags misses); slice 0 = shared contracts and formulas every later slice uses. New
code stays unused until the slice that connects it; that slice is marked `live path: yes` and gets the safety review.
Ask the owner for every input the plan needs (data, calendars, rates, decisions) at plan time, in one batch.
Assign by dominant skill: `dev-data`, `dev-quant`, `dev-backtest`, `dev-execution`, `dev-api`, `dev-ui`, `dev-core`, `cicd`, or an agent added later.

## 4. Verification tiers
T0 every step (gate hook), T1 one quick look at the changed thing, T2 the full matrix once per feature by the tester.
A PR gets its own tester only when `review-plan` says "risk area" (money, auth, migrations, destructive operations);
then the tester checks that area only. `just promote` waits until every feature in it has a signed-off T2.
Slices never run T2; never re-run a tier that already passed on unchanged code.
Builds: the gate skips a step when the branch changed nothing it reads, or when it already passed on exactly these
inputs (any worktree); `--fresh` forces it. Compiling steps take one of team.md `heavy_builds_at_once` slots, so
parallel developers queue instead of running out of memory. Reviewers never build: they read the diff, and the head
commit's gate and CI results speak for it. Stack specifics: `references/preview-and-verify.md`.

## 5. Dispatch (the only format)
```
Task: <id> <one-line goal>      Size: S|M|L
Plan: just plan <id> --slice <n>   (M: just plan <id>; S: 5-line brief here) - works inside worktrees
Branch: just new-branch feat/<id>-<slug>
Scope: files/modules you may change
Acceptance: <= 3 bullets          Verify: T0 | T0+T1
Output: <exact path from review-pack>   (reviewers only)
Return: verdict or handoff <= 15 lines, blockers, file path, "Where to look"
```
Owner rules, work mode, skills and CLAUDE.md reach every agent already: never restate them. Point to paths.

**Reuse over cold start.** Every fresh agent re-pays its whole start (Claude Code's prompt, CLAUDE.md, preloaded skills,
memory) - often more than the work. Continue the existing agent (SendMessage to its id) for fix rounds, the next step
in the same area, triage questions and command-only steps (rebase). Never launch or continue an agent to run `just ship`. The tech lead who planned or merged is
the one you continue. Continue only while an agent is small: `just warm` / `just next` say continue or start fresh
(`reuse_max_context`, `reuse_max_steps`: a long run costs more per step, so the next slice goes to a fresh developer
with `just plan <id> --slice <n>` as its brief). **Reviewers are fresh per PR** (a reused reviewer carries every earlier review pack: measured
~150k per review against ~80k fresh) and are continued only for re-reviews of the same PR. A hook blocks the first
fresh launch of a `reuse_roles` agent (tech lead, architect) that already ran on this feature.
Fresh launches are for parallel work or a different area. `just budget-report <id>` shows start vs work per run.

**Records and reviews:** the manager's own records (board, decisions, reports) reach the integration branch through `just records-pr` (no reviewer). Reviewers in worktrees end with `just save-review <path>` so findings survive the worktree. `just run-report <agent>` explains one run's cost.

**Hand-backs:** an agent's final reply IS its hand-back (agents never also send a message, which would wake you twice).
Read once; pass on the verdict and the path, never restate or relay the text; don't re-read files you just wrote. **Checkpoints:** developers commit on their branch whenever T0 passes, so undo = `git revert`.
**Milestones:** after a merged wave, a merged feature or a release, update the board and run `just milestone "<resume>"`;
then suggest `/clear` - the manager's conversation is re-sent on every step, the board carries the state. Things only
the owner can do go in the board's "Owner actions" list (`just status` shows them).

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
| minor, nit, low, info | fix now if cheap (security/safety findings: fix now) |
| a minor/nit the reviewer itself says belongs in a later slice | defer: that slice |
| a minor/nit first raised after round 1, outside the fix (`defer_minor_after_round`) | defer: next slice |
| disputed, conflicting fixes for one spot, no clear fix, unknown severity | **needs the tech lead** |
| a blocking finding, or a security/safety finding of any severity, that mentions a later slice | fix now; only the tech lead defers it, by name |
| `question` severity, or a finding that asks for a decision (design, wording, policy, limits) | **never auto-decided**: tech lead, or the owner for product calls |
Every deferral names a target and goes into the feature record's Deferred list: `just deferred <F>` shows the open ones,
and `just promote` refuses while one isn't carried. An open "fix now" finding of any severity blocks the merge until
its reviewer marks it fixed (minor/nit from code or domain reviewers: the tech lead may `just verify-fixes`).
Security findings at `security_block_severities` (default critical, high, medium) block while open.

**Status is checked, never remembered:** "green / ready" only from `merge-check`, `pr-wait`, `ship` or `status` run
just now. `PENDING` = CI not finished, never a failure. **Waiting is a script's job:** never poll CI or chain merge
steps with agent turns. Developers push once per hand-off or fix round (every push restarts CI).

## 9. Budget
`just budget-task <id>` at the start; usage is measured automatically. Before a batch:
`just budget-check <id> dev-data:2 code-reviewer:2` (WARN: tell the user; OVER LIMIT: ask). The owner's real plan usage
outranks the kit's caps: 70% tell the user, 85% finish the current slice and plan the rest with them, 95% stop and ask. Report
`agents X + manager Y` from `just budget-report <id>`. More: `references/budget-and-reports.md`.
