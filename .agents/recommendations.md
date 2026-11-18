# Efficiency recommendations for manager + sub-agent kits

Stack-neutral. Written after running a real feature with the kit (a Rust API + React UI project, 2026-11-18), but
every item is phrased so it applies equally to a frontend-principal kit, a dbt + Snowflake kit, a Python service kit, etc.
Where a stack makes the point concrete, the stack examples are in "e.g." brackets; the one-line origin of each lesson is in the
appendix. Nothing here has been implemented. Numbers come from the kit's own budget report.

## 0. Evidence (one measured project; treat as a pattern, not a law)

| fact | value | meaning |
|---|---|---|
| agent runs / tokens for the feature so far | 34 runs, 1.64M | the sub-agents are cheap |
| cold-start share | 668k of 1.64M (**40%**) | 4 of 10 agent tokens go on re-reading context at launch |
| tester | 6 runs, 69% of tokens were start-up | mostly paid to get ready, then re-ran what the developer and CI had run |
| code reviewer / security reviewer | start-up 45% / 57% of their tokens | one launch per PR |
| planner/merger role | 2 runs, 186k | launched twice, each time to run one script |
| orchestrator (manager) | 303k context, **42.7M processed in a day** | every tool call re-sends the whole context; this dwarfs all sub-agent spend |

Ranking of levers: (1) orchestrator context size and turns, (2) cold starts, (3) rework loops from tool or kit defects.

## 1. Orchestrator context and turns (largest saving)

1. **Compact or restart at milestones.** After each merged wave of work, write state to the board and start a fresh session;
   the board carries the resume state. A 300k context re-sent hundreds of times is most of the cost story.
2. **Never put raw diffs, lockfiles or generated files in the orchestrator's context.** Review tooling excludes them and prints
   a one-line summary ("+41 packages: names/versions", "3 models changed, 1 contract changed").
   (e.g. dependency lockfiles, bundler output, snapshots, dbt `target/` and compiled SQL, generated API clients.)
3. **One script answers "what is the state of everything".** A compact table of all open PRs (mergeable, failing checks,
   review verdicts, sign-offs) instead of several per-PR queries repeated each turn.
4. **Wait in the background.** Long waits (CI, builds, warehouse jobs) run as background jobs that return only on a final
   result, not as repeated "still running" polls that each re-send the context.
5. **Fixed short status template for hand-backs** (id, verdict, blocker, next) with a link to the detail file; do not
   restate agent reports in prose to the user.
6. **Ask the user only real decisions,** batched, each with a recommended default so "yes" is one word. If a dialog tool
   misfires, repeat the question in plain text once instead of re-opening it.
7. **Show the user what stays theirs.** Keep a short list of "owner actions pending" (settings to toggle, approvals) on the
   board so they are not rediscovered each session.

## 2. Cold starts and review fan-out

8. **Reuse warm agents.** Continue the same developer for fix rounds and the same reviewer for re-reviews (this worked well).
   Go further: one code reviewer and one security reviewer per feature, fed each new PR via a message, so only the first PR
   pays the start.
9. **A tool-free "envelope" path for small, mechanical PRs** (a rules-only check in one call, no agent) and a size threshold
   based on *production* lines (exclude lockfiles, snapshots, generated code, docs). Verify the envelope's own prerequisites
   are installed, or it silently never runs.
10. **Tester once per feature, plus targeted runs on risk areas** (money/precision, auth, data contracts, destructive
    operations). A tester per PR mostly repeats the developer's gate and CI.
11. **One combined reviewer for tiny sensitive diffs** (a 10-line CI, config or manifest change) instead of separate
    code and security agents.
12. **Don't spend an agent on a script.** If the action is "run this command once the gate passes" (merge, tag, deploy
    to a non-production environment), let the orchestrator run it, or keep one warm agent for the whole feature.
13. **Parallelise only slices big enough to repay a second cold start** (rule of thumb: >150 production lines each);
    otherwise continue one developer sequentially.
14. **Dependency-manifest changes do not always need full review.** If a PR adds only dependencies already approved in the
    plan, treat it as routine; reserve full review for new dependency classes.

## 3. Context shared or read twice

15. **One review pack, one read.** Reviewers get a single file: this slice's goal and acceptance criteria, earlier findings
    for this PR, the diff without generated files, and a short checklist. Not "read the plan, the profile, three skills,
    then the diff".
16. **Per-slice plan files** (goal, files, acceptance, contracts touched) beside the master plan. Developers and reviewers
    get their slice; the master stays for the planner.
17. **Plans and shared docs must be visible inside every isolated worktree** (commit before dispatch or link them), so
    agents do not need absolute paths into another checkout.
18. **Keep dispatch prompts short.** Per-role defaults (branch rules, reply format, verification tiers) live in the agent
    files. A dispatch carries: task id, slice file, branch name, and only the special warnings.
19. **Measure preload weight.** A small script that reports tokens of agent file + preloaded skills + project instructions
    per agent shows which to trim. Re-measure after each change.
20. **Skills: a ~60-line core plus on-demand references.** Core = rules, a "which reference to open" index, and the done
    checklist; examples, code sketches and long tables live in `references/` and are opened only when needed.
21. **Role digests.** A ~40-line "golden rules" digest in each developer agent file so agents do not open three skills
    just to be told the same rules.
22. **A "task type -> read only these files" index** in the shared handbook, so agents stop reading adjacent material.
23. **Pre-approve dependency classes in the plan** (type definitions, lint plugins, test utilities from already approved
    vendors; for dbt: packages from an approved list). Only a genuinely new runtime dependency needs a human.
24. **Short board rows** (~200 characters for "next step"); long notes go in the feature record or decisions file.

## 4. Tooling hygiene that prevents rework

25. **Isolation guards must allow review artifacts.** If workers run in isolated worktrees, they still need a sanctioned
    place to write findings that the gate reads; otherwise a human or the orchestrator copies files by hand and agents
    claim files exist that do not. Give the gate a "collect reviews from all worktrees" command.
26. **Validate that every command named in agent files and skills exists** (a "doctor" check). A missing command costs a
    failed call plus a fallback read in every agent that tries it.
27. **Command guards parse the actual command and branch,** not words in the text. Text matching blocks harmless commands
    (a `grep`, a quoted string, a merge inside a feature worktree).
28. **Offer the routine-merge permission at session start.** If merges are scripted and the human approved the workflow,
    offer a one-time permission rule for the merge script, written by a tool the human runs, so sub-agents are not blocked
    mid-run. Do not let agents edit permission settings themselves.
29. **Triage never auto-defaults a decision.** Findings of type "question/decision" (design, wording, policy, precision or
    retention rules) are always listed for the orchestrator or owner; only mechanical fixes are auto-routed.
30. **PR-scoped finding ids** (`CR-12-1`, not `CR-1` reused in every PR) so a fix list is unambiguous.
31. **One source of truth for output paths:** agents write where the pack says; the pack is the only place a path is defined.
32. **Keep the test baseline green.** A suite with known failures hides regressions ("10 before, 11 after"). Fix it or mark
    known failures explicitly.
33. **Create the feature record at plan approval,** so status and feature queries work from the first slice.
34. **Budget commands should default sensibly** (agent list from the plan) and not fail on a bare call.
35. **Merge policy is a setting, not a hard-coded flag.** Things the owner may have opinions on (delete branches after merge,
    squash vs merge commit, auto-merge) must be read from team settings with a safe default and recorded as owner rules
    the first time the owner corrects them.
36. **Verification that needs a running app (screenshots, dev servers, warehouse connections) has a stated substitute**
    when agents are forbidden from human-only recipes (render tests, mock servers, a dry-run or `--empty` build), so a
    developer does not stop and ask.

## 5. CI and supply-chain hygiene (kit templates)

37. **Guard every optional CI job on the file or artefact it needs** (skip until the generator, schema, Dockerfile or
    models directory exists). A drift check that runs before its generator exists fails every PR.
    (e.g. generated API types, dbt docs/contract checks, GraphQL codegen.)
38. **Lint workflow files in preflight** (e.g. `actionlint`) so an invalid workflow is caught before the first push; a
    syntactically rejected workflow fails silently on every push.
39. **Slow or scarce runners (macOS, Windows, large warehouses) are push-only or manual by default,** with the fast Linux/small
    job as the PR gate; avoid `cancel-in-progress` on jobs that queue for a long time, which turns a queue into a "failure".
40. **Pin the package-manager version in the manifest** (e.g. `packageManager`, `required-version`, `.tool-versions`) so CI
    setup does not fail on "no version specified".
41. **Documented fallback when the hosting API cannot update a PR branch** (token without workflow scope): merge the base
    into the branch with plain git in the PR's worktree, by the developer who owns the branch.
42. **Advisory policy for "unmaintained" (not vulnerable) dependencies:** a dated, reasoned allow-list entry with an owner
    note plus a follow-up decision record, not a silent ignore and not a blocking failure.
43. **Install scripts and release-age guards for package managers** (an allow-list for build scripts, a minimum release
    age) and frozen-lockfile installs in every recipe and CI step.
44. **Add a "restore deleted branch" note** to the git skill: a merged PR's head commit can be recreated by API, so an
    accidental branch deletion is recoverable.

## 6. What worked (keep)

- Same developer for fix rounds and the same reviewer for re-review: no second cold start.
- Scripted triage and one merge gate: nobody argued with it.
- Owner rules plus a board as resume state: a session can stop and restart without re-explaining.
- One fix push per round: no PR needed more than one round.
- Short, batched decisions with a recommended default.
- Writing the owner's correction as a standing rule at once, then fixing the cause in the kit rather than just the instance.

## 7. Suggested order of work (any kit)

| order | change | effort | typical saving |
|---|---|---|---|
| 1 | tester once per feature (a setting) | XS | ~150k per feature (measured project) |
| 2 | review-pack without generated files + a working envelope + production-line thresholds (items 2, 9) | S | a reviewer pair per small PR; manager context |
| 3 | collect reviews from worktrees, doctor check for missing commands, PR-scoped ids, one path source (25, 26, 30, 31) | S | ~6 manual copies and failed calls per feature |
| 4 | guard parsing, routine-merge permission offer (27, 28) | S, human changes hooks/permissions | ~8 failed calls per feature |
| 5 | background waits and a one-table PR status (3, 4) | S | many orchestrator turns |
| 6 | warm reviewers per feature; orchestrator runs the merge script (8, 12) | M | ~300k per feature |
| 7 | per-slice plans, lean preloads, skill core + references (16-22) | M | 20-30% of cold-start cost |
| 8 | CI template fixes (37-40) | S | avoids ~3 PRs on every new project |

## 8. Checklist when generating a new stack kit

- Does every command named in agent files exist? (item 26)
- Do review artifacts have a writable, collected path under isolation? (25)
- Is there a stack-appropriate generated-artefact summary instead of raw diffs? (2)
- Are CI jobs guarded on their prerequisites, workflows linted, slow runners manual? (37-39)
- Does the plan template carry slice files, pre-approved dependency classes, and a human-only-recipe substitute? (16, 23, 36)
- Are merge policy choices settings with safe defaults? (35)

## 9. Measure

Track cold-start share per task (target below 25%) and orchestrator context size at each milestone; re-run after each change
and keep only changes that move these numbers. Estimates above are extrapolated from one project; verify before committing
to effort on any single item.

## Appendix: where each lesson came from (Bazaar F1, for traceability only)

| items | origin |
|---|---|
| 2, 9, 14 | `just envelope` said "claude not found on PATH" on every PR; `review-plan` counted lockfiles (2742 lines, 288 production) |
| 10 | `tester_review = large` launched a tester per PR; 6 runs, 69% start-up |
| 25, 31 | tester/reviewer findings could not be written into the main checkout from a worktree (5 PRs), and two reviewers used other paths |
| 26 | `just docs` referenced by CLAUDE.md and agent files but missing; three developers reported it |
| 27, 28 | `branch-guard.sh` blocked text mentioning `develop` / `gh pr merge`; `just ship` denied twice for the tech lead |
| 29, 30 | `just triage` auto-routed ceilings/placeholder-wording questions; ids `CR-F1-1` repeated across PRs |
| 32 | `kit-smoke.sh` had 10 pre-existing failures |
| 35, 44 | `just ship` hard-coded `--delete-branch` (fixed in PR #17); five branches were restored via the API |
| 36 | dev-ui could not take a screenshot (dev server is a human-only recipe) |
| 37, 38, 39 | `openapi-drift` ran before the generator existed (PR #16), `release.yml` had `hashFiles()` in a job `if` (PR #18), macOS queue and cancellations (PR #18, #19) |
| 40, 41 | pnpm `packageManager` missing on #13; `gh pr update-branch` refused for workflow-touching PRs |
| 42 | `cargo deny` advisories for unmaintained crates handled by dated ignores |
| 43 | security review of the web scaffold (no install-script or release-age guard) |
