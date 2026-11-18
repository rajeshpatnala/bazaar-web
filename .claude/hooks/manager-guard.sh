#!/usr/bin/env bash
# PreToolUse (Bash|Read) for the manager: its conversation is re-sent on every step, so raw diffs, lockfiles and
# generated files stay out of it (summaries come from `just review-plan`, `just status`, `git diff --stat`).
# `--for manager`: agent-attached hooks can fire for other agents' calls; this judges only the manager's.
root="${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel 2>/dev/null || pwd)}"
exec python3 "$root/.claude/kit/git_guard.py" manager "$@"
