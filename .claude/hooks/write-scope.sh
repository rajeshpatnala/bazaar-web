#!/usr/bin/env bash
# PreToolUse (Edit|Write) for coordinating/reviewing agents.
# Usage: write-scope.sh <allowed-dir>...   e.g. write-scope.sh .agents docs
# Allows writes only under the given repo-relative dirs (and the agent's memory dir).
input=$(cat)
command -v jq >/dev/null 2>&1 || { echo "write-scope: jq is required" >&2; exit 2; }
# Agent-attached hooks can also fire for tool calls of OTHER agents (e.g. the manager's hooks while it runs as the
# main session, or two sub-agents running at once). `--for <agent>` makes this hook apply only to that agent:
# when the call comes from a different agent (hook input `agent_type`), it is not ours to judge.
for_agent=""
if [ "${1:-}" = "--for" ]; then for_agent="${2:-}"; shift 2; fi
caller=$(jq -r '.agent_type // empty' <<<"$input" 2>/dev/null)
if [ -n "$for_agent" ] && [ -n "$caller" ] && [ "$caller" != "$for_agent" ]; then exit 0; fi
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
