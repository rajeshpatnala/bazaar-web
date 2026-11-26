# Team configuration

This file is the single place for every setting. Edit a value, then run `just agents-sync` and start a
new Claude Code session (or ask the manager, e.g. "set dev-data effort to high").
`-` means "use the Claude Code default". **Full guide with examples and recipes: `AGENT-SETTINGS.md`** in the repo root;
`just agents-settings` prints every setting and its allowed values.
New agents you hire get their rows here automatically.

## Agents

| agent | model | effort | max_turns | max_tokens_per_run |
|---|---|---|---|---|
| manager | claude-sonnet-5-5 | medium | - | - |
| tech-lead | claude-sonnet-5-5 | high | 40 | 200,000 |
| architect | claude-sonnet-5-5 | high | 30 | 200,000 |
| dev-data | claude-sonnet-5-5 | medium | 80 | 400,000 |
| dev-quant | claude-sonnet-5-5 | medium | 80 | 400,000 |
| dev-backtest | claude-sonnet-5-5 | medium | 80 | 400,000 |
| dev-execution | claude-sonnet-5-5 | medium | 80 | 400,000 |
| dev-api | claude-sonnet-5-5 | medium | 80 | 400,000 |
| dev-ui | claude-sonnet-5-5 | medium | 80 | 400,000 |
| dev-core | claude-sonnet-5-5 | medium | 80 | 400,000 |
| tester | claude-sonnet-5-5 | medium | 40 | 300,000 |
| code-reviewer | claude-sonnet-5-5 | medium | 15 | 150,000 |
| security-reviewer | claude-sonnet-5-5 | medium | 15 | 150,000 |
| cicd | claude-sonnet-5-5 | medium | 40 | 300,000 |
| build-runner | claude-sonnet-5-5 | medium | 10 | 100,000 |
| market-strategist | claude-sonnet-5-5 | high | 40 | 250,000 |
| risk-guardian | claude-sonnet-5-5 | high | 40 | 200,000 |
| quant-reviewer | claude-sonnet-5-5 | high | 40 | 200,000 |
| prototyper | claude-sonnet-5-5 | medium | 30 | 150,000 |
| combined-reviewer | claude-sonnet-5-5 | medium | 6 | 80,000 |

- `model`: sonnet, opus, haiku, fable, a full model id (e.g. claude-sonnet-5-5), or inherit.
- `effort`: low | medium | high | xhigh | max - how hard the agent thinks per step.
- `max_turns`: most steps (file reads, commands, edits) per task; the agent is stopped there. The hard limit.
- `max_tokens_per_run`: token cap per launch. Claude Code can't stop an agent mid-run on tokens, so the
  manager enforces it between launches: it warns you when a run goes over and plans the next launch to fit.
  Defaults are set near real usage (a typical developer slice uses 50-150k); raise one only when an agent keeps hitting it.

## Agents - advanced

| agent | context_window | cache_ttl | memory | permission_mode |
|---|---|---|---|---|
| manager | 160,000 | 1h | project | - |
| tech-lead | - | - | project | - |
| architect | - | - | project | - |
| dev-data | - | - | project | - |
| dev-quant | - | - | project | - |
| dev-backtest | - | - | project | - |
| dev-execution | - | - | project | - |
| dev-api | - | - | project | - |
| dev-ui | - | - | project | - |
| dev-core | - | - | project | - |
| tester | - | - | project | - |
| code-reviewer | - | - | none | - |
| security-reviewer | - | - | none | - |
| cicd | - | - | project | - |
| build-runner | - | - | none | - |
| market-strategist | - | - | project | - |
| risk-guardian | - | - | none | - |
| quant-reviewer | - | - | none | - |
| prototyper | - | - | none | - |
| combined-reviewer | - | - | none | - |

- `context_window`: 100,000-1,000,000 tokens; the conversation is summarised when it reaches this size.
  Smaller = cheaper turns on long tasks. The manager defaults to 160,000: its whole conversation is re-sent on
  every step, so a long one is the biggest cost; the board carries the state across summaries and fresh sessions. Needs Claude Code 2.1.296+.
- `cache_ttl`: 5m | 1h - how long the agent's prompt cache lives. 1h makes repeat launches within an hour
  cheaper (ignored while on usage credits). The manager defaults to 1h: it often waits longer than 5 minutes for
  agents or for you, and with 5m its whole conversation is written to the cache again after every pause.
- `memory`: project | user | local | none - where the agent keeps notes between runs.
- `permission_mode`: default | manual | acceptEdits | auto | dontAsk | plan. `bypassPermissions` is not
  allowed by the kit. The safety hooks run in every mode.

## Session (applies to every agent; written to `.claude/settings.json`)

| setting | value |
|---|---|
| max_output_tokens | - |
| max_thinking_tokens | - |
| subagent_model | - |
| max_parallel_agents | - |
| compact_at_percent | - |
| context_window | - |
| file_read_max_tokens | - |
| command_output_max_chars | - |

- `max_output_tokens`: longest single reply, in tokens (model default; higher leaves less room before summarising).
- `max_thinking_tokens`: thinking budget per reply on models with a fixed budget.
- `subagent_model`: model for any agent whose `model` is `-`.
- `max_parallel_agents`: agents running at the same time (Claude Code default 20).
- `compact_at_percent`: 1-100; summarise earlier, at this % of the context window (can only lower it).
- `context_window`: 100,000-1,000,000; one window for every agent, overriding `context_window` above.
- `file_read_max_tokens`: largest chunk one file read returns.
- `command_output_max_chars`: largest command output an agent sees (default 30,000, max 150,000).

Some of these only take effect when Claude Code is restarted; a few may need setting in your shell
instead (then `just doctor` still shows them here as documentation).

## Parallelism (read by the manager)

Work is sized first (S / M / L, see `delivery-process` -> Size ladder). Only L work is split into parallel slices;
then there is no fixed cap on developers - the budget is the governor.

| setting | value |
|---|---|
| work_mode | worktrees |
| max_review_rounds | 3 |
| max_slice_changed_lines | 400 |
| small_max_files | 3 |
| checkpoint_commits | yes |
| preview_first | yes |

| reuse_roles | tech-lead, architect, market-strategist |
| reuse_window_minutes | 90 |
| reuse_max_context | 120,000 |
| efficiency_review | yes |
| reuse_max_steps | 60 |
| min_slice_lines | 150 |
| heavy_builds_at_once | 1 |
| build_jobs | - |
| build_wait_minutes | 30 |
| rustc_wrapper | auto |
- `heavy_builds_at_once`: compiling commands (cargo build/clippy/test, or a step marked `"heavy"`) that may run at the
  same time across the checkout and every worktree; the rest wait for a free slot. `1` keeps parallel agents from
  running the machine out of memory; `0` = no limit.
- `build_jobs`: cap on cargo's parallel jobs (and test threads) for kit-run builds; `-` = cargo's default (one per
  CPU). A lower number uses less memory per build.
- `build_wait_minutes`: how long a gate waits for a build slot before it stops and says another build is running.
- `rustc_wrapper`: `auto` = use sccache when it is installed (crates compiled once, shared by every worktree);
  `sccache` = always; `none` = never. Any other wrapper comes only from your own `RUSTC_WRAPPER` environment variable.
- `work_mode`: `worktrees` (developers in parallel, each in its own git worktree) or `in-place` (one developer at a
  time in this checkout, for repos where worktrees fail). `just preflight` recommends one; `just work-mode <mode>` switches.
- `small_max_files`: work touching at most this many files in one area is size S (no plan, one developer, gate only).
- `checkpoint_commits`: `yes` = developers commit on their own feature branch whenever the gate passes, so any step can
  be undone with `git revert` instead of another agent run. Never pushed to protected branches.
- `preview_first`: `yes` = work judged by look or feel (or otherwise a matter of taste) gets a cheap preview the owner
  picks from before anything is built.

## Review policy (read by `just review-plan`, `review-pack` and `merge-check`)

Code review runs on every PR. Security review and testing scale with risk so small PRs stay cheap.

| setting | value |
|---|---|
| security_review | sensitive-or-large |
| tester_review | feature |
| large_pr_lines | 150 |
| tester_min_source_lines | 20 |
| review_pack_max_diff_lines | 1200 |
| require_feature_record | yes |
| require_bug_record | yes |

| light_lane | skip |
| light_review_docs | yes |
| envelope_reviews | yes |
| envelope_max_lines | 400 |
| envelope_max_diff_chars | 60000 |
| envelope_command | claude |
| require_feature_t2 | yes |
| review_rounds | fixes-only |
| defer_minor_after_round | 1 |
| nit_policy | defer |
| unreviewed_fix_max_lines | 20 |
| security_block_severities | critical, high, medium |
| require_reviewed_commit | yes |
`security_review`: `always` or `sensitive-or-large` (auth, config, SQL/migrations, dependency/lock files, CI, Dockerfile, or >= large_pr_lines).
`tester_review`: `feature` = the full test matrix runs **once per feature** (T2 after the last slice); a PR gets its own
tester run only when it touches a risk area (money maths, auth, migrations, destructive operations: `tester_risk_globs`
in `.agents/stack.json` plus kit defaults) with >= tester_min_source_lines production lines. Older options: `always`,
`behaviour-change` (every feat/fix/perf PR with >= tester_min_source_lines), `large`.
`require_feature_record`: `yes` = a feat/fix/refactor/perf PR for feature F<n> can't merge until `docs/features/F<n>-*.md` exists (on the PR or already merged).
`require_bug_record`: `yes` = a `fix/B<n>-...` PR can't merge until it commits `docs/bugs/B<n>-*.md` with `Regression test:` filled.
## Context (token use)

| setting | value |
|---|---|
| preload_all_stack_skills | no |

`no`: tech lead and reviewers preload only their process skills plus the core skill and load others on demand (much cheaper). `yes`: they preload every developer skill. Run `just agents-sync` after changing.

## Budget (read by `just budget*` and the manager)

Token counts come from each agent's reported usage. `unlimited` or `-` disables a limit.
Set `usd_per_million_tokens` to your blended rate to see dollar estimates.

| setting | value |
|---|---|
| per_task_tokens | 5500000 |
| per_day_tokens | 15,000,000 |
| warn_percent | 80 |
| usd_per_million_tokens | - |
| manager_clear_at | 150,000 |
| budget_measure | tokens |

## Branches

Feature PRs target the integration branch; the tech lead merges them there once `just merge-check <pr>`
passes (reviewers' verdicts, CI checks, target branch). The release branch only moves through
`just promote <pr>`, which Claude Code asks you to approve every time.

| setting | value |
|---|---|
| integration_branch | develop |
| release_branch | main |
| require_ci_checks | yes |
| delete_merged_branches | no |

| require_merge_approval | yes |
| records_extra_globs | docs/strategy/**, .claude/agent-memory/** |
## Approvals (manager asks the user before)

| checkpoint | required |
|---|---|
| plan (before developers start) | yes |
| pick a preview before building look-and-feel work | yes |
| merge feature PRs into the integration branch | no (tech lead merges when merge-check passes) |
| promote to the release branch / merge the release PR | always (Claude Code asks you) |
| new dependency or new data store | yes |
| exceeding the budget (task, day, or an agent's max_tokens_per_run) | always |
| real data stores, credentials, production, deleting data | always (cannot be disabled) |
