# Task runner used by humans and agents. Outputs are kept compact on purpose (token budget).
# Stack commands come from .agents/stack.json. just never loads .env; the application loads it itself.

default: gate

# check tools, hooks, settings and agents; prints install commands for anything missing
doctor:
    @python3 .claude/kit/doctor.py

# fix what is safe to fix automatically (permissions, .env from example, agent sync)
doctor-fix:
    @python3 .claude/kit/doctor.py --fix

# cargo fmt + clippy -D warnings + nextest; web typecheck + lint + vitest with compact output (required before any handoff)
gate:
    @python3 .claude/kit/kit.py gate

# developer gate: gate + "production code changed => unit tests added"
dev-gate: gate
    @python3 .claude/kit/kit.py test-check

# tests only (compact)
test:
    @python3 .claude/kit/kit.py test

# full local CI: gate + doc tests + cargo deny + pnpm audit
check:
    @python3 .claude/kit/kit.py check

# connection status for configured data stores/services (never prints secrets)
db-check:
    @python3 .claude/kit/kit.py db-check

# regenerate .agents/codemap.md (also runs automatically at session start and after merges/pulls)
codemap:
    @python3 .claude/kit/kit.py codemap

# can the team work here? branches, remote, gh, worktrees -> recommended work mode and how to fix problems
preflight:
    @python3 .claude/kit/kit.py preflight

# show or switch how developers work: worktrees (parallel) or in-place (one at a time in this checkout)
work-mode *mode:
    @python3 .claude/kit/kit.py work-mode {{mode}}

# integration branch that feature PRs target (team.md -> Branches)
base:
    @python3 .claude/kit/kit.py base

# create a work branch from the latest integration branch, e.g. just new-branch feat/F12-orders
new-branch name:
    @python3 .claude/kit/kit.py new-branch {{name}}

# may this PR merge? target branch, Conventional title, CI checks, reviewer verdicts (the only source for "it's green")
merge-check pr *args:
    @python3 .claude/kit/kit.py merge-check {{pr}} {{args}}

# review without a PR: just review-pack --diff develop..HEAD (also review-plan / merge-check --branch <b>)
# which reviews a PR needs (code always; security/tests by size and sensitivity - team.md -> Review policy)
review-plan pr *args:
    @python3 .claude/kit/kit.py review-plan {{pr}} {{args}}

# everything a reviewer needs in one output: size, required reviews, criteria, red flags, codemap lines, diff
review-pack pr *args:
    @python3 .claude/kit/kit.py review-pack {{pr}} {{args}}

# small PRs: all required reviews in one tool-free Claude call instead of reviewer agents (exit 3 = launch them)
envelope pr *args:
    @python3 .claude/kit/kit.py envelope {{pr}} {{args}}

# what an agent reads before it starts working, and how big each part is (no agent = summary of all)
start-report *agent:
    @python3 .claude/kit/kit.py start-report {{agent}}

# apply the default triage decisions to a PR's findings; lists only what needs the tech lead
triage feature *args:
    @python3 .claude/kit/kit.py triage {{feature}} {{args}}

# one-shot status for the manager: checkout, work mode, budget, board, open PRs with CI, bugs
status:
    @python3 .claude/kit/kit.py status

# wait for a PR's CI inside the script (free); exit 0 passed, 1 failed, 2 still running
pr-wait pr *args:
    @python3 .claude/kit/kit.py pr-wait {{pr}} {{args}}

# tech lead's merge in one command: wait for CI, merge-check, merge, update the integration branch, refresh codemap
ship pr *args:
    @python3 .claude/kit/kit.py ship {{pr}} {{args}}

# tech lead: merge-check, then squash-merge a feature PR into the integration branch
merge pr *args:
    @python3 .claude/kit/kit.py merge {{pr}} {{args}}

# promotion/release PR into the release branch - Claude Code asks the user every time
promote pr:
    @python3 .claude/kit/kit.py promote {{pr}}

# create the integration branch locally and on the remote if missing (asks the user)
setup-branches:
    @python3 .claude/kit/kit.py setup-branches

# show (or --apply) GitHub branch protection for the integration and release branches (asks the user)
protect-branches *args:
    @python3 .claude/kit/kit.py protect-branches {{args}}

# the team: every agent with model, effort, turns and role, and which agent the main session runs as
agents:
    @python3 .claude/kit/kit.py agents

# feature records (docs/features): list all, or one with its PRs, files, releases and bugs
features:
    @python3 .claude/kit/kit.py features

feature id:
    @python3 .claude/kit/kit.py feature {{id}}

# bugs: draft one (just bug-new "Cart total wrong" --feature F7 --severity high) or list open ones (--all for every one)
bug-new title *args:
    @python3 .claude/kit/kit.py bug-new "{{title}}" {{args}}

bugs *args:
    @python3 .claude/kit/kit.py bugs {{args}}

# move finished rows from the board to .agents/board-archive/<month>.md
board-archive:
    @python3 .claude/kit/kit.py board-archive

# token budget (runs are measured automatically): manual row with --estimate if unmeasured, status, or check planned launches (e.g. just budget-check F12 dev-data:4 code-reviewer:4)
budget-add task agent tokens *note:
    @python3 .claude/kit/budget.py add {{task}} {{agent}} {{tokens}} {{note}}

budget task="":
    @python3 .claude/kit/budget.py status {{task}}

# set the task that measured agent runs are booked against (the manager does this when work starts)
budget-task task:
    @python3 .claude/kit/budget.py task {{task}}

# typical cold-start size per agent type (what every fresh launch pays before working)
budget-starts:
    @python3 .claude/kit/budget.py starts

# per-agent breakdown of a task's measured usage (for reports and retrospectives)
budget-report task="":
    @python3 .claude/kit/budget.py report {{task}}

budget-check task +plan:
    @python3 .claude/kit/budget.py check {{task}} {{plan}}

# apply .agents/team.md (model / effort / max turns) to .claude/agents/*.md
agents-sync:
    @python3 .claude/kit/agents_sync.py

# list every setting team.md supports, with allowed values
agents-settings:
    @python3 .claude/kit/agents_sync.py --list

# show whether agent files match .agents/team.md
agents-check:
    @python3 .claude/kit/agents_sync.py --check

# scaffold a new agent without an interview (the manager normally hires conversationally - see agent-hiring skill)
new-agent name kind description skills="":
    @python3 .claude/kit/new_agent.py {{name}} --kind {{kind}} --description "{{description}}" --skills "{{skills}}"

# offline changelog (git-cliff); release-please does this automatically on GitHub
changelog:
    @git cliff --output CHANGELOG.md && echo "CHANGELOG.md updated"

# install UI dependencies (first run)
setup:
    pnpm -C web install --frozen-lockfile

# format Rust and UI code
fmt:
    cargo fmt --all
    pnpm -C web format

# HUMAN-ONLY: build the UI and run the server on 127.0.0.1:8787 against your data/ and Keychain; opens the browser with a one-time login link
run:
    pnpm -C web build
    cargo run --release -p bazaar-server

# HUMAN-ONLY: debug server + UI with hot reload on :5173 (stop `just run` first); open the printed http://localhost:5173/auth?token=... link once Vite is up
ui-dev:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build -q -p bazaar-server
    target/debug/bazaar-server --no-open &
    server=$!
    trap 'kill $server 2>/dev/null' EXIT
    pnpm -C web dev

# UI only in the browser with mocked API (MSW) - previews, screenshots, Playwright
app-dev:
    pnpm -C web dev:mock

# build the UI and a release server binary
build:
    pnpm -C web build
    cargo build --release -p bazaar-server

# regenerate UI types from the server's OpenAPI document
openapi:
    cargo run -q -p bazaar-server -- openapi > web/openapi.json
    pnpm -C web gen:api

# HUMAN-ONLY: store a secret in the macOS Keychain (prompts without echo), e.g. just set-secret kite_api_secret
set-secret name:
    cargo run -q -p bazaar-cli -- set-secret {{name}}

# HUMAN-ONLY: import an official bhavcopy for a trading date (YYYY-MM-DD) into data/
import-bhavcopy date:
    cargo run -q -p bazaar-cli -- import-bhavcopy --date {{date}}

# HUMAN-ONLY: refresh the instrument master (lot/tick sizes, expiries) from the live broker API
refresh-instruments:
    cargo run -q -p bazaar-cli -- refresh-instruments

# HUMAN-ONLY: headless backtest on your data/, e.g. just backtest ema-cross 2022-01-01 2025-12-31
backtest strategy from to:
    cargo run -q --release -p bazaar-cli -- backtest --strategy {{strategy}} --from {{from}} --to {{to}}

# look-ahead suite (tests named lookahead_*); fails if none ran
lookahead:
    cargo nextest run --workspace -E 'test(/lookahead_/)' --no-tests=fail

# agent-safe CLI run on fixtures and a temp data dir, e.g. just cli-fixture backtest --strategy ema-cross --from 2024-01-01 --to 2024-03-31
cli-fixture *args:
    #!/usr/bin/env bash
    set -euo pipefail
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT
    cargo run -q -p bazaar-cli -- --config-dir fixtures/config --data-dir "$tmp" {{args}}

# golden backtest regression against committed baselines
golden:
    cargo test -q -p bazaar-backtest --test golden

# benchmarks (indicators, scanner, backtest loop)
bench:
    cargo bench --workspace

# coverage report (HTML)
cov:
    cargo llvm-cov nextest --workspace --html
