#!/usr/bin/env bash
# SessionStart and SubagentStart: give every agent (and the manager) the owner's standing rules from
# .agents/owner-rules.md and the current work mode, so they never depend on the manager retyping them.
# Prints nothing when there is nothing to add. Never blocks.
input=$(cat)
root="${CLAUDE_PROJECT_DIR:-$(pwd)}"
INPUT="$input" ROOT="$root" python3 - <<'PY' 2>/dev/null || true
import json, os, pathlib, re
root = pathlib.Path(os.environ["ROOT"])
try:
    event = json.loads(os.environ.get("INPUT") or "{}").get("hook_event_name", "SubagentStart")
except ValueError:
    event = "SubagentStart"
parts = []
rules = root / ".agents" / "owner-rules.md"
if rules.exists():
    lines = [l.strip() for l in rules.read_text(errors="replace").splitlines() if re.match(r"^\s*-\s+\S", l)]
    if lines:
        parts.append("Owner rules (standing decisions by the project owner; they apply to every task, everywhere in the app):\n"
                     + "\n".join(lines[:30]))
team = root / ".agents" / "team.md"
if team.exists():
    m = re.search(r"^\|\s*work_mode\s*\|\s*`?([\w-]+)", team.read_text(errors="replace"), re.M)
    if m and m.group(1) == "in-place":
        parts.append("Work mode: in-place - no worktrees; one developer at a time in the main checkout. "
                     "Commit checkpoints on your feature branch; reviews use `just review-pack --diff <base>..HEAD`.")
if parts:
    print(json.dumps({"hookSpecificOutput": {"hookEventName": event, "additionalContext": "\n\n".join(parts)}}))
PY
exit 0
