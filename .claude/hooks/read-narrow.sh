#!/usr/bin/env bash
# PreToolUse on Read (all agents): a long markdown file read whole costs its full size in tokens on every later step
# of that agent. Blocks a whole read of a .md file over team.md read_whole_kb (default 24) and names the narrow way:
# `just codemap find`, `just plan <id> --slice <n>`, `just section <file> "<heading>"`, or Read with offset/limit.
# Exit 2 = block the call and show the reason to the agent. Never blocks anything else.
input=$(cat)
root="${CLAUDE_PROJECT_DIR:-$(pwd)}"
INPUT="$input" ROOT="$root" python3 - <<'PY'
import json, os, pathlib, re, sys
try:
    d = json.loads(os.environ.get("INPUT") or "{}")
except ValueError:
    sys.exit(0)
ti = d.get("tool_input") or {}
path = ti.get("file_path") or ""
if d.get("tool_name") != "Read" or not path.endswith(".md") or ti.get("offset") or ti.get("limit"):
    sys.exit(0)
root = pathlib.Path(os.environ["ROOT"])
limit_kb = 24.0
team = root / ".agents" / "team.md"
if team.exists():
    m = re.search(r"^\|\s*read_whole_kb\s*\|\s*`?([\d.]+)", team.read_text(errors="replace"), re.M)
    if m:
        limit_kb = float(m.group(1))
f = pathlib.Path(path)
f = f if f.is_absolute() else root / f
try:
    size = f.stat().st_size
except OSError:
    sys.exit(0)
if not limit_kb or size <= limit_kb * 1024:
    sys.exit(0)
name = f.as_posix()
if re.search(r"\.agents/codemap(/[^/]+)?\.md$", name):
    how = "`just codemap find <word>` (the file, type or route you need), or the one area file under .agents/codemap/"
elif ("/.agents/plans/" in name or name.startswith(".agents/plans/")) and "roadmap" not in f.name.lower():
    how = "`just plan <id> --slice <n>` for one slice, or `just section " + path + " \"<heading>\"`"
else:
    how = "`just section " + path + "` to list its sections, then `just section " + path + " \"<heading>\"`"
print(f"Blocked by read-narrow: {path} is {size // 1024} KB (~{size // 4:,} tokens), and every later step re-sends it. "
      f"Use {how}; or Read it with offset/limit; or grep it. (team.md read_whole_kb = {limit_kb:g})", file=sys.stderr)
sys.exit(2)
PY
