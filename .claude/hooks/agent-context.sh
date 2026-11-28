#!/usr/bin/env bash
# SessionStart and SubagentStart: give every agent (and the manager) the owner's standing rules from
# .agents/owner-rules.md and the current work mode, so they never depend on the manager retyping them.
# Agents listed in team.md owner_rules_skip (default build-runner, cicd) get no owner rules.
# Prints nothing when there is nothing to add. Never blocks.
input=$(cat)
root="${CLAUDE_PROJECT_DIR:-$(pwd)}"
INPUT="$input" ROOT="$root" python3 - <<'PY' 2>/dev/null || true
import json, os, pathlib, re
root = pathlib.Path(os.environ["ROOT"])
try:
    data = json.loads(os.environ.get("INPUT") or "{}")
except ValueError:
    data = {}
event = data.get("hook_event_name", "SubagentStart")
agent = data.get("agent_type", "")
team = root / ".agents" / "team.md"
team_text = team.read_text(errors="replace") if team.exists() else ""
# Agents that never decide how the app behaves (team.md owner_rules_skip) don't need the owner's product rules.
m = re.search(r"^\|\s*owner_rules_skip\s*\|\s*([^|]*)\|", team_text, re.M)
skip = {x.strip(" `") for x in (m.group(1) if m else "build-runner, cicd").split(",") if x.strip(" `-")}
parts = []
rules = root / ".agents" / "owner-rules.md"
if rules.exists() and not (event == "SubagentStart" and agent in skip):
    text = re.sub(r"<!--.*?-->", "", rules.read_text(errors="replace"), flags=re.S)  # examples in comments are not rules
    lines = [l.strip() for l in text.splitlines() if re.match(r"^\s*-\s+\S", l)]
    if lines:
        parts.append("Owner rules (standing decisions by the project owner; they apply to every task, everywhere in the app):\n"
                     + "\n".join(lines[:30]))
if team_text:
    m = re.search(r"^\|\s*work_mode\s*\|\s*`?([\w-]+)", team_text, re.M)
    if m and m.group(1) == "in-place":
        parts.append("Work mode: in-place - no worktrees; one developer at a time in the main checkout. "
                     "Commit checkpoints on your feature branch; reviews use `just review-pack --diff <base>..HEAD`.")
if parts:
    print(json.dumps({"hookSpecificOutput": {"hookEventName": event, "additionalContext": "\n\n".join(parts)}}))
PY
exit 0
