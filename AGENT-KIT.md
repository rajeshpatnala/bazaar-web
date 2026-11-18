# Bazaar Rust web trading app AI Agent Kit for Claude Code

**New here? Read `AGENT-GUIDE.md`**: who does what, and example requests. In short, you talk to the manager in plain words and it picks the agents.

A drop-in team of Claude Code sub-agents that plans, designs, builds, tests, reviews, merges and releases Bazaar Rust web trading app projects. You talk to the **manager**; it coordinates everyone else within a token budget.

## Install

1. Unzip the kit anywhere and run one command, pointing at your project (new or existing):

   ```bash
   ./install.sh /path/to/your/repo
   ```

   It copies the kit in, merges with anything you already have (your `CLAUDE.md`, `.gitignore`, `justfile` and `.claude/settings.json` keep their content; files like an existing `Dockerfile` or CI workflow are kept and the kit's version is saved beside them as `*.kit`), sets up a git repo if needed, creates `.env` from `.env.example`, fixes permissions, and checks your tools. Anything missing is listed with the exact command to install it.
2. Fill in `.env` yourself. Agents never read it.
3. Commit the kit files on a branch. Claude Code sessions that run in a git worktree or in the cloud (common in the desktop and web apps) only see committed files.
4. Start a **new** Claude Code session with the repo as its folder and trust the workspace. Check it: the terminal shows `@manager` in the startup header, and asking *"who are you?"* gets "the manager". `just agents` lists the whole team.
5. Say *"Manager, fill in the project profile (.agents/project.md) with me"*. The project profile is `.agents/project.md`: what the project is, its data stores, sources, integrations, environments and deploy target, plus which branches to use. The manager interviews you and fills it in; every agent reads it.

**If the manager doesn't answer** (you get a general Claude reply, or questions like "what do you mean by project profile?"), the session isn't running the kit:
- It was opened in another folder, or before the install: start a new session with the repo itself as its folder.
- It runs in a worktree or cloud copy made from the last commit: commit the kit files (step 3), then start a new session.
- The workspace isn't trusted, so project settings are ignored: accept the trust prompt.
- Claude Code is too old for the `agent` setting: update it, or start it with `claude --agent manager`.
`just doctor` in the repo flags the first two.

The tools you need are `git`, `jq`, `python3`, `just` and `just`, `cargo`, `node`, `pnpm`; `cargo-nextest`, `cargo-deny`, `cargo-llvm-cov`, `git-cliff` and an authenticated `gh` are recommended.

**Upgrading:** run the new kit's installer against the same repo. See [Upgrading](#upgrading) below.

## How it works

```
You ──▶ manager (main session: plans, launches, tracks board + budget, reports)
          ├──▶ tech-lead (plan, slices, answers questions, triage, merges, releases) ──▶ architect when needed
          │
          ├──▶ developers, as many as there are slices, all in parallel, each in its own worktree:
          │      dev-data   dev-quant   dev-backtest   dev-execution   dev-api   dev-ui   dev-core   cicd   (+ any agent you add)
          │
          │    as EACH developer hands off (others keep building):
          ├──▶ build-runner gate ─▶ code-reviewer + security-reviewer + tester(s) in parallel on that branch
          │
          └── findings ─▶ tech-lead triage ─▶ same developer fixes ─▶ re-check
                        ─▶ tech-lead merges (your approval) ─▶ code map refreshes
                        ─▶ release PR (CHANGELOG + version) ─▶ tag vX.Y.Z + image (your approval)
```

- **Sub-agents only.** Only the manager launches agents; the tech lead decides what to launch and answers technical questions through the manager.
- **Sized before it's planned.** Every request is S (one developer, no plan), M (one plan file, one developer continued step by step) or L (slices, parallel developers). The manager tells you the size and estimate before spending anything.
- **Previews before taste work.** Requests judged by look or feel get up to three cheap previews you pick from before anything is built (`preview_first`).
- **No developer limit for L work.** The tech lead slices it as finely as is safe; the manager launches one developer per slice in parallel. Developers are reused for the next step in the same area instead of starting cold.
- **Owner rules.** Corrections you give once ("cards fit the screen") go into `.agents/owner-rules.md`; a hook hands them to every agent, and the whole product is checked for the same problem.
- **Small cold starts.** Every fresh agent first reads its instructions, handbooks, CLAUDE.md levels and notebook; reviewers skip CLAUDE.md and the notebook, and `just start-report <agent>` shows what each one reads and how big it is.
- **Envelope reviews.** A small PR is reviewed by one tool-free Claude call that applies every required checklist at once (`just envelope <pr>`), instead of launching up to four reviewer agents. Big or always-full changes still get the reviewer agents.
- **Tiered verification.** Each slice: T0 (gate) + T1 (one quick look at what changed). Once per feature: T2, the full matrix, by one tester.
- **Preflight.** `just preflight` checks branches, `gh` and worktrees before work starts. If worktrees fail, `in-place` mode runs one developer at a time in your checkout, with the same reviews and gates (`--diff` / `--branch`). The manager never swaps a kit agent for a general-purpose one.
- **Pipelined reviews.** Code review, security review and testing start on each branch the moment its developer finishes, overlapping with developers still building. Big slices get an early checkpoint review on a draft PR.
- **Unit tests are mandatory.** Developers write them in the same PR; a hook blocks a developer from finishing if production code changed without new tests (`No-Tests: <reason>` commit trailer for genuine exceptions).
- **Developer specialties** (all also have `bz-core`, `bz-testing`, `git-workflow`):

| Agent | Unique skill | Typical slices |
|---|---|---|
| dev-data | `bz-market-data` | broker adapters, WebSocket client, bhavcopy, instrument master, corporate actions, storage |
| dev-quant | `bz-quant` | indicators, options math, regime, strategies, scanner and scoring |
| dev-backtest | `bz-backtest-risk` | backtester, cost model, slippage, metrics, walk-forward, sizing, DecisionGate rules |
| dev-execution | `bz-execution` | order pipeline, approvals, paper account and fills, kill switch, rate limits, audit log, broker crate |
| dev-api | `bz-web-api` | endpoints, WebSocket, auth and sessions, OpenAPI, error mapping, static UI serving |
| dev-ui | `bz-web-ui` | screens, charts, Why panels, approval dialog, settings, API client |
| dev-core | `bz-repo-structure` | core types, IST calendar, config, cli, wiring, refactors, general |
| cicd | `ci-cd` | workflows, Docker, release automation, Dependabot |

- **Tech lead, code reviewer, security reviewer** preload all developer skills.

## Budget

- `.agents/team.md` → **Budget**: `per_task_tokens` (default 3M), `per_day_tokens` (default 15M), `warn_percent` (80), optional `usd_per_million_tokens` for dollar estimates.
- **Measured automatically.** A hook reads each agent's transcript when it finishes and books its real usage against the current task (`just budget-task F12`); another measures the manager's own session. Nothing is estimated unless marked `--estimate`. Before each batch the manager checks the projection (`just budget-check F12 dev-data:2 code-reviewer:2`) from those measurements.
- `just budget-report F12` breaks a task down per agent; large tasks end with a 5-line retrospective (tokens by phase, rework, what you corrected).
- WARN → it tells you and continues; OVER LIMIT → it stops and asks (raise limit, fewer agents, lower effort, or smaller scope). Every report ends with budget used.
- `just budget` shows today's usage any time. Two numbers per run: *tokens* (the agent's context at its last step, as Claude Code shows it; limits use this) and *processed* (everything sent and received over all its steps, cache reads included, which bill at ~10%). Claude Code's `/usage` stays the authority on cost.

## Branches, reviews and merging

- **Integration branch** (default `develop`, set in `team.md` -> Branches; the manager asks you on first run): every work branch starts here and every feature PR targets it.
- **Reviews happen on the PR, before merging.** Code review, security review and testing start on each PR as soon as its author hands off, including CI/infra PRs. Verdicts go to one folder per PR, `.agents/reviews/<feature>/<pr-N|branch>/`, so slices never overwrite each other.
- **`just merge-check <pr>`** is the gate and the only source for "it's green". It checks the target branch, the Conventional Commit title, every GitHub CI check, and the reviewers' verdicts (no open blockers).
- **The tech lead merges into the integration branch** with `just ship <pr>`: it waits for CI inside the script (no agent turns spent polling), refuses if the merge-check fails, merges, and updates the integration branch and code map. Docs-only PRs need no reviewer; the tech lead reads the diff. A hook blocks any raw `gh pr merge` that skips the check, and blocks all agents from pushing or committing to the integration or release branch.
- **The release branch (`main`) moves only when you approve.** Promotion and release PRs go through `just promote <pr>`, and Claude Code asks you for permission every time.
- **Branch protection** is the real safeguard on GitHub: `just protect-branches` shows the settings (PR required, CI checks required, no force-push) and applies them with `--apply` after you approve. Solo repos keep 0 required approvals, because GitHub doesn't let you approve your own PRs. Your approval is the Claude Code prompt instead.

## Releases, CHANGELOG and tags

1. You ask for a release; the tech lead opens a promotion PR from the integration branch to `main`; `just promote` merges it after you approve.
2. release-please opens a release PR on `main` with the next version and `CHANGELOG.md` (built from the Conventional PR titles).
3. You approve again; `just promote` merges it, then CI tags `vX.Y.Z`, creates the GitHub release and pushes the image.
4. The tech lead back-merges `main` into the integration branch.

No GitHub? `just changelog` (git-cliff) plus an annotated tag after your approval.

## Keeping track: the code map

`.agents/codemap.md` lists modules, files with their doc line and public items, HTTP routes, migrations, config key names, and an **In-flight** section with every unmerged branch (changed files, new routes). It regenerates automatically at session start and after every merge, pull, push, rebase or branch change, so new work is on the map before and after it merges. `just codemap` forces a refresh.

## Keeping track: features and bugs

Every feature gets an id (`F1`, `F2`, ...) and a record in `docs/features/`, committed with its code: what it does, where it lives, decisions, known limits, and review findings deferred for later. Every bug gets an id (`B1`, ...); its fix starts with a failing regression test, and the fix PR commits `docs/bugs/B<n>-*.md` with the root cause and that test. The merge check refuses a feature PR without its record and a bug fix without its test (`.agents/team.md` -> Review policy).

`just features`, `just feature F7` (record, PRs, files, release, bugs) and `just bugs` answer "what do we have?" without reading the code. Projects that had features before the kit: say *"backfill the feature records"* once.

## Hiring a new agent (just ask)

Say it like you'd tell a manager: *"I want to hire a new agent"* or *"we need someone for performance work"*.

1. **Interview.** The manager asks what the new team member will work on, then up to one round of quick multiple-choice questions: kind of work (build code / review / plan / run tools), what they may change, how complex the work is (sets effort), and which existing skills they need. Follow-up questions come only if something is still unclear: special knowledge, what "done" means, who checks their work, anything they must never do.
2. **Team check.** If an existing agent already covers most of the role, the manager suggests extending it with a new skill instead (cheaper).
3. **Job card.** You get a short card to approve or change: name, role, when they're called, kind, effort, what they may change, skills, deliverable, reviewers, estimated cost per launch.
4. **Onboarding.** The manager writes the role instructions and, for special knowledge, asks the tech lead to write a new skill. It then installs everything: the agent file with the right tools, isolation, safety hooks and team rules for its kind; the skill; the `team.md` row; and for developers, the unit-test gate.
5. **Start a new session.** From then on the manager and tech lead assign work to the new agent by its description.

The same conversation works for changes (*"give the perf engineer database skills"*) and retiring (*"let the docs reviewer go"*). Retired agents move to `.claude/agents-retired/` and can be brought back. Manager and tech lead can't be retired.

Under the hood it's `.claude/kit/new_agent.py` (install / `--update` / `--retire`); you can run it yourself, but you never have to.

## Safety built in (hooks, not just instructions)

| Guard | What it does |
|---|---|
| `guard-secrets.sh` (all agents) | Blocks reading/grepping/editing `.env*` (except `.env.example`), key files, `secrets/`, and environment dumps. |
| Permission deny rules | Second layer for the same secret paths, plus `printenv` and force push. |
| `write-scope.sh` | Manager, tech lead, architect and reviewers can only write to `.agents/` (and `docs/` for leads). |
| `dev-git-guard.sh` | Developers, testers and cicd can't merge, promote, or change branch setup. |
| `branch-guard.sh` (all agents) | Never deletes, renames, force-moves, force-pushes or resets the integration/release branch, and never deletes the repo. No pushes or commits on those branches; every `gh pr merge` must pass `merge-check`; PRs into the release branch only via `just promote` (user-approved). |
| `dev-gate.sh` | A developer can't finish while `just gate` fails or tests are missing (one retry; then it must report FAIL). |
| `after-git-sync.sh` + SessionStart | Keep the code map current. |
| `agent-context.sh` (SessionStart, SubagentStart) | Gives every agent the owner rules and the work mode at start. |
| `budget-meter.sh` (SubagentStop, Stop) | Measures real token usage of every agent run (cold start and work) and of the manager's session. |
| `agent-reuse.sh` (PreToolUse on agent launches) | Blocks the first fresh launch of a tech lead, architect or consulting agent that already ran this session and names the warm one to continue. |
| Gate record | Every full gate pass on a clean tree is recorded per commit; local merges (`merge --branch`) require one. |

Connectivity: `just db-check` loads `.env` inside the app and prints only `OK`/`FAILED <category>` per configured store.

## Token savers

Every tool call re-sends an agent's whole context, so cost is roughly *context size x number of calls*. The kit keeps both small:

- **Review pack:** reviewers start with one `just review-pack <pr>`, which prints size, required reviews, acceptance criteria, previous findings, red-flag hits, codemap lines and the diff. That's typically 2-3 calls per review instead of 6-10.
- **Reviews scale with risk:** code review always; security review only for large PRs or sensitive files (auth, config, SQL/migrations, dependency/lock files, CI, Dockerfile); tester only for behaviour changes. `just review-plan <pr>` shows which ones a PR needs, and `merge-check` enforces exactly those. Tune in `team.md` -> Review policy.
- **Lean preloads:** reviewers preload only their process skill and the core skill (~4k tokens per turn instead of ~17k); the tech lead loads developer skills on demand. Set `preload_all_stack_skills: yes` in `team.md` -> Context to preload everything.
- **Turn limits:** reviewers 15, tester 40, tech lead 40 (`team.md`).
- Short `CLAUDE.md`, path-scoped rules, store references loaded only when used, the code map instead of repo searches, build-runner absorbing build logs, fixed short formats, and fixes reusing the same developer.
- Reported token counts include cached context, which is billed at a fraction of the normal rate; `/usage` shows actual cost.

## Configuration

Every setting lives in `.agents/team.md`: per-agent model, effort, turn limit, token cap, context window, cache, memory and permission mode; session-wide token limits; review policy; budget; branches; approvals.

**See `AGENT-SETTINGS.md` for what each setting does, examples, ready-made recipes and troubleshooting.** `just agents-settings` lists them in the terminal.

Change a value, run `just agents-sync`, restart Claude Code. Or ask the manager in chat.

`.agents/project.md` is the project profile (data stores, integrations, formats, deploy); stack commands live in `.agents/stack.json`.

## Files

```
CLAUDE.md                     agent guide (every agent loads it)
AGENT-KIT.md                  this file
.claude/settings.json         manager as main agent, permissions, hooks
.claude/agents/               20 agents: manager, tech-lead, architect, dev-data, dev-quant, dev-backtest, dev-execution, dev-api, dev-ui, dev-core,
                              cicd, tester, code-reviewer, security-reviewer, build-runner, prototyper (+ pack extras)
.claude/skills/               21 skills
.claude/rules/                path-scoped rules
.claude/hooks/                secrets, write-scope, git guard, developer gate, map refresh
.agents/                      team.md, project.md, decisions.md, templates/ (committed)
                              board.md, budget.csv, codemap.md, plans/, reviews/, reports/ (local)
.claude/kit/                   kit.py (gate, tests, code map, db-check from stack.json), budget.py, agents_sync.py, new_agent.py, doctor.py
install.sh, install.py         one-command install/upgrade (run from the kit; not copied into your repo)
.github/                      ci.yml, release.yml, dependabot.yml, PR template
release-please-config.json, .release-please-manifest.json, version.txt, CHANGELOG.md, cliff.toml
Dockerfile, .dockerignore, deny.toml, justfile, .env.example, .gitignore
```

## Upgrading

Run the new kit's installer against your repo (once per repo that uses this kit):

```bash
./install.sh /path/to/your/repo        # or: python3 install.py /path/to/your/repo
```

| What | On upgrade |
|---|---|
| Kit agents, skills, hooks, `.claude/kit/` scripts, rules, `.agents/templates/`, `AGENT-KIT.md`, `AGENT-GUIDE.md`, `AGENT-SETTINGS.md` | Replaced. Any you changed are backed up to `.agents/backup-<date>/` first |
| `CLAUDE.md`, `.gitignore`, `justfile`, `.claude/settings.json` | Merged: your lines stay, new kit lines are added |
| `.agents/team.md` | Kept. New settings are added with their defaults; your values stay; model/effort/turns are re-applied to the agents |
| `.agents/stack.json` | Merged: new kit keys (review routing, never-break rules, developer paths) are added; values you changed stay |
| `project.md`, `decisions.md`, board, plans, reviews, bug drafts, agents you hired, your code | Never touched |
| Your own `Dockerfile`, CI workflows and similar | Kept; the kit's version is saved beside them as `*.kit` to compare |

**Sending the kit for feedback or an upgrade:** `just export-kit` zips only the kit (agents, skills, hooks, `.agents`
notes and records, CI, kit docs) and prints its size and contents - never your code, `data/`, build output, worktrees,
backups or `.env` files. `just export-kit --list` shows the file list without zipping.

After upgrading:
1. Restart Claude Code in the repo.
2. `just doctor` - checks tools, hooks and settings, and tells you about anything new (for example a feature-record backfill).
3. Commit the kit changes through a PR into the integration branch, like any other change.

**Where to make lasting changes.** Edits made directly to kit files in a repo are replaced on the next upgrade (they stay in the backup). Settings belong in `.agents/team.md`; extra agents are hired through the manager and survive upgrades. Changes to the kit's own agents or skills belong in the pack the kit was built from (Kit Factory: `~/.claude/kit-factory/packs/<id>/`), followed by a rebuild.

**Where new versions come from.** If this kit was built with Kit Factory, update the factory, then say *"rebuild my <id> kit"* in Claude Code; the new kit lands in `~/agent-kits/<id>/`. The factory's README describes the whole chain.

## Verify on your machine (first run)

1. `claude --version` is recent enough for `effort`, `skills`, `memory`, `isolation` and `hooks` in agent frontmatter. Unknown fields are silently ignored, so a missing feature fails quietly. `just agents` should list every agent with no BROKEN rows (Claude Code's `/agents` wizard was removed in v2.1.198).
2. Ask the manager to launch two developers on dummy slices: each should get its own worktree, and the code map's In-flight section should show both branches.
3. Ask any agent to `cat .env`: the guard should block it.
4. On the first PR to GitHub: CI runs, the PR title check works, and after merge release-please opens a release PR. Enable branch protection with the required checks yourself.
5. Built by the Kit Factory: stack commands in `.agents/stack.json` were validated for syntax, not run against your real project; run `just gate` once after the first code lands.
