#!/usr/bin/env python3
"""Apply the agent settings in .agents/team.md to .claude/agents/*.md and .claude/settings.json.

Reads every table in team.md whose first column is `agent` (rows for the same agent are merged) and
the `Session` table. Each column maps to one Claude Code setting; `-` removes it (Claude Code default).

  agents_sync.py            apply
  agents_sync.py --check    report differences without writing (exit 1 if out of sync or invalid)
  agents_sync.py --list     print every supported setting with its allowed values
"""
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
TEAM = ROOT / ".agents" / "team.md"
AGENTS = ROOT / ".claude" / "agents"
SETTINGS = ROOT / ".claude" / "settings.json"


def is_int(lo=1, hi=None):
    def check(v):
        if not v.replace(",", "").replace("_", "").isdigit():
            return "must be a whole number"
        n = int(v.replace(",", "").replace("_", ""))
        if n < lo or (hi and n > hi):
            return f"must be between {lo:,} and {hi:,}" if hi else f"must be at least {lo:,}"
        return None
    return check


def one_of(*allowed):
    def check(v):
        return None if v in allowed else f"must be one of: {', '.join(allowed)}"
    return check


def any_text(v):
    return None if v and " " not in v else "must be a single word with no spaces"


# column in team.md -> (frontmatter key, validator, description, normaliser)
AGENT_PARAMS = {
    "model": ("model", any_text, "which Claude model: sonnet, opus, haiku, fable, a full model id, or inherit", None),
    "effort": ("effort", one_of("low", "medium", "high", "xhigh", "max"), "how hard the agent thinks per step", None),
    "max_turns": ("maxTurns", is_int(1, 10000), "most steps per task; the agent is stopped after this", None),
    "context_window": ("autoCompactWindow", is_int(100_000, 1_000_000),
                       "context size (tokens) at which the conversation is auto-summarised; smaller = cheaper turns",
                       lambda v: v.replace(",", "").replace("_", "")),
    "memory": ("memory", one_of("project", "user", "local", "none"),
               "where the agent keeps notes between runs; none = no memory", None),
    "permission_mode": ("permissionMode", one_of("default", "manual", "acceptEdits", "auto", "dontAsk", "plan"),
                        "how tool approvals work for this agent (bypassPermissions is not allowed by the kit)", None),
}
# nested frontmatter: column -> (parent map, key, validator, description)
NESTED_PARAMS = {
    "cache_ttl": ("experimental", "cacheTtl", one_of("5m", "1h"),
                  "how long the agent's prompt cache lives; 1h makes repeated launches cheaper (ignored when on usage credits)"),
}
# kit-only columns (not written to agent files; read by other kit scripts)
KIT_COLUMNS = {
    "max_tokens_per_run": (is_int(1_000), "token cap per launch, enforced by the manager's budget check (warns/stops between launches)"),
}
# Session table setting -> (settings.json env var, validator, description)
SESSION_PARAMS = {
    "max_output_tokens": ("CLAUDE_CODE_MAX_OUTPUT_TOKENS", is_int(1_000, 128_000),
                          "longest single reply any agent may write, in tokens"),
    "max_thinking_tokens": ("MAX_THINKING_TOKENS", is_int(1_024, 128_000),
                            "thinking budget per reply when extended thinking is on"),
    "subagent_model": ("CLAUDE_CODE_SUBAGENT_MODEL", any_text,
                       "fallback model for agents whose model column is - (empty = main model)"),
    "max_parallel_agents": ("CLAUDE_CODE_MAX_CONCURRENT_SUBAGENTS", is_int(1, 200),
                            "how many agents may run at the same time (Claude Code default 20)"),
    "compact_at_percent": ("CLAUDE_AUTOCOMPACT_PCT_OVERRIDE", is_int(1, 100),
                           "summarise the conversation earlier: % of the context window (can only lower the default)"),
    "context_window": ("CLAUDE_CODE_AUTO_COMPACT_WINDOW", is_int(100_000, 1_000_000),
                       "context window for every agent; overrides each agent's context_window column"),
    "file_read_max_tokens": ("CLAUDE_CODE_FILE_READ_MAX_OUTPUT_TOKENS", is_int(1_000, 200_000),
                             "largest chunk of a file one read may return, in tokens"),
    "command_output_max_chars": ("BASH_MAX_OUTPUT_LENGTH", is_int(1_000, 150_000),
                                 "largest command output an agent sees, in characters (Claude Code default 30,000)"),
}


def tables(text: str, first: str) -> list:
    """All markdown tables whose header's first cell is `first` -> list of (header, rows)."""
    out, header, rows = [], None, []
    for line in text.splitlines() + [""]:
        s = line.strip()
        if not s.startswith("|"):
            if header:
                out.append((header, rows))
            header, rows = None, []
            continue
        cells = [c.strip().strip("`") for c in s.strip("|").split("|")]
        if header is None:
            if cells and cells[0].lower() == first:
                header = [c.lower() for c in cells]
            continue
        if set("".join(cells)) <= set("-: "):
            continue
        rows.append(dict(zip(header, cells)))
    return out


def agent_rows(text: str) -> dict:
    merged = {}
    for _, rows in tables(text, "agent"):
        for r in rows:
            merged.setdefault(r["agent"], {}).update({k: v for k, v in r.items() if k != "agent"})
    if not merged:
        sys.exit(f"No agent table found in {TEAM}")
    return merged


def session_rows(text: str) -> dict:
    m = re.search(r"^## Session.*?(?=^## |\Z)", text, re.S | re.M)
    if not m:
        return {}
    out = {}
    for _, rows in tables(m.group(0), "setting"):
        for r in rows:
            out[r["setting"]] = r.get("value", "-")
    return out


def set_key(front: list, key: str, value: str) -> list:
    pat = re.compile(rf"^{key}:")
    if value in ("", "-"):
        return [l for l in front if not pat.match(l)]
    if any(pat.match(l) for l in front):
        return [f"{key}: {value}" if pat.match(l) else l for l in front]
    anchors = [i for i, l in enumerate(front) if re.match(r"^(description|model|effort|maxTurns|autoCompactWindow):", l)]
    idx = anchors[-1] + 1 if anchors else len(front)
    return front[:idx] + [f"{key}: {value}"] + front[idx:]


def set_nested(front: list, parent: str, key: str, value: str) -> list:
    """Set or remove `parent:\n  key: value` in frontmatter lines, keeping other children of parent."""
    out, children, i, found = [], [], 0, False
    while i < len(front):
        if front[i] == f"{parent}:":
            found = True
            i += 1
            while i < len(front) and front[i].startswith("  ") and not front[i].startswith("  - "):
                if not front[i].strip().startswith(f"{key}:"):
                    children.append(front[i])
                i += 1
            continue
        out.append(front[i])
        i += 1
    if value not in ("", "-"):
        children.append(f"  {key}: {value}")
    if not children:
        return out
    idx = next((n for n, l in enumerate(out) if l.startswith(("tools:", "skills:"))), len(out))
    return out[:idx] + [f"{parent}:"] + children + out[idx:]


LEAN_AGENTS = ("tech-lead", "code-reviewer", "security-reviewer")


def preload_setting(text: str) -> bool:
    m = re.search(r"^\|\s*preload_all_stack_skills\s*\|\s*(\w+)\s*\|", text, re.M)
    return bool(m and m.group(1).lower() in ("yes", "true", "on"))


def stack_json() -> dict:
    sj = ROOT / ".agents" / "stack.json"
    return json.loads(sj.read_text()) if sj.exists() else {}


def stack_skills() -> list:
    s = stack_json()
    roles = s.get("skill_roles", {})
    return list(dict.fromkeys([roles.get(k) for k in ("core", "testing", "architecture", "security", "repo")] +
                              [d["skill"] for d in s.get("developers", [])]))


def apply_preload(front: list, want_all: bool, base: set, extra: list) -> list:
    out, skills, i = [], [], 0
    while i < len(front):
        if front[i].startswith("skills:"):
            i += 1
            while i < len(front) and front[i].startswith("  - "):
                skills.append(front[i][4:].strip())
                i += 1
            continue
        out.append(front[i])
        i += 1
    if want_all:
        skills = list(dict.fromkeys(skills + [x for x in extra if x]))
    else:
        skills = [x for x in skills if x in base or x not in extra]
    idx = next((n + 1 for n, l in enumerate(out) if l.startswith("tools:")), len(out))
    return out[:idx] + (["skills:"] + [f"  - {x}" for x in skills] if skills else []) + out[idx:]


# ---- pocket cards: agents preload only their process skill; handbooks are an index looked up with `just docs`

KEEP_PRELOADED = {"manager": {"delivery-process"}, "tech-lead": {"delivery-process"}, "code-reviewer": {"code-review"},
                  "security-reviewer": {"security-review"}, "cicd": {"ci-cd"}}
HB_START, HB_END = "<!-- kit:handbooks", "<!-- /kit:handbooks -->"


def preload_mode() -> str:
    return stack_json().get("preload_mode", "cards")


def _clean(h: str) -> str:
    h = re.sub(r"^#+\s*", "", h)
    h = re.sub(r"^\d+[a-z]?(\.\d+)*\.?\s*", "", h)
    return re.sub(r"\s+", " ", re.sub(r"\(.*?\)|`|\*", "", h)).strip(" :-")


def _heading_lines(text: str) -> list:
    out, fence = [], False
    for l in text.splitlines():
        if l.lstrip().startswith("```"):
            fence = not fence
        elif not fence:
            out.append(l)
    return out


def skill_topics(skill: str) -> tuple:
    d = ROOT / ".claude" / "skills" / skill
    main = [_clean(l) for l in _heading_lines((d / "SKILL.md").read_text(errors="replace")) if l.startswith("## ")] if (d / "SKILL.md").exists() else []
    deeper = []
    for f in sorted((d / "references").glob("*.md")) if (d / "references").exists() else []:
        lines = _heading_lines(f.read_text(errors="replace"))
        hs = [_clean(l) for l in lines if l.startswith("## ")] or [_clean(l) for l in lines if l.startswith("# ")][:1]
        deeper += hs
    seen = {x.lower() for x in main}
    deeper = [x for x in dict.fromkeys(deeper) if x.lower() not in seen]
    return [x for x in main if x][:14], [x for x in deeper if x][:14]


def never_break() -> list:
    s = stack_json()
    if s.get("never_break"):
        return list(s["never_break"])
    core = s.get("skill_roles", {}).get("core", "")
    f = ROOT / ".claude" / "skills" / core / "SKILL.md"
    if not f.exists():
        return []
    m = re.search(r"^## [^\n]*golden rules[^\n]*\n(.*?)(?=^## |\Z)", f.read_text(errors="replace"), re.S | re.M | re.I)
    return [x.strip().rstrip(".") + "." for x in re.findall(r"^\s*\d+\.\s+\*\*(.+?)\*\*", m.group(1), re.M)] if m else []


def handbook_block(owned: list, omit_claude_md: bool) -> str:
    lines = [f"{HB_START} {','.join(owned)} -->", "## Handbooks: look up the part you need",
             'Run `just docs "<topic>"` (or `"<handbook>: <topic>"`) to get one section; don\'t read whole handbooks.',
             "Before writing or judging code in an area, read that handbook's golden rules "
             '(`just docs "<handbook>: golden rules"`); a rule file may already have shown them when you opened a file there.']
    for s in owned:
        main, deeper = skill_topics(s)
        lines.append(f"- **{s}**: " + " · ".join(main) + ("; deeper: " + " · ".join(deeper) if deeper else ""))
    if omit_claude_md:
        nb = never_break()
        if nb:
            lines += ["", "Never break (project rules):"] + [f"- {x}" for x in nb]
    lines.append(HB_END)
    return "\n".join(lines)


def apply_cards(agent: str, front: list, body: str) -> tuple:
    skills, out, i = [], [], 0
    while i < len(front):
        if front[i].startswith("skills:"):
            i += 1
            while i < len(front) and front[i].startswith("  - "):
                skills.append(front[i][4:].strip())
                i += 1
            continue
        out.append(front[i])
        i += 1
    keep = KEEP_PRELOADED.get(agent, set()) | set(stack_json().get("keep_preloaded", {}).get(agent, []))
    m = re.search(re.escape(HB_START) + r" ([^\n]*?) -->", body)
    owned_prev = [x for x in (m.group(1).split(",") if m else []) if x]
    owned = [x for x in dict.fromkeys(owned_prev + [s for s in skills if s not in keep])
             if (ROOT / ".claude" / "skills" / x / "SKILL.md").exists() and x not in keep]
    kept = [s for s in skills if s in keep]
    # no Skill tool for sub-agents: its manual lists every installed skill on every start; `just docs` replaces it.
    # The manager (main session) keeps it, so the user's own skills stay usable.
    if agent != "manager" and agent not in stack_json().get("keep_skill_tool", []):
        out = [re.sub(r"^tools:(.*)$", lambda m: "tools:" + ",".join(x for x in m.group(1).split(",") if x.strip() != "Skill"), l)
               if l.startswith("tools:") else l for l in out]
    idx = next((n + 1 for n, l in enumerate(out) if l.startswith("tools:")), len(out))
    out = out[:idx] + (["skills:"] + [f"  - {x}" for x in kept] if kept else []) + out[idx:]
    body = re.sub(r"\n*" + re.escape(HB_START) + r".*?" + re.escape(HB_END) + r"\n*", "\n\n", body, flags=re.S).rstrip() + "\n"
    if owned:
        omit = any(l.strip() == "omitClaudeMd: true" for l in out)
        body = body.rstrip() + "\n\n" + handbook_block(owned, omit) + "\n"
    return out, body


def list_params() -> int:
    print("Per-agent columns (.agents/team.md, tables starting with 'agent'):")
    for col, (key, _, desc, _) in AGENT_PARAMS.items():
        print(f"  {col:22} -> {key:18} {desc}")
    for col, (parent, key, _, desc) in NESTED_PARAMS.items():
        print(f"  {col:22} -> {parent + '.' + key:18} {desc}")
    for col, (_, desc) in KIT_COLUMNS.items():
        print(f"  {col:22} -> (kit)              {desc}")
    print("\nSession settings (.agents/team.md -> Session table, applied to .claude/settings.json env):")
    for name, (env, _, desc) in SESSION_PARAMS.items():
        print(f"  {name:22} -> {env:30} {desc}")
    print("\nUse - for the Claude Code default.")
    print("Full guide with examples and recipes: AGENT-SETTINGS.md (repo root)")
    return 0


def main() -> int:
    if "--list" in sys.argv:
        return list_params()
    check = "--check" in sys.argv
    team_text = TEAM.read_text()
    table = agent_rows(team_text)
    want_all, extra = preload_setting(team_text), stack_skills()
    m_mode = re.search(r"^\|\s*work_mode\s*\|\s*`?([\w-]+)", team_text, re.M)
    in_place = bool(m_mode and m_mode.group(1).lower() == "in-place")
    changed, problems = [], []

    for agent, row in table.items():
        unknown = [c for c in row if c not in AGENT_PARAMS and c not in KIT_COLUMNS and c not in NESTED_PARAMS]
        for c in unknown:
            problems.append(f"{agent}: unknown column '{c}' (see: python3 .claude/kit/agents_sync.py --list)")
        for col, (validator, _) in KIT_COLUMNS.items():
            v = row.get(col, "-")
            if v not in ("", "-") and validator(v):
                problems.append(f"{agent}: {col} {validator(v)}")
        path = AGENTS / f"{agent}.md"
        if not path.exists():
            problems.append(f"{agent}: no file {path.relative_to(ROOT)}")
            continue
        text = path.read_text()
        m = re.match(r"^---\n(.*?)\n---\n", text, re.S)
        if not m:
            problems.append(f"{agent}: missing frontmatter")
            continue
        front = m.group(1).split("\n")
        new_front = front
        for col, (key, validator, _, norm) in AGENT_PARAMS.items():
            if col not in row:
                continue
            value = row[col] or "-"
            if value != "-":
                err = validator(value)
                if err:
                    problems.append(f"{agent}: {col} '{value}' {err}")
                    continue
                if norm:
                    value = norm(value)
            if col == "memory" and value == "none":
                value = "-"
            if col == "permission_mode" and agent == "manager" and value not in ("-", "default", "acceptEdits", "auto"):
                problems.append(f"{agent}: permission_mode '{value}' would stop the manager from asking you for approvals")
                continue
            current = next((l.split(":", 1)[1].strip() for l in new_front if l.startswith(f"{key}:")), "-")
            if current != value:
                new_front = set_key(new_front, key, value)
                changed.append(f"{agent}: {key} {current} -> {value}")
        for col, (parent, key, validator, _) in NESTED_PARAMS.items():
            if col not in row:
                continue
            value = row[col] or "-"
            if value != "-" and validator(value):
                problems.append(f"{agent}: {col} '{value}' {validator(value)}")
                continue
            before_nf = list(new_front)
            new_front = set_nested(new_front, parent, key, value)
            if new_front != before_nf:
                changed.append(f"{agent}: {parent}.{key} -> {value}")
        cards = preload_mode() == "cards"
        if agent in LEAN_AGENTS and extra and not cards:
            roles = stack_json().get("skill_roles", {})
            base = {"tech-lead": {roles.get("core"), roles.get("architecture")},
                    "code-reviewer": {roles.get("core")},
                    "security-reviewer": {roles.get("security")}}[agent]
            before = [l for l in new_front if l.startswith("  - ")]
            new_front = apply_preload(new_front, want_all, base, extra)
            after = [l for l in new_front if l.startswith("  - ")]
            if before != after:
                changed.append(f"{agent}: preloaded skills {len(before)} -> {len(after)}")
        # agent-attached guard hooks apply to this agent only (older files lack --for and also fired for other agents)
        scoped = [re.sub(r'(\.claude/hooks/(?:write-scope|dev-git-guard)\.sh)(?! --for)', rf'\1 --for {agent}', l) for l in new_front]
        if scoped != new_front:
            new_front = scoped
            changed.append(f"{agent}: guard hooks scoped to {agent}")
        # work_mode in-place: no worktree isolation (one developer at a time in the user's checkout)
        on, off = "isolation: worktree", "# isolation: worktree  (off while work_mode is in-place)"
        swap = (on, off) if in_place else (off, on)
        if swap[0] in new_front:
            new_front = [swap[1] if l == swap[0] else l for l in new_front]
            changed.append(f"{agent}: worktree isolation {'off' if in_place else 'on'}")
        body = text[m.end():]
        if cards:
            before_front, before_body = list(new_front), body
            new_front, body = apply_cards(agent, new_front, body)
            if new_front != before_front or body != before_body:
                changed.append(f"{agent}: pocket card (handbooks looked up, not preloaded)")
        if (new_front != front or body != text[m.end():]) and not check:
            path.write_text("---\n" + "\n".join(new_front) + "\n---\n" + body)

    session = session_rows(team_text)
    if session and SETTINGS.exists():
        s = json.loads(SETTINGS.read_text())
        env = s.setdefault("env", {})
        before = dict(env)
        for name, value in session.items():
            if name not in SESSION_PARAMS:
                problems.append(f"Session: unknown setting '{name}' (see --list)")
                continue
            var, validator, _ = SESSION_PARAMS[name]
            value = (value or "-") if name == "subagent_model" else (value or "-").replace(",", "").replace("_", "")
            if value == "-":
                env.pop(var, None)
                continue
            err = validator(value)
            if err:
                problems.append(f"Session: {name} '{value}' {err}")
                continue
            env[var] = value
        for var in set(before) | set(env):
            if before.get(var) != env.get(var):
                changed.append(f"session: {var} {before.get(var, '-')} -> {env.get(var, '-')}")
        if env != before and not check:
            SETTINGS.write_text(json.dumps(s, indent=2) + "\n")

    for p in problems:
        print("ERROR", p)
    for c in changed:
        print(("DIFF " if check else "SET  ") + c)
    if not changed and not problems:
        print("agents in sync with .agents/team.md")
    elif not check and changed:
        print("Done. Start a new Claude Code session so the changes take effect.")
    return 1 if problems or (check and changed) else 0


if __name__ == "__main__":
    sys.exit(main())
