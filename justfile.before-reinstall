# Task runner used by humans and agents. Outputs are kept compact on purpose (token budget).
# Stack commands come from .agents/stack.json. just never loads .env; the application loads it itself.

default: gate

# check tools, hooks, settings and agents; prints install commands for anything missing
doctor:
    @python3 .claude/kit/doctor.py

# fix what is safe to fix automatically (permissions, .env from example, agent sync)
doctor-fix:
    @python3 .claude/kit/doctor.py --fix

# typecheck + lint + format + tests + build (= pnpm verify) with compact output (required before any handoff)
gate:
    @python3 .claude/kit/kit.py gate

# developer gate: gate + "production code changed => unit tests added"
dev-gate: gate
    @python3 .claude/kit/kit.py test-check

# tests only (compact)
test:
    @python3 .claude/kit/kit.py test

# full local CI: gate + dependency audit
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

# token budget (runs are measured automatically): manual row with --estimate if unmeasured, status, or check planned launches (e.g. just budget-check F12 dev-ui:4 code-reviewer:4)
budget-add task agent tokens *note:
    @python3 .claude/kit/budget.py add {{task}} {{agent}} {{tokens}} {{note}}

budget task="":
    @python3 .claude/kit/budget.py status {{task}}

# set the task that measured agent runs are booked against (the manager does this when work starts)
budget-task task:
    @python3 .claude/kit/budget.py task {{task}}

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

# run the dev server
dev:
    pnpm dev

# end-to-end tests (needs Playwright browsers)
e2e:
    pnpm e2e

# Storybook
storybook:
    pnpm storybook
