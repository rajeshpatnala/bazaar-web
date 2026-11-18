#!/usr/bin/env bash
# PreToolUse (Agent/Task): before the manager launches a FRESH tech lead, architect (or another role in reuse_roles)
# while one already worked in this session, block once and point at the warm agent to continue with SendMessage.
# A fresh launch re-pays the whole cold start (prompt, CLAUDE.md, preloaded skills). Repeating the same launch
# goes through, so genuinely parallel or unrelated work is never stuck. Never blocks anything else.
input=$(cat)
root="${CLAUDE_PROJECT_DIR:-$(pwd)}"
INPUT="$input" ROOT="$root" python3 - <<'PY'
import csv, datetime as dt, hashlib, json, os, pathlib, re, sys
root = pathlib.Path(os.environ["ROOT"])
try:
    data = json.loads(os.environ.get("INPUT") or "{}")
except ValueError:
    sys.exit(0)
ti = data.get("tool_input") or {}
agent, session = ti.get("subagent_type") or "", data.get("session_id") or ""
if not agent or not session:
    sys.exit(0)
roles, window = {"tech-lead", "architect"}, 90
team = root / ".agents" / "team.md"
if team.exists():
    tt = team.read_text(errors="replace")
    m = re.search(r"^\|\s*reuse_roles\s*\|\s*([^|]*)\|", tt, re.M)
    if m:
        roles = {r.strip() for r in m.group(1).split(",") if r.strip() and r.strip() != "-"}
    m = re.search(r"^\|\s*reuse_window_minutes\s*\|\s*(\d+)", tt, re.M)
    if m:
        window = int(m.group(1))
if agent not in roles:
    sys.exit(0)
ledger = root / ".agents" / "budget.csv"
if not ledger.exists():
    sys.exit(0)
now = dt.datetime.now()
prev = None
with ledger.open() as f:
    for r in csv.DictReader(f):
        if r.get("agent") != agent or r.get("session") != session or r.get("source") != "auto":
            continue
        try:
            when = dt.datetime.fromisoformat(f"{r['date']}T{r['time']}")
        except (KeyError, ValueError):
            continue
        if (now - when).total_seconds() <= window * 60:
            prev = r
if not prev:
    sys.exit(0)
key = hashlib.sha1(f"{session}|{agent}|{(ti.get('prompt') or '')[:400]}".encode()).hexdigest()[:16]
seen = root / ".agents" / ".reuse-nudged"
done = seen.read_text().split() if seen.exists() else []
if key in done:
    sys.exit(0)  # second identical launch: the manager decided a fresh agent is needed
seen.write_text("\n".join((done + [key])[-200:]) + "\n")
aid = (re.search(r"id (\S+)$", prev.get("note", "")) or [None, "?"])[1]
start = int(prev.get("start") or 0)
cost = f"~{start // 1000}k tokens of cold start" if start else "its whole cold start"
print(f"Reuse first: a {agent} already worked in this session at {prev['time']} (agent id {aid}). Continue it with "
      f"SendMessage to {aid} - its context is warm, while a fresh launch re-pays {cost}. Only if you need a separate one "
      f"(parallel work or an unrelated area), repeat this exact launch and it will go through.", file=sys.stderr)
sys.exit(2)
PY
exit $?
