#!/usr/bin/env bash
# PreToolUse (Bash) for ALL agents: protected branches only change through checked PRs.
# - no push to the integration or release branch, no commits/merges/rebases/resets while on them
# - POLICY: never delete, rename, force-move or overwrite the integration or release branch (or the repo)
# - `gh pr merge` runs the merge-check first; PRs into the release branch must use `just promote`
#   (a command Claude Code asks the user about every time)
# The command is parsed (kit/git_guard.py), not text-matched: a branch name inside a grep, a commit message or a
# PR body never blocks anything. GitHub branch protection is the authoritative safeguard; this hook stops mistakes early.
root="${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel 2>/dev/null || pwd)}"
exec python3 "$root/.claude/kit/git_guard.py" branch "$@"
