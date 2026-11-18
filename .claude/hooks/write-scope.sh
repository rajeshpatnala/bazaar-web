#!/usr/bin/env bash
# PreToolUse (Edit|Write) for coordinating/reviewing agents.
# Usage: write-scope.sh <allowed-dir>...   e.g. write-scope.sh .agents docs
# Allows writes only under the given repo-relative dirs (and the agent's memory dir).
input=$(cat)
command -v jq >/dev/null 2>&1 || { echo "write-scope: jq is required" >&2; exit 2; }
p=$(jq -r '.tool_input.file_path // .tool_input.notebook_path // empty' <<<"$input")
[ -z "$p" ] && exit 0

root="${CLAUDE_PROJECT_DIR:-$(pwd)}"
rel="${p#"$root"/}"
for d in "$@" .claude/agent-memory .claude/agent-memory-local; do
  case "$rel" in "$d"/*) exit 0 ;; esac
  case "$p" in */"$d"/*) exit 0 ;; esac
done
echo "Blocked: this agent may only write under: $* (plus its memory). Source changes are made by developers - report it to the manager instead." >&2
exit 2
