#!/usr/bin/env python3
"""Status line for the kit (Claude Code runs it after each turn with session data on stdin).

- Records your real plan usage (5-hour and 7-day windows) to .agents/.usage-limits.json, so `just status`,
  `just budget` and the session hook can warn at 70/85/95% - the kit's own token counts can't see your plan.
- Runs your personal status line (from ~/.claude/settings.json) first, if you have one, and keeps its output.
- Adds one short kit line: context use, plan usage.
Must stay fast and never fail: any error just prints less.
"""
import json
import os
import pathlib
import subprocess
import sys
import time


def main() -> int:
    raw = sys.stdin.read()
    try:
        data = json.loads(raw or "{}")
    except ValueError:
        data = {}
    root = pathlib.Path((data.get("workspace") or {}).get("project_dir") or os.environ.get("CLAUDE_PROJECT_DIR") or os.getcwd())
    rl = data.get("rate_limits") or {}
    if rl and (root / ".agents").is_dir():
        try:
            (root / ".agents" / ".usage-limits.json").write_text(json.dumps({**rl, "at": int(time.time())}))
        except OSError:
            pass
    out = []
    try:  # chain the owner's personal status line (project settings would otherwise replace it)
        user = json.loads((pathlib.Path.home() / ".claude" / "settings.json").read_text()).get("statusLine") or {}
        cmd = user.get("command", "") if user.get("type", "command") == "command" else ""
        if cmd and "statusline.py" not in cmd:
            r = subprocess.run(cmd, shell=True, input=raw, capture_output=True, text=True, timeout=3)
            if r.stdout.strip():
                out.append(r.stdout.rstrip("\n"))
    except (OSError, ValueError, subprocess.TimeoutExpired):
        pass
    parts = []
    cw = data.get("context_window") or {}
    if cw.get("used_percentage") is not None:
        parts.append(f"context {float(cw['used_percentage']):.0f}%")
    for key, label in (("five_hour", "5h"), ("seven_day", "7d")):
        w = rl.get(key) or {}
        if w.get("used_percentage") is not None:
            pct = float(w["used_percentage"])
            mark = " !" if pct >= 85 else ""
            parts.append(f"{label} {pct:.0f}%{mark}")
    if parts:
        out.append("kit: " + " | ".join(parts))
    print("\n".join(out))
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception:  # noqa: BLE001 - a status line must never break the UI
        sys.exit(0)
