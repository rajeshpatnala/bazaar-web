---
name: team-handbook
description: Reference for every agent on this team - who does what, where every file lives, the full command list and the hard rules explained in detail. Not preloaded; look up the part you need with `just docs "<topic>"`.
---

# Team handbook

The short version of everything here is in CLAUDE.md. Look up one section with `just docs "<topic>"`.

## Team

The main session runs as **manager** (`.claude/settings.json` -> `"agent": "manager"`). Only the manager launches agents.

| Agent | Role | Writes |
|---|---|---|
| manager | User contact, coordination, board, approvals, reports | `.agents/` only |
| tech-lead | Plans and slices, answers technical questions, triages findings, **only one who merges** | `.agents/`, `docs/` |
| architect | Designs, ADRs, NFRs | `.agents/`, `docs/` |
| dev-data / dev-quant / dev-backtest / dev-execution / dev-api / dev-ui / dev-core | Implement one slice each **with unit tests**; any number in parallel, own worktree + branch, open PRs | source, tests |
| cicd | CI workflows, Docker, release automation (CHANGELOG + tags) | `.github/`, build/release files |
| tester (many) | Test plans, acceptance/integration tests, sign-off | tests on assigned branch, `.agents/reviews/` |
| code-reviewer (many) | Reviews each branch as soon as it's handed off | `.agents/reviews/` only |
| security-reviewer (many) | Security review, verifies fixes | `.agents/reviews/` only |
| build-runner | Runs gates/commands, returns compact results | nothing |
| prototyper | Up to 3 throwaway previews before look-and-feel work is built | `previews/` only |

## Where things are

- `AGENT-GUIDE.md` - for humans: who does what and how to ask. `AGENT-SETTINGS.md` - every setting.
- `.agents/project.md` - project profile (purpose, data stores, integrations, data formats, deploy). Read first.
- `.agents/owner-rules.md` - the owner's standing rules; a hook gives them to every agent at start. They apply everywhere, not only where first said.
- `.agents/team.md` - models, effort, review rounds, **token budget**, approvals. Agents added later appear here and in `.claude/agents/`.
- `.agents/board.md` - task state; how work resumes. (local)
- `.agents/codemap.md` - modules, routes, migrations, config key names and **in-flight branches**; a big module list is split into one file per area under `.agents/codemap/`. Look things up with `just codemap find <word>` **instead of searching the repo**. Auto-refreshed at session start and after merges/pulls/pushes.
- `.agents/decisions.md` - decision index; ADRs in `docs/adr/`.
- `docs/features/F<n>-*.md` - **committed feature records** (what each feature does, where it lives, decisions, limits, deferred items). `docs/bugs/B<n>-*.md` - committed bug records (root cause, fix, regression test). `just features`, `just feature F7`, `just bugs`.
- `.agents/plans/`, `.agents/reviews/`, `.agents/reports/`, `.agents/bugs/` - per-feature working files and bug drafts. (local)
- `.agents/templates/` - plan, test plan, handoff, findings, report formats.
- Skills in `.claude/skills/` hold the standards; agents preload the ones they need.

Agents in worktrees get **absolute paths** to `.agents/` files in their prompt; local working files are not in their checkout.

## Commands (full list; `just --list` shows them too)

```
just preflight     # can the team work here (branches, gh, worktrees)? just work-mode in-place if worktrees fail
just doctor        # kit health: tools, hooks, settings, agents (doctor-fix repairs safe issues)
just gate          # cargo fmt + clippy -D warnings + nextest; web typecheck + lint + vitest, compact
just dev-gate      # gate + 'code changed => unit tests added'. Developers must pass it before handing off.
just test          # tests only
just check         # gate + doc tests + cargo deny + pnpm audit
just db-check      # connection status of configured data stores (never prints secrets)
just codemap       # regenerate .agents/codemap.md (normally automatic)
just budget [task] # token usage vs limits; budget-add / budget-check for the manager
just agents-sync   # apply .agents/team.md to agent files and settings (just agents-settings lists every setting)
just new-agent ... # used by the manager when hiring; users just say "hire a new agent"
just changelog     # offline changelog (release-please does it on GitHub)
just base          # integration branch; just new-branch <name> creates a work branch from it
just review-pack N # one-shot review context; just review-plan N = which reviews are needed
just merge-check N # can PR N merge? (target, title, CI, reviewer verdicts) - the only source for "green"
just ship N        # manager, once approved: wait for CI, merge-check, merge, update branch + codemap (just pr-wait N = only wait)
just promote N     # release branch, asks the user
just status        # one-shot status: checkout, budget, board, open PRs + CI, bugs
just envelope N    # small PR: all required reviews in one tool-free call (exit 3 = launch reviewers)
just triage F N    # default triage decisions for a PR's findings; lists only what needs the tech lead
just agents        # the team: agents, model, effort, role
just features      # feature records; just feature F7 = one feature with PRs, files, release, bugs
just bugs          # open bugs; just bug-new "<title>" --feature F7 --severity high drafts one
```

## Hard rules in detail

1. **Secrets:** never read, grep, print, copy or edit `.env*` (except `.env.example`), key files, `secrets/`, or environment dumps. Use `just db-check` for connectivity. Hooks enforce this.
2. **Mocks only:** tests use fakes/mocks, mock servers, or ephemeral test databases started by the test. No agent touches real or shared data stores.
3. **Git:** work branches come from the integration branch (`just base`, default `develop`) and PRs target it, with Conventional Commit titles. Reviews happen on the PR; a PR merges once it is approved (by `just triage` when nothing needs judgment, else by the tech lead with `just approve <pr>`) and the manager runs `just ship <pr>` (waits for CI in the script, then merge-check, merge, update) - never poll CI turn by turn; no agent is launched just to merge. The release branch (`main`) moves only through `just promote`, which the user approves. Nobody pushes to protected branches. Report PR status only from `just merge-check`.
4. **Reviewers don't change source.** Findings go to the exact paths `just review-pack` prints (`.agents/reviews/<feature-id>/<pr-N|branch>/`); the tech lead triages; developers fix.
5. **Stay in scope:** do your slice/task only; report anything else.
6. **Ask, don't guess** on API contracts, data semantics, security behaviour: return a question in your handoff.
7. **Be brief:** replies use the handoff template (<= 15 lines, with *Where to look*); details go in files or PRs. Don't paste big files or logs.
8. **Read narrowly, call rarely:** every tool call re-sends your whole context. Reviewers start with one `just review-pack <pr>`; use diffs over whole files, `just codemap find <word>` over repo searches, `just plan <id> --slice <n>` and `just section <file> "<heading>"` over whole notes, Read with offset/limit, and load extra skills only when needed. Keep agent memory files under 40 lines.
9. **Use only what the project declares:** data stores, services and integrations listed in `project.md`.
10. **Unit tests** are part of every code change, written by the developer in the same PR. **Verify by tier:** each slice T0 (gate) + T1 (one quick look at what changed); the full matrix (T2) runs once per feature, by the tester.
11. **Never wipe the integration or release branch:** no deleting, renaming, force-moving, force-pushing or resetting `develop`/`main` (or deleting the repo). Hooks block it; if it ever seems necessary, stop and ask the user.
12. **Records travel with the code:** a feature PR adds/updates `docs/features/F<n>-*.md`; a bug fix (`fix/B<n>-...`) starts with a failing regression test and commits `docs/bugs/B<n>-*.md`. Answer questions about a feature from `just feature <id>` before reading code.
13. **Never work around the kit:** if worktrees, branches or a kit agent fail, stop and report (`just preflight`); don't swap in a general-purpose agent or skip reviews.


## Default stack

Rust 2024 Cargo workspace (tokio, rust_decimal for all money, chrono + chrono-tz Asia/Kolkata, sqlx/SQLite, Parquet + DuckDB, polars, tracing, keyring-core with the macOS Keychain) with an axum 0.8 server (REST /api/v1 + WebSocket on 127.0.0.1 only, launch-token session, utoipa OpenAPI) serving a React 19 + TypeScript + Vite UI charting with TradingView lightweight-charts 5; Zerodha Kite Connect + official NSE/BSE bhavcopy data. Layout and layering: `bz-repo-structure`, `bz-architecture`.
