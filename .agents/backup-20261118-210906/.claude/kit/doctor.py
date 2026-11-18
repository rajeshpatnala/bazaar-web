#!/usr/bin/env python3
"""Health check for the agent kit: tools (from .agents/stack.json), hooks, settings, agents, .env presence.

  doctor.py            full report with install commands for anything missing
  doctor.py --quiet    print only problems, in a few lines (used at session start)
  doctor.py --fix      also fix what is safe to fix automatically (permissions, .env, agent sync)

Never reads .env - it only checks that the file exists.
"""
import json
import os
import pathlib
import platform
import shutil
import stat
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
QUIET = "--quiet" in sys.argv
FIX = "--fix" in sys.argv

BASE_TOOLS = [  # every kit needs these
    {"name": "git", "why": "version control", "required": True},
    {"name": "jq", "why": "used by the safety hooks", "required": True},
    {"name": "python3", "why": "kit scripts", "required": True},
    {"name": "just", "why": "task runner used by agents", "required": True, "install": "cargo install --locked just  # or brew/apt/winget install just"},
    {"name": "gh", "why": "GitHub PRs (agents open PRs with it)", "required": False},
    {"name": "git-cliff", "why": "offline changelog", "required": False, "install": "cargo install --locked git-cliff  # or brew install git-cliff"},
]


def stack() -> dict:
    p = ROOT / ".agents" / "stack.json"
    return json.loads(p.read_text()) if p.exists() else {}


def tools() -> list:
    seen, out = set(), []
    for t in stack().get("tools", []) + BASE_TOOLS:
        if t["name"] not in seen:
            seen.add(t["name"])
            out.append(t)
    return out


def have(t: dict) -> bool:
    if t.get("check"):
        return subprocess.run(t["check"], shell=True, capture_output=True).returncode == 0
    return shutil.which(t["name"]) is not None


def install_hint(missing: list) -> list:
    hints, plain = [], []
    for t in missing:
        if t.get("install"):
            hints.append(t["install"])
        else:
            plain.append(t["name"])
    if plain:
        sysname = platform.system()
        if sysname == "Darwin":
            hints.append("brew install " + " ".join(plain))
        elif sysname == "Linux":
            hints.append("sudo apt-get install -y " + " ".join(plain) + "   # or dnf/pacman equivalents")
        else:
            hints.append("winget/scoop install " + " ".join(plain) + "   (run Claude Code from Git Bash or WSL)")
    return list(dict.fromkeys(hints))


def main() -> int:
    problems, notes, fixed = [], [], []

    all_tools = tools()
    missing_req = [t for t in all_tools if t.get("required") and not have(t)]
    missing_rec = [t for t in all_tools if not t.get("required") and not have(t)]
    if missing_req:
        problems.append("missing required tools: " + ", ".join(t["name"] for t in missing_req))
    if missing_rec:
        notes.append("missing recommended tools: " + ", ".join(t["name"] for t in missing_rec))

    # executable hooks and scripts
    not_exec = [p for p in list((ROOT / ".claude" / "hooks").glob("*.sh")) + list((ROOT / ".claude" / "kit").glob("*"))
                if p.is_file() and not os.access(p, os.X_OK)]
    if not_exec:
        if FIX:
            for p in not_exec:
                p.chmod(p.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
            fixed.append(f"made {len(not_exec)} hook/script files executable")
        else:
            problems.append(f"{len(not_exec)} hook/script files are not executable (fix: python3 .claude/kit/doctor.py --fix)")

    # settings
    settings = ROOT / ".claude" / "settings.json"
    try:
        s = json.loads(settings.read_text())
        if s.get("agent") != "manager":
            problems.append(".claude/settings.json: \"agent\" is not \"manager\" - prompts won't go to the manager")
    except Exception as e:  # noqa: BLE001
        problems.append(f".claude/settings.json invalid: {e}")

    # agents in sync with team.md
    sync = subprocess.run([sys.executable, str(ROOT / ".claude" / "kit" / "agents_sync.py"), "--check"],
                          capture_output=True, text=True)
    if sync.returncode != 0:
        if FIX and "ERROR" not in sync.stdout:
            subprocess.run([sys.executable, str(ROOT / ".claude" / "kit" / "agents_sync.py")], capture_output=True)
            fixed.append("applied .agents/team.md to agent files")
        else:
            problems.append("agent files differ from .agents/team.md: " + sync.stdout.strip().splitlines()[0])

    # .env presence (never read)
    env, example = ROOT / ".env", ROOT / ".env.example"
    if not env.exists():
        if FIX and example.exists():
            shutil.copyfile(example, env)
            fixed.append("created .env from .env.example - fill in the values yourself")
        else:
            notes.append(".env not found - copy .env.example to .env and fill it (agents never read it)")

    # git repo / gh auth
    if not (ROOT / ".git").exists():
        problems.append("not a git repository (run: git init -b main)")
    if shutil.which("gh") and subprocess.run(["gh", "auth", "status"], capture_output=True).returncode != 0:
        notes.append("gh is not authenticated (run: gh auth login) - agents will keep PRs as local branches")

    # kit not committed: worktree and cloud sessions start from the last commit and won't see it
    if (ROOT / ".git").exists():
        tracked = subprocess.run(["git", "ls-files", "--", ".claude/settings.json", ".claude/agents"], cwd=ROOT,
                                 capture_output=True, text=True).stdout.strip()
        if not tracked:
            notes.append("kit files are not committed yet - sessions in a worktree or the cloud won't see the manager; "
                         "commit them on a branch (git switch -c chore/agent-kit && git add -A && git commit -m \"chore: add agent kit\")")

    # instruction files every agent reads on every call: keep them small
    for name, limit in (("CLAUDE.md", 12_000), ("AGENTS.md", 12_000)):
        f = ROOT / name
        if f.exists() and f.stat().st_size > limit:
            notes.append(f"{name} is {f.stat().st_size // 1000} KB; every agent re-reads it on every call. Keep it under "
                         f"{limit // 1000} KB: move history and long how-tos to docs/ and link them (ask the manager to \"slim {name}\")")
    profile = ROOT / ".agents" / "project.md"
    if profile.exists() and "<!--" in profile.read_text()[:400] and any((ROOT / n).exists() for n in ("PROJECT.md", "ARCHITECTURE.md", "docs/architecture.md")):
        notes.append("the project already has its own docs but .agents/project.md is still the template - ask the manager to "
                     "\"fill in the project profile\" (it writes a short digest that points at your docs)")

    # feature records: feature work in history but no records yet -> suggest a backfill
    if (ROOT / ".git").exists():
        feats = [p for p in (ROOT / "docs" / "features").glob("F*.md")] if (ROOT / "docs" / "features").is_dir() else []
        feat_commits = subprocess.run(["git", "log", "--oneline", "-E", "--grep=^feat(\\(|:|!)", "-n", "50"], cwd=ROOT,
                                      capture_output=True, text=True).stdout.split("\n")
        n = len([l for l in feat_commits if l.strip()])
        if n and not feats:
            notes.append(f"{n}+ feature commits but no feature records in docs/features/ - ask the manager to \"backfill the feature records\"")

    if QUIET:
        if problems or missing_req:
            cmd = "just doctor" if shutil.which("just") else "python3 .claude/kit/doctor.py"
            print("Kit health: " + "; ".join(problems) + f". Run `{cmd}` for fixes.")
        backfill = [n for n in notes if any(k in n for k in ("backfill the feature records", "not committed yet", " KB; every agent", "still the template"))]
        for n in backfill:
            print("Kit note: " + n + ".")
        return 1 if problems else 0

    print(f"{stack().get('title', 'Agent')} kit - doctor\n")
    for f in fixed:
        print("fixed    " + f)
    for p in problems:
        print("PROBLEM  " + p)
    for n in notes:
        print("note     " + n)
    hints = install_hint(missing_req + missing_rec)
    if hints:
        print("\nInstall missing tools:")
        for h in hints:
            print("  " + h)
    if not problems:
        print("\nAll required checks passed." + (" Recommended tools are optional." if missing_rec else ""))
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
