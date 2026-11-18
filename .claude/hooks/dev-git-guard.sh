#!/usr/bin/env bash
# PreToolUse (Bash) for developers, testers and cicd: no merging, promoting or branch setup, no pushing to the
# protected branches, no force push, no commits on protected branches. Only the tech lead merges.
# `--for <agent>`: agent-attached hooks can also fire for OTHER agents' tool calls; this makes the hook judge only
# that agent's calls. The command is parsed (kit/git_guard.py), not text-matched.
root="${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel 2>/dev/null || pwd)}"
exec python3 "$root/.claude/kit/git_guard.py" dev "$@"
