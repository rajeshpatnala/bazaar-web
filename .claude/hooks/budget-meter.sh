#!/usr/bin/env bash
# SubagentStop (every agent) and Stop (the manager's session): measure real token usage from the transcripts
# and book it in .agents/budget.csv / budget-sessions.json, so the budget never relies on estimates.
input=$(cat)
root="${CLAUDE_PROJECT_DIR:-$(pwd)}"
case "$(printf '%s' "$input" | python3 -c 'import json,sys;print(json.load(sys.stdin).get("hook_event_name",""))' 2>/dev/null)" in
  SubagentStop) printf '%s' "$input" | python3 "$root/.claude/kit/budget.py" hook-subagent >/dev/null 2>&1 ;;
  Stop)         printf '%s' "$input" | python3 "$root/.claude/kit/budget.py" hook-session  >/dev/null 2>&1 ;;
esac
exit 0
