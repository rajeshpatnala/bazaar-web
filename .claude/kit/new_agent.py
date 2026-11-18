#!/usr/bin/env python3
"""Install, update or retire an agent. Normally run by the manager after a hiring interview
(skill: agent-hiring); humans can run it too.

  new_agent.py <name> --kind <developer|reviewer|lead|runner> --description "Called for ..."
               [--skills a,b] [--effort medium] [--max-turns 60]
               [--body-file .agents/hiring/<name>.md] [--skill-file .agents/hiring/<name>-skill.md]
               [--new-skill]                       # empty skill stub instead of --skill-file
  new_agent.py <name> --update [--description ...] [--skills a,b] [--body-file ...] [--skill-file ...]
  new_agent.py <name> --retire                     # move to .claude/agents-retired/, remove row + hooks

Install does: .claude/agents/<name>.md (tools, worktree isolation, hooks and write limits from the
kind; the standard team rules for the kind are appended to the body), optional skill, a row in
.agents/team.md, and for developers the SubagentStop gate registration in .claude/settings.json.
Start a new Claude Code session afterwards so the agent can be launched.
"""
import argparse
import json
import pathlib
import re
import shutil
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
AGENTS = ROOT / ".claude" / "agents"
RETIRED = ROOT / ".claude" / "agents-retired"
SKILLS = ROOT / ".claude" / "skills"
TEAM = ROOT / ".agents" / "team.md"
SETTINGS = ROOT / ".claude" / "settings.json"
MODEL = "claude-sonnet-5-5"
STACK = json.loads((ROOT / ".agents" / "stack.json").read_text()) if (ROOT / ".agents" / "stack.json").exists() else {}
ROLES = STACK.get("skill_roles", {})
CORE = [ROLES["core"]] if ROLES.get("core") else []
TESTING = [ROLES["testing"]] if ROLES.get("testing") else []
RULES_MARK = "<!-- team-rules -->"

KINDS = {
    "developer": dict(
        tools="Read, Glob, Grep, Write, Edit, Bash, Skill", isolation=True, memory=True,
        hook=("Bash", "dev-git-guard.sh"), skills=CORE + TESTING + ["git-workflow"], turns=80,
        rules="""## Team rules (developer)
- One step or slice per launch on your own branch (`just new-branch <type>/<feature-id>-<slug>`; your own worktree, or the main checkout in in-place mode); PRs target the integration branch (`gh pr create --base "$(just base)"`). You may be continued for the next step: keep what you know.
- Read the plan slice or brief, `.agents/project.md`, `.agents/codemap.md`, then only the files you change. Follow the owner rules from your start-up context everywhere.
- Ambiguity or conflict with the skills -> stop and return a question.
- **Unit tests are mandatory** for every new behaviour and error path (mocks only). Verify T0 (`just dev-gate`) + T1 (one quick look at what changed); never the full T2 matrix.
- Commit with Conventional Commits, push, open a PR (`git-workflow`). Never merge or push to the integration or release branch.
- Never read `.env*`/keys/`secrets/`, never connect to real data stores (only `just db-check`).
- Return the handoff (`.agents/templates/handoff.md`), <= 15 lines, with *Where to look*.
""",
        body="""You are **{name}**, a developer on the AI delivery team.

**Specialty:** TODO - what this developer is best at and what it owns.
"""),
    "reviewer": dict(
        tools="Read, Glob, Grep, Bash, Write, Edit, Skill", isolation=False, memory=False, omit_claude_md=True,
        hook=("Edit|Write", "write-scope.sh .agents"), skills=CORE, turns=20,
        rules="""## Team rules (reviewer)
- Start with one `just review-pack <pr>` (or `--diff <base>..<branch>`): diff, files, acceptance criteria and your output path; not the whole repo. Bash only for that and read-only git/rg.
- Write findings to the path `review-pack` prints for you (template `.agents/templates/review-findings.md`, ids `{prefix}-<feature>-<n>`); update statuses in place on re-review.
- Never change source code, commit, push or merge. The tech lead triages; developers fix; you verify.
- Never read `.env*`/keys/`secrets/`.
- Return <= 10 lines: verdict, counts by severity, blocker ids, findings path.
- You start without CLAUDE.md or a memory notebook: `just review-pack`, your skills and this file are what you need.
""",
        body="""You are **{name}**, a reviewer on the AI delivery team.

**Focus:** TODO - what you check and why.
"""),
    "lead": dict(
        tools="Read, Glob, Grep, Write, Edit, Bash, Skill, WebSearch, WebFetch", isolation=False, memory=True,
        hook=("Edit|Write", "write-scope.sh .agents docs"), skills=[], turns=40,
        rules="""## Team rules (lead)
- You produce plans, designs or decisions, not source code (writes limited to `.agents/` and `docs/`).
- Record lasting decisions in `.agents/decisions.md`.
- Never read `.env*`/keys/`secrets/`.
- Return to the manager <= 20 lines: decisions, open questions, files written.
""",
        body="""You are **{name}** on the AI delivery team.

**Responsibility:** TODO.
"""),
    "prototyper": dict(
        tools="Read, Glob, Grep, Write, Edit, Bash, Skill", isolation=False, memory=False,
        hook=("Edit|Write", "write-scope.sh previews .agents"), skills=[], turns=30,
        rules="""## Team rules (prototyper)
- You make throwaway previews so the owner can choose a direction by looking, before anything is built.
- Write only under `previews/<task-id>/` (gitignored). No git commits, no dependency installs, no changes to the product.
- At most 3 clearly different directions, plus an index page or file listing them with one line each.
- Use the project's real names, brand, sample data shapes and assets where they exist; never real customer data or secrets.
- Check each preview once (open, run or screenshot it) and say what you saw.
- Return <= 10 lines: the paths, one line per direction, and the question for the owner.
""",
        body="""You are **{name}**, the team's prototyper.

**Previews you make:** TODO.
"""),
    "runner": dict(
        tools="Bash, Read", isolation=True, memory=False, hook=None, skills=[], turns=15,
        rules="""## Team rules (runner)
- You run commands and summarise results; you don't edit code.
- Never read `.env*`/keys/`secrets/`, never print environment variables.
- Return at most 20 lines: command, PASS/FAIL, first relevant errors.
""",
        body="""You are **{name}** on the AI delivery team.

**Commands you run:** TODO.
"""),
}


def fail(msg: str) -> None:
    sys.exit(f"new_agent: {msg}")


def safe_desc(d: str) -> str:
    d = " ".join(d.split())
    return d.replace(": ", " - ").lstrip("'\"")  # keep YAML plain-scalar safe


def split_doc(text: str):
    m = re.match(r"^---\n(.*?)\n---\n", text, re.S)
    if not m:
        fail("agent file has no frontmatter")
    return m.group(1).split("\n"), text[m.end():]


def read_body_file(path: str) -> str:
    p = (ROOT / path) if not pathlib.Path(path).is_absolute() else pathlib.Path(path)
    if not p.exists():
        fail(f"body file not found: {path}")
    text = p.read_text().strip()
    if text.startswith("---"):  # strip accidental frontmatter
        text = re.sub(r"^---\n.*?\n---\n", "", text, flags=re.S).strip()
    return text


def check_skills(skills, allow_new=None):
    missing = [s for s in skills if s != allow_new and not (SKILLS / s / "SKILL.md").exists()]
    if missing:
        fail(f"unknown skills: {', '.join(missing)} (available: {', '.join(sorted(p.name for p in SKILLS.iterdir() if p.is_dir()))})")


def write_skill(name: str, skill_file, stub: bool, description: str) -> None:
    sk = SKILLS / name / "SKILL.md"
    sk.parent.mkdir(parents=True, exist_ok=True)
    content = read_body_file(skill_file) if skill_file else f"# {name}\n\nTODO - standards, procedures and examples for this specialty."
    sk.write_text(f"---\nname: {name}\ndescription: {safe_desc('Specialist knowledge for ' + name + ' - ' + description)}\n---\n\n{content}\n")
    print(f"skill  {sk.relative_to(ROOT)}")


def with_for(script: str, agent: str) -> str:
    """'write-scope.sh .agents' -> 'write-scope.sh --for <agent> .agents' (the hook then applies to this agent only)."""
    name, _, rest = script.partition(" ")
    return f"{name} --for {agent}" + (f" {rest}" if rest else "")


def kind_of(front) -> str:
    tools = next((l for l in front if l.startswith("tools:")), "")
    iso = any(l.startswith("isolation:") for l in front)
    hooks = "\n".join(front)
    if "dev-git-guard.sh" in hooks:
        return "developer"
    if re.search(r"write-scope\.sh( --for \S+)? \.agents docs", hooks):
        return "lead"
    if re.search(r"write-scope\.sh( --for \S+)? \.agents", hooks):
        return "reviewer"
    return "runner" if iso and "Write" not in tools else "lead"


def team_rows(add: dict = None, remove: str = None) -> None:
    """Add a row (values keyed by column name) to, or remove an agent from, every agent table in team.md."""
    lines, out, i, seen = TEAM.read_text().splitlines(), [], 0, False
    while i < len(lines):
        line = lines[i]
        cells = [c.strip() for c in line.strip().strip("|").split("|")] if line.strip().startswith("|") else []
        if cells and cells[0].lower() == "agent":
            seen = True
            header = [c.lower() for c in cells]
            out.append(line)
            i += 1
            while i < len(lines) and lines[i].strip().startswith("|"):
                if not (remove and lines[i].strip().startswith(f"| {remove} |")):
                    out.append(lines[i])
                i += 1
            if add:
                out.append("| " + " | ".join(str(add.get(h, "-")) for h in header) + " |")
            continue
        out.append(line)
        i += 1
    if not seen:
        fail("agent table not found in .agents/team.md")
    TEAM.write_text("\n".join(out) + "\n")


def gate_hook(name: str, register: bool) -> None:
    s = json.loads(SETTINGS.read_text())
    for entry in s.get("hooks", {}).get("SubagentStop", []):
        if "dev-gate.sh" in json.dumps(entry):
            names = [n for n in entry["matcher"].split("|") if n and n != name]
            if register:
                names.append(name)
            entry["matcher"] = "|".join(names)
    SETTINGS.write_text(json.dumps(s, indent=2) + "\n")


def install(a) -> None:
    path = AGENTS / f"{a.name}.md"
    if path.exists():
        fail(f"{path.relative_to(ROOT)} already exists (use --update)")
    if not a.kind or not a.description:
        fail("--kind and --description are required to install")
    k = KINDS[a.kind]
    new_skill = a.name if (a.skill_file or a.new_skill) else None
    skills = list(dict.fromkeys(k["skills"] + [s.strip() for s in (a.skills or "").split(",") if s.strip()] + ([new_skill] if new_skill else [])))
    check_skills(skills, allow_new=new_skill)
    turns = a.max_turns or k["turns"]
    effort = a.effort or "medium"

    front = [f"name: {a.name}", f"description: {safe_desc(a.description)}", f"model: {MODEL}",
             f"effort: {effort}", f"maxTurns: {turns}"]
    if k["isolation"]:
        front.append("isolation: worktree")
    front.append(f"tools: {k['tools']}")
    if skills:
        front += ["skills:"] + [f"  - {s}" for s in skills]
    if k["memory"]:
        front.append("memory: project")
    if k.get("omit_claude_md"):
        front.append("omitClaudeMd: true")  # reviewers work from review-pack + skills; keeps their start small
    if k["hook"]:
        matcher, script = k["hook"]
        front += ["hooks:", "  PreToolUse:", f'    - matcher: "{matcher}"', "      hooks:",
                  "        - type: command",
                  f'          command: "\\"$CLAUDE_PROJECT_DIR\\"/.claude/hooks/{with_for(script, a.name)}"']
    prefix = "".join(w[0] for w in a.name.split("-")).upper()[:4] or "RV"
    if prefix in {"ADR", "SEC", "CR", "TS"}:  # avoid clashing with ADRs and built-in finding ids
        prefix += "X"
    body = read_body_file(a.body_file) if a.body_file else k["body"].format(name=a.name).strip()
    rules = k["rules"].format(name=a.name, prefix=prefix)
    path.write_text("---\n" + "\n".join(front) + "\n---\n\n" + body + "\n\n" + RULES_MARK + "\n" + rules)
    print(f"agent  {path.relative_to(ROOT)}  ({a.kind}, effort {effort}, max turns {turns})")
    if new_skill:
        write_skill(a.name, a.skill_file, a.new_skill, a.description)
    caps = {"developer": "400,000", "reviewer": "150,000", "lead": "200,000", "runner": "100,000"}
    team_rows(add={"agent": a.name, "model": MODEL, "effort": effort, "max_turns": turns,
                   "max_tokens_per_run": f"{int(a.max_tokens.replace(',', '')):,}" if a.max_tokens else caps[a.kind], "context_window": "-", "cache_ttl": "-",
                   "memory": "project" if k["memory"] else "none", "permission_mode": "-"})
    print("team   row added to .agents/team.md")
    if a.kind == "developer":
        gate_hook(a.name, True)
        print("hook   registered for the developer gate")
    if not a.body_file:
        print("note   fill the TODO lines in the agent file")


def update(a) -> None:
    path = AGENTS / f"{a.name}.md"
    if not path.exists():
        fail(f"no agent named {a.name}")
    front, body = split_doc(path.read_text())
    kind = kind_of(front)
    if a.description:
        front = [f"description: {safe_desc(a.description)}" if l.startswith("description:") else l for l in front]
    if a.skills is not None or a.skill_file:
        new_skill = a.name if a.skill_file else None
        skills = list(dict.fromkeys(KINDS[kind]["skills"] + [s.strip() for s in (a.skills or "").split(",") if s.strip()] + ([new_skill] if new_skill else [])))
        current = [l.strip()[2:] for l in front if l.startswith("  - ")]
        if a.skills is None:  # keep existing list, just add the new skill
            skills = list(dict.fromkeys(current + ([new_skill] if new_skill else [])))
        elif a.name in current and a.name not in skills:  # never drop the agent's own skill silently
            skills.append(a.name)
        check_skills(skills, allow_new=new_skill)
        out, skip = [], False
        for l in front:
            if l.startswith("skills:"):
                skip = True
                continue
            if skip and l.startswith("  - "):
                continue
            skip = False
            out.append(l)
        idx = next(i for i, l in enumerate(out) if l.startswith("tools:")) + 1
        front = out[:idx] + (["skills:"] + [f"  - {s}" for s in skills] if skills else []) + out[idx:]
        if new_skill:
            desc = next(l.split(":", 1)[1].strip() for l in front if l.startswith("description:"))
            write_skill(a.name, a.skill_file, False, desc)
    if a.body_file:
        rules = body[body.index(RULES_MARK):] if RULES_MARK in body else RULES_MARK + "\n" + KINDS[kind]["rules"].format(
            name=a.name, prefix="".join(w[0] for w in a.name.split("-")).upper()[:4])
        body = "\n" + read_body_file(a.body_file) + "\n\n" + rules
    path.write_text("---\n" + "\n".join(front) + "\n---\n" + body)
    print(f"agent  {path.relative_to(ROOT)} updated ({kind})")


def retire(a) -> None:
    path = AGENTS / f"{a.name}.md"
    if not path.exists():
        fail(f"no agent named {a.name}")
    if a.name in {"manager", "tech-lead"}:
        fail(f"{a.name} is a core role and can't be retired")
    RETIRED.mkdir(parents=True, exist_ok=True)
    shutil.move(str(path), RETIRED / path.name)
    team_rows(remove=a.name)
    gate_hook(a.name, False)
    print(f"agent  {a.name} retired -> .claude/agents-retired/{path.name} (move it back to rehire)")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("name")
    ap.add_argument("--kind", choices=KINDS)
    ap.add_argument("--description")
    ap.add_argument("--skills")
    ap.add_argument("--effort", choices=["low", "medium", "high", "xhigh", "max"])
    ap.add_argument("--max-turns", type=int)
    ap.add_argument("--max-tokens", help="max_tokens_per_run for team.md (default by kind)")
    ap.add_argument("--body-file")
    ap.add_argument("--skill-file")
    ap.add_argument("--new-skill", action="store_true")
    mode = ap.add_mutually_exclusive_group()
    mode.add_argument("--update", action="store_true")
    mode.add_argument("--retire", action="store_true")
    a = ap.parse_args()

    if not re.fullmatch(r"[a-z][a-z0-9-]{1,40}", a.name) or "claude" in a.name or "anthropic" in a.name:
        fail("name must be kebab-case (a-z, 0-9, -), 2-41 chars, without 'claude'/'anthropic'")
    if a.retire:
        retire(a)
    elif a.update:
        update(a)
    else:
        install(a)
    print("next   start a new Claude Code session so the change takes effect")
    return 0


if __name__ == "__main__":
    sys.exit(main())
