#!/usr/bin/env bash
# OWNER RUNS THIS (agents must not). Applies .agents/owner-edits-history-checkpoint.md. Safe to run twice.
# Usage, from the repo root: bash .agents/scripts/apply-human-only.sh [repo-root]
set -euo pipefail
cd "${1:-.}"
python3 -I - <<'PY'
import json, pathlib, sys

def read(p): return pathlib.Path(p).read_text(encoding="utf-8")

out, problems = {}, []

# 1. the human-only list in 14 files
old = "`import-bhavcopy`, `backtest`)"
new = ("`import-bhavcopy`, `backtest`, `bazaar-cli history-checkpoint accept-current`, "
       "`bazaar-cli paper-session new`, `bazaar-cli paper-session resume`)")
files = ["CLAUDE.md", ".agents/stack.json", ".claude/kit/stack.kit.json"]
files += sorted(str(p) for p in pathlib.Path(".claude/agents").glob("*.md"))
done = 0
for f in files:
    t = read(f)
    if old in t:
        out[f] = t.replace(old, new)
        done += 1
    elif new in t:
        done += 1
if done != 14:
    problems.append(f"list found in {done} files, expected 14")

# 2. deny rules
f = ".claude/settings.json"
t = read(f)
rules = []
for cmd in ("history-checkpoint accept-current", "paper-session new", "paper-session resume"):
    rules += [f"Bash(*bazaar-cli*{cmd}*)", f"Bash(just cli-fixture {cmd}*)"]
have = json.loads(t)["permissions"]["deny"]
add = [r for r in rules if r not in have]
if add:
    mark = '"deny": [\n'
    if t.count(mark) != 1:
        problems.append("settings.json: deny block not found exactly once")
    else:
        t = t.replace(mark, mark + "".join(f"      {json.dumps(r)},\n" for r in add))
        json.loads(t)
        out[f] = t

# 3. bazaar-cli reference
f = ".claude/skills/bz-repo-structure/references/bazaar-cli.md"
t = read(f)
for anchor, extra, seen in (
    ("`live-opt-in --mode` (human-only)",
     ", `history-checkpoint verify`, `history-checkpoint accept-current` (human-only), `paper-session list`, "
     "`paper-session new` / `resume` (human-only; not built yet)",
     "`history-checkpoint verify`"),
    ("`bazaar-cli live-opt-in`",
     ", `bazaar-cli history-checkpoint accept-current`, `bazaar-cli paper-session new` / `resume`",
     "`bazaar-cli history-checkpoint accept-current`"),
):
    if seen in t:
        continue
    if t.count(anchor) != 1:
        problems.append(f"bazaar-cli.md: anchor not found exactly once: {anchor}")
        continue
    t = t.replace(anchor, anchor + extra)
    out[f] = t

if problems:
    sys.exit("NOTHING CHANGED:\n  " + "\n  ".join(problems))
for p, text in out.items():
    pathlib.Path(p).write_text(text, encoding="utf-8")
print(f"human-only list in {done} of 14 files; {len(add)} deny rules added; {len(out)} files written")
PY
