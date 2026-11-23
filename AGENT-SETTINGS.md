# Agent settings guide

Every setting for your agent team lives in one file: **`.agents/team.md`**. This guide explains each one, with examples.

## How to change a setting

1. Open `.agents/team.md` and change the value in the table.
2. Run `just agents-sync`.
3. Restart Claude Code (close it and open it again in your repo).

Or tell the manager in chat, for example *"set code-reviewer max_turns to 25"*. It edits the same file and runs step 2 for you; you still restart.

Good to know:
- `-` means "use the Claude Code default".
- Numbers can have commas: `150,000` and `150000` are the same.
- `just agents-settings` prints every setting with its allowed values.
- `just agents-check` tells you whether the agents match `team.md` (and points out typos).
- Wrong values are refused with a message saying what's allowed; nothing changes until you fix them.
- Upgrading the kit keeps your values.

Which settings need step 2 and a restart:

| Section | Run `just agents-sync`? | Restart Claude Code? |
|---|---|---|
| Agents, Agents - advanced, Session, Context | yes | yes |
| Review policy, Budget, Branches, Parallelism, Approvals | no (read live) | no |

---

## Agents (one row per agent)

```
| agent         | model             | effort | max_turns | max_tokens_per_run |
|---------------|-------------------|--------|-----------|--------------------|
| code-reviewer | claude-sonnet-5-5 | medium | 15        | 150,000            |
```

### `model`
Which Claude model the agent runs on.
- Values: `sonnet`, `opus`, `haiku`, `fable`, a full model id such as `claude-sonnet-5-5`, or `inherit` (same as the main session).
- Example - cheaper, faster reviewer: `| code-reviewer | haiku | ... |`
- Example - strongest model for planning: `| tech-lead | opus | ... |`

### `effort`
How hard the agent thinks on each step. Higher = better reasoning, more tokens, slower.
- Values: `low`, `medium`, `high`, `xhigh`, `max`.
- Example - developers on tricky work: `| dev-data | claude-sonnet-5-5 | high | ... |`
- Example - build-runner only runs commands: `| build-runner | claude-sonnet-5-5 | low | ... |`
- Defaults: `high` for tech-lead and architect (planning, triage, design); `medium` for the manager (it mostly routes,
  runs commands and relays, and takes the most turns of any agent) and everyone else. If the manager starts sizing or
  classifying requests badly, set it back to `high`.

### `max_turns`
The most steps an agent can take on one task. A step is one action: reading a file, running a command, searching, writing a file. When it hits the limit, the agent stops and hands back what it has; the manager continues it or asks you. **This is the hard limit.**
- Values: a whole number, or `-` for no limit.
- Raise it when an agent keeps stopping before finishing real work. Lower it when an agent wanders on small tasks.
- Example: `| code-reviewer | claude-sonnet-5-5 | medium | 25 | 150,000 |`

### `max_tokens_per_run`
A token cap for one launch of the agent. Claude Code can't stop an agent part-way on tokens, so the kit enforces it between launches: each run's usage is measured automatically from its transcript (`just budget-report`), and if it went over the cap the manager tells you and gives that agent smaller work next time (or asks you to raise the cap). Budget forecasts also use it as the ceiling per launch. Defaults sit near real usage: developers 400,000, tech lead and architect 200,000, testers and cicd 300,000, reviewers 150,000.
- Values: a whole number of at least 1,000, or `-` for no cap.
- Example - keep reviews cheap: `| code-reviewer | ... | 100,000 |`
- Pair it with `max_turns`: turns stop a run, the token cap shapes the next one.

## Agents - advanced (one row per agent)

```
| agent         | context_window | cache_ttl | memory  | permission_mode |
|---------------|----------------|-----------|---------|-----------------|
| code-reviewer | -              | -         | project | -               |
```

### `context_window`
When the agent's conversation reaches this many tokens, Claude Code summarises it to make room. Smaller = each step re-sends less, so long tasks cost less, at the price of losing detail sooner.
- Values: 100,000 to 1,000,000, or `-`. Needs Claude Code 2.1.296 or newer.
- Example - developers on long tasks: `| dev-data | 200,000 | ... |`

### `cache_ttl`
How long the agent's prompt cache stays warm. With `1h`, launching the same agent again within an hour re-uses the cache and costs less (cache writes cost a little more). Ignored while you're on usage credits.
- Values: `5m`, `1h`, or `-` (5 minutes).
- Example - reviewers launched many times a session: `| code-reviewer | - | 1h | ... |`

### `memory`
Where the agent keeps notes it learns between runs.
- Values: `project` (in the repo, shared with your team), `local` (in the repo, not committed), `user` (in your home folder, all projects), `none`.
- Example - stop the tester remembering anything: `| tester | - | - | none | - |`

### `permission_mode`
How tool approvals work for this agent.
- Values: `default` / `manual` (asks as usual), `acceptEdits` (file edits allowed without asking), `auto`, `dontAsk` (anything not pre-approved is refused instead of asked), `plan` (read and plan only, no changes).
- `bypassPermissions` is refused by the kit. The safety hooks (secrets, branch guards) run in every mode.
- Example - an architect that can only read and plan: `| architect | - | - | project | plan |`

## Session (applies to every agent; written to `.claude/settings.json`)

```
| setting           | value  |
|-------------------|--------|
| max_output_tokens | 16,000 |
```

| Setting | Values | What it does | Example use |
|---|---|---|---|
| `max_output_tokens` | 1,000-128,000 | Longest single reply any agent writes. Higher leaves less room before summarising | Lower to 16,000 to stop very long replies |
| `max_thinking_tokens` | 1,024-128,000 | Thinking budget per reply on models with a fixed budget | Lower to save tokens on routine work |
| `subagent_model` | model name | Model for any agent whose `model` is `-` | `haiku` as a cheap default |
| `max_parallel_agents` | 1-200 | How many agents run at once (default 20) | `6` to keep costs and laptop load down |
| `compact_at_percent` | 1-100 | Summarise earlier, at this % of the context window (can only lower) | `70` for leaner long conversations |
| `context_window` | 100,000-1,000,000 | One window for every agent; overrides the per-agent column | `200,000` for everyone |
| `file_read_max_tokens` | 1,000-200,000 | Largest chunk one file read returns | Lower if agents read huge files |
| `command_output_max_chars` | 1,000-150,000 | Largest command output an agent sees (default 30,000) | Lower to `15,000` for noisy builds |

If one of these seems to have no effect, it may need to be set in your shell before starting Claude Code instead (for example `export CLAUDE_CODE_MAX_OUTPUT_TOKENS=16000`); `just agents-settings` shows each variable name.

---

## Other sections (read live, no sync or restart needed)

### Context
| Setting | Default | Values | What it does |
|---|---|---|---|
| `preload_all_stack_skills` | `no` | `yes`, `no` | `no`: tech lead and reviewers load developer skills only when needed (cheaper). `yes`: they preload all of them. This one does need `just agents-sync` + restart |

### Parallelism
| Setting | Default | What it does |
|---|---|---|
| `work_mode` | `worktrees` | `worktrees`: developers run in parallel, each in its own git worktree. `in-place`: one developer at a time in your checkout, for repos where worktrees fail. `just preflight` recommends one; switch with `just work-mode in-place` |
| `max_review_rounds` | 3 | Fix-and-recheck rounds before the manager asks you |
| `max_slice_changed_lines` | 400 | Largest piece of work one developer gets |
| `small_max_files` | 3 | Work touching at most this many files in one area is size S: no plan, one developer, gate only |
| `checkpoint_commits` | `yes` | Developers commit on their feature branch each time the gate passes, so "undo that step" is a `git revert`, not another agent run |
| `reuse_roles` | `tech-lead, architect` (+ the kit's consulting agents) | agent names, comma-separated | Agents the manager continues instead of relaunching; a hook blocks the first fresh launch within `reuse_window_minutes` (90) and names the warm agent |
| `reuse_window_minutes` | 90 | How long after a run an agent still counts as warm for the reuse hook |
| `reuse_max_context` | 120,000 | Past this size a warm agent is restarted with a short handoff instead of continued (`just warm`) |
| `reuse_max_steps` | 60 | Past this many steps a developer's next slice goes to a fresh developer with the slice brief (`just warm`, `just next`): long runs cost more per step |
| `min_slice_lines` | 150 | `just plan` flags slices estimated below this (each PR pays its own reviews); `0` turns the flag off |
| `efficiency_review` | `yes` | After a feature's last merge the manager runs `just efficiency` and adds a few observations (no agent) |
| `preview_first` | `yes` | Look-and-feel work gets a cheap preview you pick from before anything is built (frontend kits: clickable demo pages) |

### Review policy
| Setting | Default | Values | What it does |
|---|---|---|---|
| `security_review` | `sensitive-or-large` | `always`, `sensitive-or-large` | When a security review runs (sensitive = auth, config, SQL/migrations, dependencies, CI, Dockerfile) |
| `tester_review` | `feature` | `feature`, `always`, `behaviour-change`, `large` | When a tester runs. `feature`: full test pass once per feature, plus targeted runs on PRs that touch risk areas |
| `require_feature_t2` | `yes` | `yes`, `no` | Promotion to the release branch waits for each feature's test pass |
| `large_pr_lines` | 150 | number | Size at which a PR counts as large |
| `tester_min_source_lines` | 20 | number | Smallest code change that gets a tester |
| `review_pack_max_diff_lines` | 1,200 | number | How much diff reviewers see in one go |
| `require_feature_record` | `yes` | `yes`, `no` | Feature PRs need `docs/features/F<n>-*.md` before they merge |
| `require_bug_record` | `yes` | `yes`, `no` | `fix/B<n>` PRs need `docs/bugs/B<n>-*.md` with a regression test |
| `envelope_reviews` | `yes` | `yes`, `no` | Small PRs (`envelope_max_lines`, default 400 counted lines, tests at half) are reviewed by one tool-free Claude call covering every required review, instead of reviewer agents: ~5-15k tokens instead of ~30k+ per reviewer. Big or always-full files (migrations, CI, auth, secrets, lock files, agent files) still get reviewer agents |
| `envelope_max_lines` | 400 | number | Largest PR (counted lines, tests at half) the one-call review takes |
| `envelope_command` | `claude` | path or name | The `claude` program for one-call reviews (found automatically inside a session) |
| `require_merge_approval` | `yes` | `yes`, `no` | Feature PRs merge only once approved: by `just triage` when nothing needs judgment, else by the tech lead (`just approve`) |
| `review_rounds` | `full` | `full`, `fixes-only` | `fixes-only`: from round 2 a reviewer gets its earlier findings plus only the changes since its review (`just review-pack`, envelope); it reads all of that fix diff and elsewhere reports only blocking problems. Changes no finding asked for are listed and reviewed fully |
| `defer_minor_after_round` | `off` | `off`, `1`, `2` | `1`: a minor/nit first raised after round 1, outside the fix, is deferred to the next slice by `just triage` instead of opening another round. Never applies to security or safety findings, or to problems in the fix itself |
| `nit_policy` | `defer` | `defer`, `fix` | `defer`: a nit from the code or test reviewer is deferred to the next slice (carried in the feature record's Deferred list) by `just triage` in every round, so it costs no fix round or re-review. `fix`: nits are fixed now if cheap. Security and safety findings and blocking severities are never auto-deferred |
| `unreviewed_fix_max_lines` | 20 | number | How many lines of a small fix may merge without a re-review (carry-forward). Only for fix-now minor/nit findings from the code/test reviewers, at the lines they cite. Never on `.claude/**`, policy files (team.md, owner-rules, templates), risk-guardian / quant / security-sensitive paths, or a file with a security or safety finding in the PR. `0` = off: every fix gets a re-review |
| `security_block_severities` | `critical, high, medium` | severities, comma-separated | Security findings at these severities block a merge while open |
| `require_reviewed_commit` | `yes` | `yes`, `no` | Every review file and approval names the commit it covered; merge-check refuses when the branch has its own changes after it (re-review), or its history was rewritten (full re-review) |
| `light_review_docs` | `yes` | `yes`, `no` | Docs-only PRs (never agent, CI or settings files) need no reviewer launch; the tech lead reads the diff |

Check what a PR will get: `just review-plan <pr>`.

### Budget
| Setting | Default | What it does |
|---|---|---|
| `per_task_tokens` | 3,000,000 | Budget for one task across all agents (`unlimited` to remove) |
| `per_day_tokens` | 15,000,000 | Budget for one day |
| `warn_percent` | 80 | When the manager warns you a budget is nearly used |
| `usd_per_million_tokens` | `-` | Your blended price, to show dollar estimates |
| `manager_clear_at` | 150,000 | Manager conversation size at which you are told to run `just milestone` and `/clear` |
| `budget_measure` | `tokens` | `tokens`: caps count each run's final context (well below the real cost). `cost-eq`: caps count what runs really cost, the manager included - raise `per_day_tokens` about 3x when you switch |

See usage any time: `just budget` (today) or `just budget <task>`; `just budget-report <task>` breaks it down per agent.
Every run also shows its **cold start** (what the agent loaded before doing any work) separately from its work; `just budget-starts` lists the typical start per agent type. A large start is the reason to continue agents rather than relaunch them, and to keep preloaded skills small.
Usage is **measured, not estimated**: a hook reads every agent's transcript when it finishes and books it against the
current task (`just budget-task F12`), and the manager's own session is measured the same way.

### Branches
| Setting | Default | What it does |
|---|---|---|
| `integration_branch` | `develop` | Branch feature PRs merge into |
| `release_branch` | `main` | Branch that only moves with your approval (`just promote`) |
| `require_ci_checks` | `yes` | Whether GitHub CI must pass before a merge |
| `delete_merged_branches` | `no` | `yes` deletes a work branch after `just ship` merges it; the integration and release branches never |
| `records_extra_globs` | `-` | More paths `just records-pr` may commit without review (comma-separated globs, e.g. `docs/strategy/**`): notes only, never code or settings |

### Approvals
Says what the manager asks you before. Rows marked `always` can't be turned off.

---

## Recipes

**Cheapest reasonable setup**
```
| code-reviewer     | haiku | low    | 15 | 100,000 |
| security-reviewer | haiku | medium | 15 | 100,000 |
| build-runner      | haiku | low    | 10 | 50,000  |
```
Session: `max_parallel_agents | 6`, `compact_at_percent | 70`.

**Stronger planning, same cost elsewhere**
```
| tech-lead | opus | high | 40 | 600,000 |
| architect | opus | high | 30 | 500,000 |
```

**Developers keep stopping before they finish**
Raise `max_turns` for that developer (e.g. 80 -> 120) and its `max_tokens_per_run` to match (e.g. 400,000 -> 600,000).

**Security review on every PR**
Review policy: `security_review | always`.

**Reviews cost too much**
Lower `max_tokens_per_run` for the reviewers, keep `preload_all_stack_skills | no`, set `cache_ttl | 1h` for reviewers, and check `just review-plan <pr>` to see which reviews ran.

---

## Troubleshooting

| Symptom | Fix |
|---|---|
| Change had no effect | Did you run `just agents-sync` and restart Claude Code? |
| `ERROR ... must be one of ...` | A typo or unsupported value; the message lists what's allowed |
| `ERROR unknown column` | Column name misspelt; `just agents-settings` lists the real names |
| `agents-check` shows DIFF | Someone edited an agent file directly; run `just agents-sync` to bring it back in line |
| Manager says OVER AGENT CAP | An agent used more than its `max_tokens_per_run`; raise the cap or let the manager split the work |
| Session setting ignored | Set the variable in your shell before starting Claude Code (see Session above) |
