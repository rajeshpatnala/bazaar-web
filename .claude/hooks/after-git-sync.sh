#!/usr/bin/env bash
# PostToolUse (Bash): after merges, pulls, rebases onto main, pushes or branch creation, refresh the
# code map in the main checkout so in-flight and merged work are always on the map.
input=$(cat)
command -v jq >/dev/null 2>&1 || exit 0
cmd=$(jq -r '.tool_input.command // ""' <<<"$input")
if grep -Eq 'gh[[:space:]]+pr[[:space:]]+merge|git[[:space:]]+(merge|pull|rebase|push|switch[[:space:]]+-c|checkout[[:space:]]+-b|branch[[:space:]]+-[dD])' <<<"$cmd"; then
  KIT_ROOT="${CLAUDE_PROJECT_DIR:-$(pwd)}" python3 "${CLAUDE_PROJECT_DIR:-.}/.claude/kit/kit.py" codemap >/dev/null 2>&1 || true
fi
exit 0
