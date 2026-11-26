#!/usr/bin/env python3
"""Install (or upgrade) this agent kit into a repository.

  python3 install.py /path/to/your/repo      (or ./install.sh /path/to/your/repo)

Safe by design:
- never overwrites your own files: CLAUDE.md, .gitignore and justfile are merged; settings.json is
  merged key by key; files you already have (Dockerfile, workflows, CHANGELOG.md, ...) are kept and
  the kit's version is saved next to them as <name>.kit for you or the cicd agent to merge
- keeps your team configuration (.agents/team.md, project.md, decisions.md, board) on upgrades
- backs up any kit file it replaces into .agents/backup-<timestamp>/
- creates .env from .env.example only if .env doesn't exist (and never reads it)
"""
import datetime as dt
import filecmp
import json
import re
import pathlib
import shutil
import subprocess
import sys

KIT = pathlib.Path(__file__).resolve().parent
SKIP = {"install.py", "install.sh", ".git", "__pycache__"}
MARK = "agent-kit"
TITLE = json.loads((KIT / ".agents" / "stack.json").read_text()).get("title", "Agent") if (KIT / ".agents" / "stack.json").exists() else "Agent"

# Kit-owned: replaced on upgrade (with backup if changed locally).
KIT_OWNED = (".claude/agents/", ".claude/skills/", ".claude/hooks/", ".claude/rules/", ".claude/kit/", "AGENT-SETTINGS.md", "AGENT-GUIDE.md",
             ".agents/templates/", "AGENT-KIT.md")
# Team state: created once, never overwritten.
KEEP_IF_EXISTS = {".agents/team.md", ".agents/stack.json", ".agents/project.md", ".agents/owner-rules.md", ".agents/decisions.md", ".agents/board.md",
                  ".agents/codemap.md", ".env.example", "version.txt", ".release-please-manifest.json", "CHANGELOG.md"}

report = {"added": [], "updated": [], "merged": [], "kept": [], "side": []}


def split_row(line: str) -> list:
    """Cells of one markdown table row, split only on unescaped pipes (`a \\| b` stays one cell). A copy of
    kit.py split_row: keep them identical (kit-test checks)."""
    s = line.strip()
    if not s.startswith("|"):
        return []
    cells, cur, i = [], "", 1
    while i < len(s):
        if s[i] == "\\" and i + 1 < len(s):
            cur, i = cur + s[i:i + 2], i + 2
            continue
        if s[i] == "|":
            cells.append(cur)
            cur = ""
        else:
            cur += s[i]
        i += 1
    if cur.strip():  # a row without its closing pipe
        cells.append(cur)
    return cells


def rel(p: pathlib.Path) -> str:
    return p.relative_to(KIT).as_posix()


def ask(question: str) -> bool:
    if not sys.stdin.isatty():
        return True
    return input(f"{question} [Y/n] ").strip().lower() in ("", "y", "yes")


def merge_lines(dst: pathlib.Path, src: pathlib.Path) -> None:
    have = set(dst.read_text().splitlines())
    if f"# --- rust-{MARK} ---" in have:
        have.add(f"# --- {MARK} ---")
    new = [l for l in src.read_text().splitlines() if l and l not in have]
    if new:
        with dst.open("a") as f:
            f.write(f"\n# --- {MARK} ---\n" + "\n".join(new) + "\n")
        report["merged"].append(f"{dst.name} (+{len(new)} lines)")


KIT_GUIDE_SIGNATURE = "This repo is built by an AI team of Claude Code sub-agents"


def strip_unmarked_kit_copy(text: str) -> tuple:
    """Older installs wrote the kit guide without markers; a later upgrade then appended a second copy.
    Returns (text without that unmarked kit copy, removed part)."""
    if not text.startswith("# Agent guide") or KIT_GUIDE_SIGNATURE not in text[:400]:
        return text, ""
    cut = text.find(f"<!-- {MARK}:start -->")
    if cut < 0:
        return "", text
    return text[cut:], text[:cut]


def merge_claude_md(dst: pathlib.Path, src: pathlib.Path, backup: pathlib.Path = None) -> None:
    text, kit = dst.read_text(), src.read_text()
    text = text.replace("<!-- rust-agent-kit:start -->", f"<!-- {MARK}:start -->").replace("<!-- rust-agent-kit:end -->", f"<!-- {MARK}:end -->")
    start, end = f"<!-- {MARK}:start -->", f"<!-- {MARK}:end -->"
    block = f"{start}\n{kit.strip()}\n{end}\n"
    original = text
    text, removed = strip_unmarked_kit_copy(text)
    if removed and backup is not None:
        b = backup / "CLAUDE.md"
        b.parent.mkdir(parents=True, exist_ok=True)
        b.write_text(original)
        report["side"].append(f"CLAUDE.md: removed an old copy of the kit guide that had no markers (it was duplicated). "
                              f"If you had added your own notes inside it, copy them back from {b.relative_to(dst.parent)} - "
                              f"outside the <!-- {MARK} --> markers they survive future upgrades")
    if start in text and end in text:
        before, rest = text.split(start, 1)
        after = rest.split(end, 1)[1]
        new = before + block + after.lstrip("\n")
    elif not text.strip():
        new = block
    else:
        new = text.rstrip() + "\n\n" + block
    text = original
    if new != text:
        dst.write_text(new)
        report["merged"].append("CLAUDE.md (kit section)")


def merge_justfile(target: pathlib.Path, src: pathlib.Path) -> None:
    jf, aj = target / "justfile", target / "agents.just"
    text = jf.read_text()
    if "import 'agents.just'" in text:  # merged before: only refresh the kit recipes
        if not aj.exists() or not filecmp.cmp(src, aj, shallow=False):
            shutil.copyfile(src, aj)
            report["updated"].append("agents.just")
        return
    if text.startswith("# Task runner used by humans and agents"):  # the kit's own justfile
        shutil.copyfile(src, jf)
        report["updated"].append("justfile")
        return
    shutil.copyfile(src, aj)
    jf.write_text(f"# {MARK}: agent recipes\nimport 'agents.just'\n\n" + text)
    report["merged"].append("justfile (imports agents.just; rename any duplicate recipe names in yours)")


def merge_settings(dst: pathlib.Path, src: pathlib.Path) -> None:
    before = dst.read_text()
    mine, kit = json.loads(before), json.loads(src.read_text())
    if mine.get("agent") and mine["agent"] != "manager":
        report["side"].append(f".claude/settings.json: your \"agent\" was {mine['agent']!r}, now \"manager\"")
    mine["agent"] = "manager"
    mine.setdefault("env", {}).update(kit.get("env", {}))
    perms = mine.setdefault("permissions", {})
    for key in ("allow", "deny", "ask"):  # "ask" = the owner approves (promote, settings, rule files, kit scripts)
        cur = perms.setdefault(key, [])
        cur += [r for r in kit.get("permissions", {}).get(key, []) if r not in cur]
    hooks = mine.setdefault("hooks", {})
    for event, entries in kit.get("hooks", {}).items():
        cur = hooks.setdefault(event, [])
        existing = {h.get("command") for entry in cur for h in entry.get("hooks", [])}
        for e in entries:
            missing = [h for h in e.get("hooks", []) if h.get("command") not in existing]
            if missing:
                cur.append({**e, "hooks": missing})
                existing.update(h.get("command") for h in missing)
    for k, v in kit.items():  # new top-level kit keys (e.g. includeGitInstructions); your own values stay
        mine.setdefault(k, v)
    out = json.dumps(mine, indent=2) + "\n"
    if out != before:
        dst.write_text(out)
        report["merged"].append(".claude/settings.json")



LEGACY_SCRIPTS = ["scripts/gate.sh", "scripts/test-check.sh", "scripts/codemap.sh", "scripts/budget.py",
                  "scripts/agents_sync.py", "scripts/new_agent.py", "scripts/doctor.py"]


CAP_DEFAULTS = {"manager": "-", "tech-lead": "400,000", "architect": "300,000", "tester": "600,000", "code-reviewer": "150,000",
                "security-reviewer": "150,000", "cicd": "600,000", "build-runner": "100,000"}


def add_missing_setting_rows(t: str, kit_text: str) -> str:
    """For every `| setting | value |` / `| checkpoint | required |` table in the kit's team.md, add rows the user's
    team.md lacks (new settings), at the end of the same table. Existing rows and values are never changed."""
    def tables(text):
        out = {}
        for m in re.finditer(r"^(## [^\n]+)\n(.*?)(?=^## |\Z)", text, re.S | re.M):
            rows = re.findall(r"^\|\s*([^|]+?)\s*\|\s*([^|]*?)\s*\|\s*$", m.group(2), re.M)
            out[m.group(1).split("(")[0].strip()] = [(k, v) for k, v in rows if k not in ("setting", "checkpoint") and not set(k) <= set("-: ")]
        return out
    mine, kit = tables(t), tables(kit_text)
    for heading, rows in kit.items():
        if heading not in mine:
            continue
        have = {k for k, _ in mine[heading]}
        missing = [(k, v) for k, v in rows if k not in have]
        if not missing:
            continue
        m = re.search(rf"^{re.escape(heading)}[^\n]*\n(.*?)(?=^## |\Z)", t, re.S | re.M)
        last = list(re.finditer(r"^\|[^\n]*\|[ \t]*$\n?", m.group(0), re.M))  # [ \t], not \s: never swallow the blank line
        if not last:
            continue
        cut = m.start() + last[-1].end()
        add = "".join(f"| {k} | {v} |\n" for k, v in missing)
        t = t[:cut] + ("" if t[cut - 1] == "\n" else "\n") + add + t[cut:]
    return t


def reviewers_without_memory(t: str) -> str:
    """Reviewers now start without CLAUDE.md or memory (smaller cold start). Switch rows still at the old default
    `project` to `none` for agents whose new kit file has omitClaudeMd; any other value the user chose is kept."""
    for f in sorted((KIT / ".claude" / "agents").glob("*.md")):
        if "\nomitClaudeMd: true" not in f.read_text():
            continue
        t = re.sub(rf"^(\|\s*{re.escape(f.stem)}\s*\|[^|\n]*\|[^|\n]*\|\s*)project(\s*\|)", r"\1none\2", t, count=1, flags=re.M)
    return t


DOC_START = re.compile(r"^(- )?`([a-z_]+)`:")


def setting_docs(text: str) -> dict:
    """{setting: (start, end, paragraph)} for the kit's description paragraphs ('`name`: ...' or '- `name`: ...')."""
    lines, out, i = text.split("\n"), {}, 0
    pos = [0]
    for l in lines:
        pos.append(pos[-1] + len(l) + 1)
    while i < len(lines):
        m = DOC_START.match(lines[i])
        if not m:
            i += 1
            continue
        j = i + 1
        while j < len(lines) and lines[j].strip() and not DOC_START.match(lines[j]) \
                and not lines[j].startswith(("|", "#")) and (not m.group(1) or lines[j].startswith("  ")):
            j += 1
        out.setdefault(m.group(2), (pos[i], pos[j], "\n".join(lines[i:j])))
        i = j
    return out


def refresh_setting_docs(t: str, kit_text: str) -> tuple:
    """Replace the kit's own description of each setting with the current kit's wording (rows and values are never
    touched). Only paragraphs that start with the setting's name are replaced, so your own notes stay."""
    kit_docs, refreshed = setting_docs(kit_text), []
    for name, (_, _, new) in kit_docs.items():
        mine = setting_docs(t).get(name)
        if mine and mine[2].strip() != new.strip():
            t = t[:mine[0]] + new + "\n" + t[mine[1]:]
            refreshed.append(name)
    return t, refreshed


def add_missing_agent_rows(t: str, kit_text: str, target: pathlib.Path) -> str:
    """Agents new in this kit version (e.g. prototyper) get their rows in each agent table with the same columns."""
    def agent_tables(text):
        out = []
        for m in re.finditer(r"^(\| agent \|[^\n]*\|)\n(\|[-| ]+\|)\n((?:\|[^\n]*\|\n?)*)", text, re.M):
            rows = {split_row(r)[0].strip(): r for r in m.group(3).splitlines() if r.strip()}
            out.append((m.group(1).strip(), m, rows))
        return out
    kit_tables = {h: rows for h, _, rows in agent_tables(kit_text)}
    for header, m, rows in reversed(agent_tables(t)):
        new = [r for name, r in kit_tables.get(header, {}).items()
               if name not in rows and (KIT / ".claude" / "agents" / f"{name}.md").exists()]
        if new:
            end = m.end(3)
            add = ("" if t[end - 1:end] == "\n" else "\n") + "\n".join(new) + "\n"
            t = t[:end] + add + t[end:]
    return t


def upgrade_agent_tables(t: str, kit_text: str) -> str:
    """Add max_tokens_per_run, the advanced agent table and the Session section to an older team.md."""
    lines = t.splitlines()
    agents, i = [], 0
    while i < len(lines):
        cells = [c.strip() for c in split_row(lines[i])]
        if cells and cells[0].lower() == "agent" and "max_tokens_per_run" not in [c.lower() for c in cells] and "model" in [c.lower() for c in cells]:
            lines[i] = lines[i].rstrip().rstrip("|").rstrip() + " | max_tokens_per_run |"
            lines[i + 1] = lines[i + 1].rstrip().rstrip("|") + "|---|"
            j = i + 2
            while j < len(lines) and lines[j].strip().startswith("|"):
                name = (split_row(lines[j]) or [""])[0].strip()
                agents.append(name)
                cap = CAP_DEFAULTS.get(name, "400,000" if name.startswith("dev-") else "200,000")
                lines[j] = lines[j].rstrip().rstrip("|").rstrip() + f" | {cap} |"
                j += 1
            i = j
            continue
        i += 1
    t = "\n".join(lines) + "\n"
    if "## Agents - advanced" not in t and kit_text and "## Agents - advanced" in kit_text:
        if not agents:
            in_table = False
            for l in t.splitlines():
                s = l.strip()
                if s.lower().startswith("| agent"):
                    in_table = True
                    continue
                if in_table and s.startswith("|---"):
                    continue
                if in_table and s.startswith("|"):
                    agents.append(split_row(s)[0].strip())
                elif in_table:
                    break
        sec = kit_text[kit_text.index("## Agents - advanced"):]
        sec = sec[:sec.find("\n## ", 4) + 1]
        head = [l for l in sec.splitlines() if l.startswith("| agent") or l.startswith("|---")]
        notes = [l for l in sec.splitlines() if l.startswith("- ") or l.startswith("  ")]
        rows = [f"| {a} | - | - | {'none' if a == 'build-runner' else 'project'} | - |" for a in agents]
        block = "## Agents - advanced\n\n" + "\n".join(head + rows) + "\n\n" + "\n".join(notes) + "\n\n"
        anchor = next((h for h in ("## Session", "## Parallelism", "## Review policy", "## Budget") if h in t), None)
        t = t.replace(anchor, block + anchor, 1) if anchor else t + "\n" + block
    if "## Session" not in t and kit_text and "## Session" in kit_text:
        sec = kit_text[kit_text.index("## Session"):]
        nxt = sec.find("\n## ", 4)
        sec = sec[:nxt + 1] if nxt > 0 else sec
        anchor = next((h for h in ("## Parallelism", "## Review policy", "## Budget") if h in t), None)
        t = t.replace(anchor, sec + "\n" + anchor, 1) if anchor else t + "\n" + sec
    return t


def _merge_value(mine, kit, base, path: str, kept: list):
    """Three-way merge of one stack.json value. base = the kit version installed last time (None if unknown).
    Unchanged by you -> the new kit value; changed by you -> yours (named in `kept`); new kit keys/items -> added."""
    if mine == kit:
        return mine
    if base is not None and mine == base:
        return kit
    if isinstance(mine, dict) and isinstance(kit, dict):
        out = dict(mine)
        for k, v in kit.items():
            if k not in mine:
                out[k] = v
            else:
                out[k] = _merge_value(mine[k], v, (base or {}).get(k) if isinstance(base, dict) else None, f"{path}.{k}", kept)
        return out
    named = lambda xs: isinstance(xs, list) and xs and all(isinstance(x, dict) and "name" in x for x in xs)
    if named(mine) and named(kit):
        bmap = {x["name"]: x for x in base} if named(base) else {}
        kmap = {x["name"]: x for x in kit}
        out = [_merge_value(x, kmap[x["name"]], bmap.get(x["name"]), f"{path}[{x['name']}]", kept) if x["name"] in kmap else x
               for x in mine]
        have = {x["name"] for x in mine}
        out += [x for x in kit if x["name"] not in have and x["name"] not in bmap]  # new in the kit (not ones you removed)
        return out
    kept.append(path)
    return mine


def merge_stack_json(target: pathlib.Path, backup: pathlib.Path) -> None:
    """.agents/stack.json carries the pack's data (gate commands, review routing, never-break rules, developer paths).
    Upgrades add what the kit gained and keep what you changed. The installed kit version is remembered in
    .claude/kit/stack.kit.json so the next upgrade can tell your edits from old kit values."""
    src, dst = KIT / ".agents" / "stack.json", target / ".agents" / "stack.json"
    base_f = target / ".claude" / "kit" / "stack.kit.json"
    if not src.exists():
        return
    kit = json.loads(src.read_text())
    if dst.exists():
        mine = json.loads(dst.read_text())
        base = json.loads(base_f.read_text()) if base_f.exists() else None
        kept = []
        merged = _merge_value(mine, kit, base, "stack", kept)
        if merged != mine:
            b = backup / ".agents" / "stack.json"
            b.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(dst, b)
            dst.write_text(json.dumps(merged, indent=2, ensure_ascii=False) + "\n")
            report["merged"].append(".agents/stack.json (new kit keys added"
                                    + (f"; your values kept: {', '.join(k.replace('stack.', '') for k in kept[:6])}" if kept else "") + ")")
    base_f.parent.mkdir(parents=True, exist_ok=True)
    base_f.write_text(json.dumps(kit, indent=2, ensure_ascii=False) + "\n")


def migrate_legacy(target: pathlib.Path, backup: pathlib.Path) -> None:
    """Upgrade installs of the first Rust kit layout (scripts/ at the repo root)."""
    moved = []
    for r in LEGACY_SCRIPTS:
        f = target / r
        if f.exists():
            b = backup / r
            b.parent.mkdir(parents=True, exist_ok=True)
            shutil.move(str(f), b)
            moved.append(r)
    sd = target / "scripts"
    if moved and sd.is_dir() and not any(sd.iterdir()):
        sd.rmdir()
    st = target / ".claude" / "settings.json"
    if st.exists():
        s = json.loads(st.read_text())
        changed = False
        for event, entries in s.get("hooks", {}).items():
            for e in entries:
                keep = [h for h in e.get("hooks", []) if not any(x in h.get("command", "") for x in ("/scripts/codemap.sh", "/scripts/doctor.py", "/scripts/gate.sh"))]
                if len(keep) != len(e.get("hooks", [])):
                    e["hooks"], changed = keep, True
            s["hooks"][event] = [e for e in entries if e.get("hooks")]
        if changed:
            st.write_text(json.dumps(s, indent=2) + "\n")
            moved.append(".claude/settings.json (old hook commands)")
    team = target / ".agents" / "team.md"
    if team.exists() and "## Branches" not in team.read_text():
        t = team.read_text()
        section = """## Branches

Feature PRs target the integration branch; the tech lead merges them there once `just merge-check <pr>`
passes. The release branch only moves through `just promote <pr>`, which Claude Code asks you to approve.

| setting | value |
|---|---|
| integration_branch | develop |
| release_branch | main |
| require_ci_checks | yes |

"""
        t = t.replace("## Approvals", section + "## Approvals", 1) if "## Approvals" in t else t + "\n" + section
        t = t.replace("| merge to main | yes |", "| merge feature PRs into the integration branch | no (tech lead merges when merge-check passes) |")
        t = t.replace("| release (merging the release PR -> tag) | yes |", "| promote to the release branch / merge the release PR | always (Claude Code asks you) |")
        team.write_text(t)
        moved.append(".agents/team.md (added Branches section)")
    if team.exists():
        t = team.read_text()
        before = t
        # manager: summarise at 160k and keep its cache for an hour (only when still at the old defaults)
        t = re.sub(r"^\|\s*manager\s*\|\s*-\s*\|\s*-\s*\|", "| manager | 160,000 | 1h |", t, flags=re.M)
        # reviewers are no longer kept warm across PRs (measured: ~150k per reused review vs ~80k fresh)
        m = re.search(r"^(\|\s*reuse_roles\s*\|\s*)([^|]*?)(\s*\|)$", t, re.M)
        if m:
            reviewers = {"code-reviewer", "security-reviewer", "combined-reviewer"}
            try:
                st = json.loads((target / ".agents" / "stack.json").read_text())
                reviewers |= {ea["name"] for ea in st.get("extra_agents", []) if ea.get("kind") == "reviewer"}
            except (OSError, ValueError):
                pass
            have = [x.strip() for x in m.group(2).split(",") if x.strip()]
            keep = [x for x in have if x not in reviewers]
            if keep != have:
                t = t[:m.start()] + m.group(1) + (", ".join(keep) or "-") + m.group(3) + t[m.end():]
        t = re.sub(r"^\|\s*reuse_max_context\s*\|\s*100,000\s*\|", "| reuse_max_context | 120,000 |", t, flags=re.M)
        t = re.sub(r"^\|\s*envelope_max_lines\s*\|\s*150\s*\|", "| envelope_max_lines | 400 |", t, flags=re.M)
        # the old tester default ran a tester on every PR; the new default runs the full pass once per feature
        t = re.sub(r"^\|\s*tester_review\s*\|\s*behaviour-change\s*\|", "| tester_review | feature |", t, flags=re.M)
        # old default turn limits -> new leaner defaults (only rows still at the old default)
        for name, old_n, new_n in [("tech-lead", 60, 40), ("architect", 40, 30), ("tester", 60, 40), ("code-reviewer", 40, 15),
                                   ("security-reviewer", 40, 15), ("cicd", 60, 40), ("build-runner", 15, 10)]:
            t = re.sub(rf"^(\|\s*{name}\s*\|[^|]*\|[^|]*\|)\s*{old_n}\s*\|", rf"\g<1> {new_n} |", t, flags=re.M)
        kit_team = KIT / ".agents" / "team.md"
        if kit_team.exists():
            kt = kit_team.read_text()
            for heading in ("## Review policy", "## Context (token use)"):
                if heading not in t and heading in kt:
                    sec = kt[kt.index(heading):]
                    nxt = sec.find("\n## ", 4)
                    sec = sec[:nxt + 1] if nxt > 0 else sec
                    t = t.replace("## Budget", sec + "\n## Budget", 1) if "## Budget" in t else t + "\n" + sec
        if kit_team.exists():
            t = add_missing_setting_rows(t, kit_team.read_text())
            t, refreshed = refresh_setting_docs(t, kit_team.read_text())
            if refreshed:
                moved.append(f".agents/team.md (descriptions refreshed: {', '.join(refreshed[:8])})")
        t = upgrade_agent_tables(t, kit_team.read_text() if kit_team.exists() else "")
        if kit_team.exists():
            t = add_missing_agent_rows(t, kit_team.read_text(), target)
            t = reviewers_without_memory(t)
        if t != before:
            team.write_text(t)
            moved.append(".agents/team.md (new settings added; your values kept)")
    board = target / ".agents" / "board.md"
    if board.exists() and "## Owner actions" not in board.read_text():
        b = board.read_text()
        sections = ("## Resume\n\n<!-- Where things stand and the next step (<= 5 lines); `just milestone \"...\"` rewrites it. -->\n\n"
                    "## Owner actions\n\n<!-- Things only the owner can do. One line each: - [ ] <what> (<where>) - since <date> -->\n\n"
                    "## Tasks\n\n")
        i = b.find("\n| id |")
        board.write_text(b[:i + 1] + sections + b[i + 1:] if i >= 0 else b + "\n" + sections)
        moved.append(".agents/board.md (added Resume and Owner actions sections; your rows kept)")
    if moved:
        report["updated"] += [f"migrated {m}" for m in moved]

def kit_identity(path: pathlib.Path) -> dict:
    """{id, title, built} of a kit folder or of the kit installed in a repo (older installs: from stack files)."""
    for f in (path / ".claude" / "kit" / "KIT_VERSION.json",):
        try:
            return json.loads(f.read_text())
        except (OSError, ValueError):
            pass
    for f in (path / ".claude" / "kit" / "stack.kit.json", path / ".agents" / "stack.json"):
        try:
            d = json.loads(f.read_text())
            return {"id": d.get("id", ""), "title": d.get("title", ""), "built": ""}
        except (OSError, ValueError):
            pass
    return {}


def ask_no(question: str) -> bool:
    """A question whose safe answer is No; without a terminal it is No unless --force was given."""
    if "--force" in sys.argv:
        return True
    if not sys.stdin.isatty():
        return False
    return input(f"{question} [y/N] ").strip().lower() in ("y", "yes")


def check_kit_match(target: pathlib.Path) -> None:
    new, have = kit_identity(KIT), kit_identity(target)
    if not have.get("id") or not new.get("id"):
        return
    if have["id"] != new["id"]:
        print(f"STOP: this repository uses the '{have.get('title') or have['id']}' kit, and you are installing the "
              f"'{new.get('title') or new['id']}' kit from {KIT}.")
        print("Installing a different kit replaces its agents, skills and recipes. Usually this is the wrong folder.")
        if not ask_no("Install the different kit anyway?"):
            sys.exit("Nothing changed. Run the installer from the right kit folder (or pass --force).")
    elif have.get("built") and new.get("built") and new["built"] < have["built"]:
        print(f"STOP: the kit you are installing (built {new['built']}) is OLDER than the one installed ({have['built']}).")
        if not ask_no("Install the older kit anyway?"):
            sys.exit("Nothing changed. Unzip the newest kit and run its installer (or pass --force).")


def just_recipes_in(text: str) -> set:
    return {m.group(1) for m in re.finditer(r"^@?([A-Za-z_][A-Za-z0-9_-]*)(?:\s[^:\n]*)?:(?!=)", text, re.M)
            if m.group(1) not in ("set", "export", "import", "mod", "alias")}


def check_recipes(target: pathlib.Path) -> list:
    kit_r = just_recipes_in((KIT / "justfile").read_text()) if (KIT / "justfile").exists() else set()
    have = ""
    for f in (target / "justfile", target / "agents.just"):
        if f.exists():
            have += f.read_text()
    return sorted(kit_r - just_recipes_in(have))


def main() -> int:
    if KIT.name == "installer" and KIT.parent.name == "kit":
        sys.exit("This is the copy kept for `just export-kit`; run install.py from a kit folder or an exported kit zip.")
    target = pathlib.Path(next((a for a in sys.argv[1:] if not a.startswith("--")), ".")).expanduser().resolve()
    if target == KIT:
        sys.exit("Run this against your repository: python3 install.py /path/to/your/repo")
    if not target.exists():
        sys.exit(f"{target} does not exist")
    print(f"Installing the {TITLE} agent kit into {target}\n")
    if not (target / ".git").exists():
        if ask(f"{target} is not a git repository. Initialise one?"):
            subprocess.run(["git", "init", "-q", "-b", "main"], cwd=target, check=True)
            report["added"].append(".git (new repository)")
        else:
            sys.exit("The kit needs a git repository (agents use branches and worktrees).")

    check_kit_match(target)
    backup = target / ".agents" / f"backup-{dt.datetime.now():%Y%m%d-%H%M%S}"
    migrate_legacy(target, backup)
    merge_stack_json(target, backup)
    for src in sorted(KIT.rglob("*")):
        if src.is_dir() or any(part in SKIP for part in src.relative_to(KIT).parts):
            continue
        r = rel(src)
        dst = target / r
        dst.parent.mkdir(parents=True, exist_ok=True)

        if not dst.exists():
            if r == "justfile":
                shutil.copyfile(src, dst)
            elif r == "CLAUDE.md":  # wrapped in markers so your own notes outside them survive upgrades
                dst.write_text(f"<!-- {MARK}:start -->\n{src.read_text().strip()}\n<!-- {MARK}:end -->\n")
            else:
                shutil.copy2(src, dst)
            report["added"].append(r)
            continue
        if filecmp.cmp(src, dst, shallow=False):
            continue
        if r == ".gitignore":
            merge_lines(dst, src)
        elif r == "CLAUDE.md":
            merge_claude_md(dst, src, backup)
        elif r == "justfile":
            merge_justfile(target, src)
        elif r == ".claude/settings.json":
            merge_settings(dst, src)
        elif r == ".agents/stack.json":
            pass  # merged above (merge_stack_json)
        elif r in KEEP_IF_EXISTS or r.startswith((".agents/plans/", ".agents/reviews/", ".agents/reports/", ".agents/bugs/", "docs/features/README", "docs/bugs/README")):
            report["kept"].append(r)
        elif r.startswith(KIT_OWNED):
            b = backup / r
            b.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(dst, b)
            shutil.copy2(src, dst)
            report["updated"].append(r)
        else:  # your own file (Dockerfile, workflows, deny.toml, ...): keep it, put ours beside it
            side = dst.with_name(dst.name + ".kit")
            shutil.copy2(src, side)
            report["side"].append(f"{r} kept; kit version saved as {side.relative_to(target)}")

    # the installer itself, kept beside the engine so `just export-kit` sends a kit someone can upgrade from
    for name in ("install.py", "install.sh"):
        src, dst = KIT / name, target / ".claude" / "kit" / "installer" / name
        if src.is_file() and not (dst.is_file() and filecmp.cmp(src, dst, shallow=False)):
            report["updated" if dst.exists() else "added"].append(f".claude/kit/installer/{name}")
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(src, dst)

    doctor = target / ".claude" / "kit" / "doctor.py"
    print_section("Added", report["added"], limit=12)
    print_section("Updated (backups in " + str(backup.relative_to(target)) + ")" if backup.exists() else "Updated",
                  report["updated"], limit=12)
    print_section("Merged", report["merged"])
    print_section("Kept your version", report["kept"], limit=12)
    print_section("Needs a look", report["side"])
    missing = check_recipes(target)
    if missing:
        print("\n" + "!" * 70)
        print(f"MISSING KIT RECIPES in your justfile: {', '.join(missing[:12])}{' ...' if len(missing) > 12 else ''}")
        print("Agents will call these and fail. Your justfile was probably edited; compare it with "
              f"{KIT / 'justfile'} and add them back.")
        print("!" * 70)
    print()
    subprocess.run([sys.executable, str(doctor), "--fix"], cwd=target)
    print(f"""
Next:
  1. Fill in {target / '.env'} yourself (agents never read it).
  2. Commit the kit files on a branch (Claude Code sessions that run in a worktree or in the
     cloud only see committed files), e.g.  git switch -c chore/agent-kit && git add -A && git commit -m "chore: add agent kit"
  3. Start a NEW Claude Code session with {target} as its folder and trust the workspace.
     Check it worked: the terminal header shows @manager, or ask "who are you?" -> "the manager".
     `just agents` lists the whole team.
  4. Say: "Manager, fill in the project profile (.agents/project.md) with me"
     The manager interviews you about data stores, sources, environments and branches.
     Not the manager answering? See AGENT-KIT.md -> "If the manager doesn't answer".""")
    if report["side"]:
        print("  5. Ask the manager to have the cicd agent merge the *.kit files with yours.")
    return 0


def print_section(title: str, items: list, limit: int = 50) -> None:
    if not items:
        return
    print(f"{title} ({len(items)}):")
    for i in items[:limit]:
        print(f"  {i}")
    if len(items) > limit:
        print(f"  ... and {len(items) - limit} more")


if __name__ == "__main__":
    sys.exit(main())
