# Task runner used by humans and agents. Outputs are kept compact on purpose (token budget).
# Stack commands come from .agents/stack.json. just never loads .env; the application loads it itself.

# recipe arguments reach the kit as separate, quoted arguments: free text with ; : ( ) is safe
set positional-arguments

default: gate

# check tools, hooks, settings and agents; prints install commands for anything missing
doctor:
    @python3 .claude/kit/doctor.py

# fix what is safe to fix automatically (permissions, .env from example, agent sync)
doctor-fix:
    @python3 .claude/kit/doctor.py --fix

# cargo fmt + clippy -D warnings + nextest; web typecheck + lint + vitest with compact output (required before any handoff)
gate: duckdb-check
    @python3 .claude/kit/kit.py gate

# developer gate: gate + "production code changed => unit tests added"
dev-gate: gate
    @python3 .claude/kit/kit.py test-check

# tests only (compact)
test: duckdb-check
    @python3 .claude/kit/kit.py test

# check the system DuckDB library the build links (BZ-106): present, headers found, version matches the crate
duckdb-check:
    @sh scripts/duckdb-check.sh

# full local CI: gate + doc tests + cargo deny + pnpm audit
check:
    @python3 .claude/kit/kit.py check

# connection status for configured data stores/services (never prints secrets)
db-check:
    @python3 .claude/kit/kit.py db-check

# regenerate .agents/codemap.md (also runs automatically at session start and after merges/pulls)
# `just codemap find <word>`: only the codemap lines that mention it (cheaper than reading the map)
codemap *args:
    @python3 .claude/kit/kit.py codemap {{args}}

# one section of a long markdown file (roadmap, plan, report); no heading = list the sections and their sizes
section file *heading:
    @python3 .claude/kit/kit.py section {{file}} {{heading}}

# move all but the newest handoff and the dated recommendations files to .agents/archive/
archive-notes:
    @python3 .claude/kit/kit.py archive-notes

# can the team work here? branches, remote, gh, worktrees -> recommended work mode and how to fix problems
preflight:
    @python3 .claude/kit/kit.py preflight

# show or switch how developers work: worktrees (parallel) or in-place (one at a time in this checkout)
work-mode *mode:
    @python3 .claude/kit/kit.py work-mode "$@"

# integration branch that feature PRs target (team.md -> Branches)
base:
    @python3 .claude/kit/kit.py base

# create a work branch from the latest integration branch, e.g. just new-branch feat/F12-orders
new-branch name:
    @python3 .claude/kit/kit.py new-branch {{name}}

# may this PR merge? target branch, Conventional title, CI checks, reviewer verdicts (the only source for "it's green")
merge-check pr *args:
    @python3 .claude/kit/kit.py merge-check "$@"

# review without a PR: just review-pack --diff develop..HEAD (also review-plan / merge-check --branch <b>)
# which reviews a PR needs (code always; security/tests by size and sensitivity - team.md -> Review policy)
review-plan pr *args:
    @python3 .claude/kit/kit.py review-plan "$@"

# everything a reviewer needs in one output: size, required reviews, criteria, red flags, codemap lines, diff
review-pack pr *args:
    @python3 .claude/kit/kit.py review-pack "$@"

# small PRs: all required reviews in one tool-free Claude call (exit 3 = launch the reviewers, 4 = can't run here: one combined-reviewer)
envelope pr *args:
    @python3 .claude/kit/kit.py envelope "$@"

# test the envelope path once (find Claude Code, version, one tiny call) and say what to fix
envelope-check:
    @python3 .claude/kit/kit.py envelope-check

# what an agent reads before it starts working, and how big each part is (no agent = summary of all)
start-report *agent:
    @python3 .claude/kit/kit.py start-report "$@"

# the library: one section of the skills/handbooks on request (e.g. just docs money; --list shows topics)
docs *topic:
    @python3 .claude/kit/kit.py docs "$@"

# reviewers and testers: check a findings file (verdict, severities) and store it where merge-check reads it
save-review *args:
    @python3 .claude/kit/kit.py save-review "$@"

# copy review files that agents wrote inside their worktrees into this checkout (merge-check and triage do it too)
collect-reviews:
    @python3 .claude/kit/kit.py collect-reviews

# zip only the kit (agents, skills, hooks, .agents notes, records, CI) for feedback or upgrades; shows size and contents
export-kit *args:
    @python3 .claude/kit/kit.py export-kit "$@"

# apply the default triage decisions to a PR's findings; lists only what needs the tech lead
triage feature *args:
    @python3 .claude/kit/kit.py triage "$@"

# a feature plan, or one slice's brief (works inside agent worktrees): just plan F3 --slice 2
plan id *args:
    @python3 .claude/kit/kit.py plan "$@"

# close a milestone: rewrite the board's Resume note, archive finished rows, then start a fresh session (/clear)
milestone *text:
    @python3 .claude/kit/kit.py milestone "$@"

# one-shot status for the manager: checkout, work mode, budget, board, open PRs with CI, bugs
status:
    @python3 .claude/kit/kit.py status

# wait for a PR's CI inside the script (free); exit 0 passed, 1 failed, 2 still running
pr-wait pr *args:
    @python3 .claude/kit/kit.py pr-wait "$@"

# tech lead, after deciding a PR's open questions: it may merge once merge-check passes (triage approves by itself when nothing needs judgment)
approve pr *args:
    @python3 .claude/kit/kit.py approve "$@"

# change one team setting safely (checked against what the kit supports); --list shows them all
set *args:
    @python3 .claude/kit/kit.py set "$@"

# where one agent run's tokens went, step by step (newest run of <agent>, or --match "PR 29")
run-report *args:
    @python3 .claude/kit/kit.py run-report "$@"

# agents of this session: continue (SendMessage id) or start fresh, by their context size
warm:
    @python3 .claude/kit/kit.py warm

# efficiency review from the kit's own records (no agent): the current feature, F<n>, --session (this session) or --all
efficiency *args:
    @python3 .claude/kit/kit.py efficiency "$@"

# remove leftover agent worktrees that hold nothing unsaved (branches are kept); --dry-run lists them
worktree-clean *args:
    @python3 .claude/kit/kit.py worktree-clean "$@"

# commit the manager's records (board, decisions, reports, project, codemap) through a small docs PR; no reviewer
records-pr *args:
    @python3 .claude/kit/kit.py records-pr "$@"

# one plan for every open PR, worked out by the script: what to run next, who to launch, what to wait on
next:
    @python3 .claude/kit/kit.py next

# wait in the script (free) until every required review of a PR is in; run it in the background
review-wait *args:
    @python3 .claude/kit/kit.py review-wait "$@"

# the developer's fix brief: only the findings to fix, plus the fix-round rules
fix-list *args:
    @python3 .claude/kit/kit.py fix-list "$@"

# changes since the last review: which ones a finding asked for, which ones nobody asked for
fix-scope *args:
    @python3 .claude/kit/kit.py fix-scope "$@"

# tech lead: mark minor/nit fixes verified after reading the fix diff (never blocking or security/safety findings)
verify-fixes *args:
    @python3 .claude/kit/kit.py verify-fixes "$@"

# deferred findings still open, and whether the feature record carries them (promote refuses lost ones)
deferred *args:
    @python3 .claude/kit/kit.py deferred "$@"

# the kit's own decision tests (fake repos and reviews; no GitHub, no agents) - run after changing .claude/kit/*.py
kit-test:
    @python3 .claude/kit/tests/test_kit.py

# fast-forward your local integration branch to origin (ship does this after every merge)
sync-base:
    @python3 .claude/kit/kit.py sync-base

# merge in one command (run by the manager once the PR is approved): wait for CI, merge-check, merge, update the integration branch, refresh codemap
ship pr *args:
    @python3 .claude/kit/kit.py ship "$@"

# tech lead: merge-check, then squash-merge a feature PR into the integration branch
merge pr *args:
    @python3 .claude/kit/kit.py merge "$@"

# promotion/release PR into the release branch - Claude Code asks the user every time
promote pr:
    @python3 .claude/kit/kit.py promote {{pr}}

# create the integration branch locally and on the remote if missing (asks the user)
setup-branches:
    @python3 .claude/kit/kit.py setup-branches

# show (or --apply) GitHub branch protection for the integration and release branches (asks the user)
protect-branches *args:
    @python3 .claude/kit/kit.py protect-branches "$@"

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
    @python3 .claude/kit/kit.py bug-new "$@"

bugs *args:
    @python3 .claude/kit/kit.py bugs "$@"

# move finished rows from the board to .agents/board-archive/<month>.md
board-archive:
    @python3 .claude/kit/kit.py board-archive

# token budget (runs are measured automatically): manual row with --estimate if unmeasured, status, or check planned launches (e.g. just budget-check F12 dev-data:4 code-reviewer:4)
budget-add task agent tokens *note:
    @python3 .claude/kit/budget.py add "$@"

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
    # `serve` has no option to skip the browser: it opens the printed :5173 link itself (may race Vite startup; reload once).
    target/debug/bazaar-server serve &
    server=$!
    trap 'kill $server 2>/dev/null' EXIT
    pnpm -C web dev

# UI only in the browser with mocked API (MSW) - previews, screenshots, Playwright
app-dev:
    pnpm -C web dev:mock

# one screenshot of a route for T1 evidence (agents may run it): starts its own dev server on a spare port, waits,
# saves {{out}}, stops the server. e.g. just ui-shot /settings 360
ui-shot route="/" width="1440" out="web/.shots/shot.png":
    #!/usr/bin/env bash
    set -euo pipefail
    port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
    mkdir -p "$(dirname "{{out}}")"
    VITE_MOCK_API=1 pnpm -C web exec vite --port "$port" --strictPort --host 127.0.0.1 >"$(dirname "{{out}}")/ui-shot.log" 2>&1 &
    server=$!
    trap 'kill $server 2>/dev/null || true' EXIT
    for _ in $(seq 1 60); do curl -sf "http://127.0.0.1:$port/" >/dev/null && break; sleep 1; done
    pnpm -C web exec playwright screenshot --viewport-size="{{width}},900" --wait-for-timeout=1500 "http://127.0.0.1:$port{{route}}" "{{out}}"
    echo "saved {{out}} ({{width}} wide) - open it with Read and say what you saw"

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
    @test -f crates/backtest/tests/golden.rs || { echo "golden: no golden suite yet (crates/backtest/tests/golden.rs arrives with F9 slice 7b3) - nothing to run"; exit 0; }
    cargo test -q -p bazaar-backtest --test golden

# benchmarks (indicators, scanner, backtest loop)
bench:
    cargo bench --workspace

# coverage report (HTML)
cov:
    cargo llvm-cov nextest --workspace --html

# which CI jobs a change needs (same script CI uses): web / rust / full and the `-p` crate list vs a base
ci-scope base="origin/develop":
    @python3 scripts/ci-scope.py {{base}}

# self-check: a crash in the scope script, or an unknown path, must give a full run (fail safe)
ci-scope-selftest:
    @CI_SCOPE_FILES=crates/core/src/a.rs CI_SCOPE_CRASH=1 python3 scripts/ci-scope.py origin/develop 2>/dev/null | grep -qx 'full=yes' && CI_SCOPE_FILES=.cargo/config.toml python3 scripts/ci-scope.py origin/develop 2>/dev/null | grep -qx 'full=yes' && echo "ci-scope selftest: ok"

# one cargo command under the kit's build slot and job cap, e.g. just cargo nextest run -p bazaar-core my_test
cargo *args:
    @python3 .claude/kit/kit.py cargo "$@"
