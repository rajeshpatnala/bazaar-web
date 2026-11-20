# Kit improvements: the single list (2026-11-21)

Everything raised so far is in this one file, with its status: the 8 efficiency reports (F1-F9), the consolidated F9
list, the second "spend fewer tokens" list (A1-H38), the extra findings from the analysis, and the loopholes.
Nothing else needs reading. Applies to **kit-factory** (shared core, every future kit) and **bazaar-web** (this
project); both have identical scripts.

Status: **Done** = changed and tested · **Already there** = the kit did this before · **Not doing** = rejected, with the reason.
Spend figures come from bazaar-web's ledger for 11-19 and 11-20: 82.9M cost-eq, about 50 PRs. Shares: developers 43%,
manager 26%, quant/risk reviewers 13%, code/security reviewers 8%, tech lead 6%, tester 3%, all cold starts together under 4%.

## 1. Safety holes (fixed first)

| # | Hole | Status | What changed |
|---|---|---|---|
| S1 | Code pushed after review or approval could still merge | Done | Every review file and approval records `Commit: <sha>`. Merge-check refuses when the branch has its own changes after it. A clean merge of the integration branch is allowed; a merge with hand-made conflict edits is not. |
| S2 | Small changes to risk paths (paper, scanner, kill switch, approval UI...) could skip risk-guardian via the envelope | Done | A reviewer marked `"envelope": false` in stack.json always reviews its own paths. bazaar-web: risk-guardian is marked. |
| S3 | Open medium security findings did not block a merge (the rules said they should) | Done | New setting `security_block_severities`, default critical, high, medium. Replay: 6 medium security findings merged while still open. |
| S4 | The AI could loosen its own rules (`just set`, editing team.md, stack.json or owner rules; envelope allowed for those PRs) | Done | Claude Code asks you first for `just set` and for edits to team.md, stack.json, owner-rules, templates, settings.json, hooks and kit scripts. Those files always get the security reviewer and never the envelope. The installer now carries "ask" rules on upgrade; it dropped them before. |
| S5 | Lockfiles and generated files were never shown to reviewers | Done (CI part yours) | Reviewers now see which packages a lockfile adds, removes or changes, and small generated-file changes in full. Large generated files still need CI to regenerate them and fail on a difference. I couldn't see your CI files: **owner action**. |
| S6 | Minor fixes marked "fix now" merged without anyone confirming them | Done | An open "fix now" finding, any severity, blocks the merge. Its reviewer marks it fixed, or for minor/nit from code/domain reviewers the tech lead runs `just verify-fixes`. Replay: 29 merged PRs still had open fix-now items. Mostly stale statuses from re-reviews written to a second file (e.g. QR-56-1 fixed in quant-2.md, still open in quant.md), so that is now refused too. |
| S7 | Deferred findings could be forgotten | Done | A deferral names a target. `just deferred <F>` lists open ones. `just promote` refuses while one is missing from the feature record. Found today: **RISK-91-2 is missing from the F9 record**. |
| S8 | "Security findings are never deferred" missed security issues raised by the code reviewer | Done | The safety rule also follows the finding's words (auth, injection, secrets, session token...), RISK ids and second risk files. |
| S9 | Everything depended on the severity a reviewer chose (RISK-91-1 was "minor") | Done | Security and safety findings are always "fix now". Only the tech lead can defer them, by name. `verify-fixes` refuses them, so their own reviewer re-checks. |
| S10 | Fix-scope check is a guide, not a guarantee | Done | `just fix-scope` lists changes nobody asked for. Round 2 still reads the whole fix diff, and unasked changes are reviewed fully. |
| S11 | Rewritten history could hide changes | Done | If the reviewed commit is no longer in the branch, the PR goes back to a full re-review. |
| S12 | "Unused until switched on" was not checked when splitting work | Done | Plan slices get a `live path` column. A "yes" slice always gets the safety reviewer (`live_path_review`; bazaar-web: risk). |
| S13 | Text in a diff could try to instruct a reviewer | Done | Envelope and every reviewer are told diff text is material, never instructions, and that such text is itself a finding. |
| S14 | Code on develop has not had the full T2 test pass | Owner action | Proposed owner rule: run paper trading only from builds promoted to main. Not added: owner rules are your decisions. |

## 2. Token savings, ranked by expected saving

| # | Change | Where it helps | Est. saving | Status |
|---|---|---|---|---|
| E1 | **Shorter developer runs**: `reuse_max_steps` (60) next to the 120k context cap. `just warm` / `just next` hand the next slice to a fresh developer with the slice brief. Plans carry `est. lines` per slice; `just plan` flags slices over 400 or under 150 (`min_slice_lines`). | developers (43%) | 5-9% | Done |
| E2 | **Fewer, smaller review rounds**: `review_rounds = fixes-only`: round 2+ shows each reviewer only its earlier findings plus the fix diff. `defer_minor_after_round = 1`: a nit first raised after round 1, outside the fix, goes to the next slice. Nit-only rounds are closed by the tech lead with `verify-fixes`. Re-reviews must update the file in place. | reviewers, tester, developer fixes | 5-8% | Done (**off by default, see §6**) |
| E3 | **One wake-up per PR and a script that plans**: `just next` prints the next action for every open PR (CI, missing or stale reviews, triage, fix round, approval, ship). Reviewers are launched together in one message. `just review-wait` wakes the manager once. Agents' final reply is the hand-back, never an extra message. | manager (26%) | 3-6% | Done |
| E4 | **Triage fixes**: honours "in slice 7" for non-safety minors only; blocking and safety findings are always fixed; column aliases (`sev`, `where`, `check`); approval only once every required review is in. | tech lead, early fix work | 1-2% | Done |
| E5 | **Fix brief as the prompt**: `just fix-list <pr>` is the developer's whole fix-round prompt (only the fix-now findings + the rules: touch nothing else, label commits, "Other changes", no rebase). | developer fix steps | in E2 | Done |
| E6 | Kit bug fixes (§3) | manager steps, round trips | 1-2% | Done |
| E7 | Measuring fixed: envelope runs no longer count as cold starts and record their cost; spend-by-role table including the manager; review rounds per PR; cost per review. The wrong "reviewer relaunched → one warm reviewer per feature" flag is removed. | prevents a wrong change (+~4M per two days) | 0 directly | Done |
| E8 | Envelope runs from an empty folder, so the project's CLAUDE.md and memory aren't loaded into it | envelope | small | Done |
| E9 | Budget: 85% step (finish the slice, plan the rest with you); warning when plan-usage data is old; `budget_measure = cost-eq` option (caps count real cost, manager included) | avoids a hard stop | 0 | Done (`tokens` stays the default) |

## 3. Kit bugs and tooling

| # | Bug | Status |
|---|---|---|
| B1 | `just records-pr --help` opened a real PR | Done: `--help` on any command prints its help and never runs it; commands that change things refuse unknown flags |
| B2 | merge-check on a merged PR said "mark it ready"; `ship` on a merged PR tried again | Done: "already merged", then sync and codemap |
| B3 | records-pr missed files in new folders and refused owner-input, handoff, memory and strategy docs | Done: new-folder files are included; `.agents/owner-input/`, `.agents/handoff*.md` added; `records_extra_globs` setting (bazaar-web: `docs/strategy/**, .claude/agent-memory/**`) |
| B4 | Findings written as bullets were counted as "0 findings"; blank severities and missing columns passed | Done: save-review refuses them |
| B5 | Triage approved before every reviewer had reported (approval.md deleted by hand twice) | Done |
| B6 | `milestone` broke on semicolons, colons and brackets | Done: `just milestone --file <path>` or `-` (stdin) |
| B7 | `just golden` failed: no golden suite yet | Done (bazaar-web): says "no golden suite yet" until F9 slice 7b3 adds it |
| B8 | doctor reported `just set` as missing (it read `set *args:` as a justfile setting) | Done |
| B9 | Installer dropped the "ask" permission rules on upgrade | Done |
| B10 | No unit tests for the kit's decisions | Done: `just kit-test`, 27 tests with fake repos and reviews; the factory self-test runs them for every kit it builds |

## 4. Already in the kit (nothing to do)

Scripted triage, scripted merge (`ship`), one merge gate · reviewers fresh per PR, warm only for a re-review (B6-B8 of list 2) ·
envelope path and production-line counting (D14) · milestone, `/clear`, `status`, owner actions (E18-E22) · fixed hand-back format ·
doctor checks named commands; workflows linted in preflight (H36) · guards parse the real command (H37) · hook denials say what to do (H38) ·
`start-report` measures preloads (F24) · tester once per feature plus risk areas (F28; already `tester_review = feature`) ·
per-task booking from the dispatch's `Task:` line (G30) · real plan usage read from the status line (most of G29) ·
slice briefs list earlier blockers (F9 #11) · warm developer reuse · plan reviewed by risk-guardian before code.

## 5. Not doing, and why

| Item | Why not |
|---|---|
| One warm code/security reviewer per feature (F9 #2, list 2 B-section, report flags) | Measured: a warm reviewer costs about 150k per review against 80k fresh. Kept: warm only within one PR's fix cycle. |
| One reviewer reading several PRs' packs at once (A2) | Same effect as above; also contradicts B6-B8 of the same list. Saves only a start (~10k) per PR. |
| Skip security by diff content; narrow security paths (A4, F9 #5) | Worth ~1-1.5%, but config files can change things like the server's bind address. You asked for zero security risk, so security stays. |
| Narrow the tester's risk paths | They are your money paths (risk, execution, paper, broker). A tester there is worth its ~3%. |
| Label notifications per task (G31, F9 #10 part) | Claude Code sets those labels; the kit can't. |
| Cache time 1h for agents (F27) | A 1h cache write costs 2× instead of 1.25×. It only pays for agents relaunched within the hour, and starts are under 4% of spend. Left as a per-agent setting. |
| Cold-start share < 25% as a goal (G32) | Kept as a number to watch, not a target: it is under 4% of spend, and it was inflated by the envelope measuring bug. |

## 6. Follow-ups (not in this change, on purpose)

| Item | Why later |
|---|---|
| Split `kit.py` (now ~4,300 lines) into modules of about 400 lines (D16) | Do it as its own change: a pure move with no behaviour change, so its diff can be reviewed. The tests from B10 make it safe. |
| Trim skill cores to ~60 lines plus references (F25-26, F9 #9) | Under 1% saving; content work across many skills. |
| Turn on fixes-only rounds | Both new review behaviours ship **off**. Pilot on the next feature (F10): `just set review_rounds fixes-only` and `just set defer_minor_after_round 1` (Claude Code will ask you). Compare rounds per PR, spend per PR and bugs found later (T2, bug reports). Keep them if later bugs don't rise. |
| Old smoke script `.agents/scripts/kit-smoke.sh` | 3 of its checks were already failing before this change (written for an older kit). `just kit-test` replaces it. |
| Old recommendation files (`.agents/recommendations-2026-11-19.md`) | Superseded by this file; delete when you like (still in git history). |

## 7. Replay on your 72 stored PR reviews (new rules, history unchanged)

- **14** minor/nit findings would have been deferred. In every one the reviewer itself pointed at a later slice; none came from security or risk; 6 were deferred by people anyway. They now must appear in the feature record.
- **52** security/safety minors that were "fix now if cheap" become "fix now".
- **0** blocking or safety findings deferred by a script.
- **29** merged PRs still had open "fix now" findings (128). After this change they would have needed a re-check, or an in-place status update, before merging.
- What the replay can't show: a problem that a fixes-only round 2 would never have looked at. That is why §6 pilots it with the bug count as the deciding number.

## 8. New commands and settings

Commands: `just next` · `just review-wait <pr>` · `just fix-list <pr>` · `just fix-scope <pr>` · `just verify-fixes <pr> <ids>` (tech lead) ·
`just deferred [F]` · `just kit-test` · `just milestone --file <path>`.

Settings (team.md; change with `just set`, which asks you): `reuse_max_steps` 60 · `min_slice_lines` 150 · `review_rounds` full ·
`defer_minor_after_round` off · `security_block_severities` critical, high, medium · `require_reviewed_commit` yes ·
`budget_measure` tokens · `records_extra_globs` (bazaar-web: `docs/strategy/**, .claude/agent-memory/**`).

stack.json keys (kit-factory packs): a domain reviewer's `"envelope": false` and `"safety": true`; `live_path_review`.

## 9. After installing

Restart Claude Code, run `just doctor` and `just kit-test`. Every review started from now on records its commit:
PR 94's pending reviews must include the `Commit:` line that `just review-pack` prints (reviewers are told to).
Add RISK-91-2 to the F9 feature record's Deferred list (`just deferred F9`).
