#!/usr/bin/env python3
"""Stack-driven kit engine. Every stack-specific command and pattern comes from .agents/stack.json.

  kit.py gate                 fmt/lint/test steps with compact output (exit 0 = PASS)
  kit.py test                 only the test step
  kit.py check                gate + extra checks (docs tests, supply chain, ...)
  kit.py db-check             connection status of configured data stores (never prints secrets)
  kit.py test-check [base]    fail if production code changed without new tests (developer gate)
  kit.py codemap [out]        write .agents/codemap.md (modules, items, routes, migrations, config keys, in-flight)
  kit.py base                 print the integration branch feature PRs target (team.md -> Branches)
  kit.py new-branch <name>    create a work branch from the latest integration branch
  kit.py merge-check <pr> [--feature ID] | --branch <b> --feature ID
                              verify a PR may merge: target branch, title, CI checks, reviewer verdicts
  kit.py merge <pr>           tech lead: merge-check, then squash-merge into the integration branch
  kit.py promote <pr>         user-approved: merge a promotion/release PR into the release branch
  kit.py review-plan <pr> | --branch <b>   which reviews this PR needs (size/sensitivity policy in team.md)
  kit.py triage <feature> [--ref pr-N|branch] [--dry-run]   apply the default triage decisions; list only what
                              needs the tech lead's judgment (disputes, conflicts, unclear fixes)
  kit.py envelope <pr> | --branch <b> | --diff <base>..<head> [--feature ID] [--dry-run]
                              small PRs: review by one tool-free Claude call (all required reviews in one
                              "envelope") instead of launching reviewer agents; writes the normal findings files
  kit.py docs "<topic>" | --list [skill]   the library: print the one handbook section that matches
                              (e.g. docs "greeks", docs "bz-quant: tests"); --list shows every section
  kit.py start-report [agent] what an agent reads before it starts working, and how big each part is
  kit.py status               one compact status: checkout, work mode, task budget, board, open PRs and CI, bugs
  kit.py pr-wait <pr> [--timeout 540]     wait (in the script, free) until CI finishes; one line: passed/failed/pending
  kit.py ship <pr> | --branch <b> --title <t>   wait for CI, merge-check, merge, update the integration branch, codemap
  kit.py preflight [--quiet] [--cached]   can the team work here? branches, remote, gh, worktrees -> work mode
  kit.py work-mode [worktrees|in-place]   show or set how developers work (team.md -> Parallelism), then agents-sync
  kit.py agents               list the team's agents (name, model, effort, role) and the main-session agent
  kit.py features             list every feature record (docs/features) with status and open bugs
  kit.py feature <id>         one feature: record, PRs, files, releases, bugs, deferred items, local plan
  kit.py bug-new "<title>" [--feature F7] [--severity high] [--reported-by name]
                              draft a bug report in .agents/bugs/ (the fix PR commits it to docs/bugs/)
  kit.py bugs [--all]         open bugs (committed reports and local drafts)
  kit.py board-archive        move finished rows from .agents/board.md to .agents/board-archive/<month>.md
  kit.py review-pack <pr> | --branch <b> [--feature ID]   everything a reviewer needs in ONE output:
                              size, required reviews, acceptance criteria, red-flag hits, codemap lines, diff
  kit.py setup-branches       create the integration branch locally and on the remote if missing
  kit.py protect-branches [--apply] [--approvals N]   show/apply GitHub branch protection
"""
import datetime as dt
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys

IGNORED_DIRS = {".git", "target", "node_modules", ".venv", "venv", "__pycache__", "dist", "build", ".agents", ".claude",
                ".tox", ".mypy_cache", ".pytest_cache", "dbt_packages", "logs", ".gradle", "out"}


def root() -> pathlib.Path:
    env = os.environ.get("KIT_ROOT")
    if env:
        return pathlib.Path(env)
    try:
        top = subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True, check=True).stdout.strip()
        if (pathlib.Path(top) / ".agents" / "stack.json").exists():
            return pathlib.Path(top)
    except Exception:  # noqa: BLE001
        pass
    return pathlib.Path(__file__).resolve().parents[2]


ROOT = root()


def stack() -> dict:
    p = ROOT / ".agents" / "stack.json"
    if not p.exists():
        sys.exit("kit: .agents/stack.json not found")
    return json.loads(p.read_text())


def sh(cmd: str, **kw) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, shell=True, cwd=kw.pop("cwd", ROOT), text=True, **kw)


def ok(cmd: str) -> bool:
    return sh(cmd, capture_output=True).returncode == 0


def glob_re(pattern: str) -> re.Pattern:
    """gitignore-like glob -> regex: ** spans directories, * stays within one."""
    out, i = "", 0
    while i < len(pattern):
        if pattern.startswith("**/", i):
            out, i = out + "(?:.*/)?", i + 3
        elif pattern.startswith("**", i):
            out, i = out + ".*", i + 2
        elif pattern[i] == "*":
            out, i = out + "[^/]*", i + 1
        elif pattern[i] == "?":
            out, i = out + "[^/]", i + 1
        else:
            out, i = out + re.escape(pattern[i]), i + 1
    return re.compile(out + r"\Z")


def matches(path: str, globs: list) -> bool:
    return any(glob_re(g).match(path) for g in globs)


def walk(base: pathlib.Path, globs: list) -> list:
    found = []
    for dirpath, dirnames, filenames in os.walk(base):
        dirnames[:] = sorted(d for d in dirnames if d not in IGNORED_DIRS and not d.startswith("."))
        for f in sorted(filenames):
            rel = (pathlib.Path(dirpath) / f).relative_to(base).as_posix()
            if matches(rel, globs):
                found.append(pathlib.Path(dirpath) / f)
    return found


# ---------------------------------------------------------------- branches

def team_setting(name: str, default: str) -> str:
    p = ROOT / ".agents" / "team.md"
    if p.exists():
        m = re.search(rf"^\|\s*{re.escape(name)}\s*\|\s*`?([^|`]+?)`?\s*\|", p.read_text(), re.M)
        if m:
            return m.group(1).strip()
    return default


def branches() -> tuple:
    return team_setting("integration_branch", "develop"), team_setting("release_branch", "main")


def ref_exists(ref: str) -> bool:
    return bool(git("rev-parse", "--verify", "-q", ref).strip())


def base_ref() -> str:
    """Best existing ref for the integration branch (falls back to the release branch)."""
    integ, rel = branches()
    for r in (f"origin/{integ}", integ, f"origin/{rel}", rel):
        if ref_exists(r):
            return r
    return rel


# ---------------------------------------------------------------- gate

def run_step(step: dict, log_dir: pathlib.Path, max_lines: int, error_re: str) -> bool:
    cmd = step["cmd"]
    if step.get("needs") and not ok(step["needs"]):
        if step.get("fallback"):
            cmd = step["fallback"]
        elif step.get("optional"):
            print(f"skip {step['name']} (tool not installed)")
            return True
    log = log_dir / f"{step['name']}.log"
    with log.open("w") as f:
        rc = sh(cmd, stdout=f, stderr=subprocess.STDOUT).returncode
    if rc == 0:
        print(f"ok   {step['name']}")
        return True
    print(f"FAIL {step['name']}")
    lines = log.read_text(errors="replace").splitlines()
    rx = re.compile(error_re) if error_re else None
    picked = [l for l in lines if rx and rx.search(l)] or lines[-max_lines:]
    for l in picked[:max_lines]:
        print(l)
    print(f"full log: {log.relative_to(ROOT)}")
    print(f"GATE: FAIL ({step['name']})")
    return False


def gate(only: str = None, extra: bool = False) -> int:
    s = stack()
    g = s.get("gate", {})
    req = g.get("requires_file")
    if req and not (ROOT / req).exists():
        print(f"GATE: SKIP (no {req} yet)")
        return 0
    log_dir = ROOT / ".agents" / ".gate"
    log_dir.mkdir(parents=True, exist_ok=True)
    max_lines = int(os.environ.get("GATE_MAX_LINES", "25"))
    steps = [st for st in g.get("steps", []) if not only or st["name"] == only]
    if extra:
        steps += g.get("check_steps", [])
    for st in steps:
        if not run_step(st, log_dir, max_lines, g.get("error_regex", "")):
            return 1
    print("GATE: PASS" if not only else f"{only}: PASS")
    if not only and not [l for l in git("status", "--porcelain").splitlines() if l and not l[3:].startswith(".agents/")]:
        record_gate_pass(git("rev-parse", "HEAD").strip())
    return 0


def gate_file() -> pathlib.Path:
    common = git("rev-parse", "--git-common-dir").strip() or ".git"
    p = pathlib.Path(common)
    return (p if p.is_absolute() else ROOT / p) / "kit-gate-passed"


def record_gate_pass(sha: str) -> None:
    """Commits whose full gate passed on a clean tree (shared by all worktrees); merge-check --branch reads it."""
    if not sha:
        return
    f = gate_file()
    try:
        old = f.read_text().split() if f.exists() else []
        f.write_text("\n".join((old + [sha])[-500:]) + "\n")
    except OSError:
        pass


def gate_passed(ref: str) -> bool:
    sha = git("rev-parse", ref).strip()
    f = gate_file()
    return bool(sha) and f.exists() and sha in f.read_text().split()


def db_check() -> int:
    d = stack().get("db_check")
    if not d:
        print("db-check: no data stores configured for this stack")
        return 0
    if d.get("requires") and not (ROOT / d["requires"]).exists():
        print(f"db-check: {d['requires']} does not exist yet")
        return 0
    return sh(d["cmd"]).returncode


# ---------------------------------------------------------------- test-check

def git(*args) -> str:
    return subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True).stdout


def test_check(base: str = None) -> int:
    tc = stack().get("test_check")
    if not tc:
        print("tests: no test-check configured")
        return 0
    if not base:
        base = base_ref()
    names = set(git("diff", "--name-only", f"{base}...HEAD").split()) | set(git("diff", "--name-only").split())
    tests = [n for n in names if matches(n, tc.get("test_globs", []))]
    src = [n for n in names if matches(n, tc.get("source_globs", [])) and n not in tests]
    if not src:
        print("tests: no production code changes")
        return 0
    trailer = re.search(r"^No-Tests:\s*(\S.*)$", git("log", "--format=%B", f"{base}..HEAD"), re.M)
    if trailer:
        print(f"tests: skipped by No-Tests trailer: {trailer.group(1)}")
        return 0
    patch = git("diff", f"{base}...HEAD") + git("diff")
    marker = tc.get("test_marker_regex")
    added_markers = marker and re.search(r"^\+.*(?:" + marker + ")", patch, re.M)
    if tests or added_markers:
        print("tests: ok")
        return 0
    print("tests: FAIL - production code changed but no unit tests were added:")
    for n in sorted(src)[:10]:
        print(f"  {n}")
    print("Add tests for the new behaviour, or add a commit trailer 'No-Tests: <reason>' if truly none apply.")
    return 1


# ---------------------------------------------------------------- codemap

def codemap(out: str = None) -> int:
    s = stack()
    cm = s.get("codemap", {})
    out_path = ROOT / (out or ".agents/codemap.md")
    out_path.parent.mkdir(parents=True, exist_ok=True)
    doc_re = re.compile(cm["doc_regex"], re.M) if cm.get("doc_regex") else None
    item_re = re.compile(cm["item_regex"], re.M) if cm.get("item_regex") else None
    route_re = re.compile(cm["route_regex"]) if cm.get("route_regex") else None
    lines = ["# Code map", "",
             f"_Generated {dt.datetime.now(dt.timezone.utc):%Y-%m-%dT%H:%MZ} at commit {git('rev-parse', '--short', 'HEAD').strip() or 'none'} "
             f"on {git('branch', '--show-current').strip() or '?'}. Do not edit by hand._", ""]

    groups = []
    for pat in cm.get("groups", ["."]):
        groups += sorted(p for p in ROOT.glob(pat) if p.is_dir() and p.name not in IGNORED_DIRS) if pat != "." else [ROOT]
    body = []
    for gdir in groups:
        files = walk(gdir, cm.get("file_globs", ["**/*"]))
        if not files:
            continue
        desc = ""
        man = gdir / cm.get("group_manifest", "") if cm.get("group_manifest") else None
        if man and man.is_file() and cm.get("group_desc_regex"):
            m = re.search(cm["group_desc_regex"], man.read_text(errors="replace"), re.M)
            desc = m.group(1) if m else ""
        title = "(root)" if gdir == ROOT else gdir.relative_to(ROOT).as_posix()
        body += [f"### {title}" + (f" - {desc}" if desc else "")]
        for f in files:
            text = f.read_text(errors="replace")
            doc = doc_re.search(text).group(1).strip() if doc_re and doc_re.search(text) else ""
            items = []
            if item_re:
                for m in item_re.finditer(text):
                    items.append(" ".join(x for x in m.groups() if x))
            items = sorted(dict.fromkeys(items))
            item_txt = ", ".join(items)[:400]
            body.append(f"- `{f.relative_to(ROOT).as_posix()}`" + (f" - {doc}" if doc else "") + (f" | {item_txt}" if item_txt else ""))
        body.append("")
    if body:
        lines += [f"## {cm.get('modules_title', 'Modules')}", ""] + body
    else:
        lines += ["No source files yet.", ""]

    src_globs = cm.get("source_globs", cm.get("file_globs", []))
    if route_re:
        routes = []
        for f in walk(ROOT, src_globs):
            for n, l in enumerate(f.read_text(errors="replace").splitlines(), 1):
                for m in route_re.finditer(l):
                    routes.append(f"- `{m.group(1)}` -> {f.relative_to(ROOT).as_posix()}:{n}")
        if routes:
            lines += [f"## {cm.get('routes_title', 'Routes')}", ""] + sorted(set(routes)) + [""]

    for section in cm.get("file_lists", []):
        files = walk(ROOT, section["globs"])
        if files:
            lines += [f"## {section['title']}", ""] + [f"- {f.relative_to(ROOT).as_posix()}" for f in files[:200]] + [""]

    env = ROOT / ".env.example"
    if env.exists():
        keys = re.findall(r"^[#\s]*([A-Z][A-Z0-9_]*)=", env.read_text(), re.M)
        if keys:
            lines += ["## Config keys (names from .env.example)", "", ", ".join(dict.fromkeys(keys)), ""]

    feats = records(FEATURE_DIR)
    if feats:
        lines += ["## Features (docs/features; details: just feature <id>)", ""]
        for f in feats:
            lines.append(f"- {f['id']} {f['title']} [{f.get('status', '-')}]" + (f" - {f['summary'][:120]}" if f.get("summary") else ""))
        lines.append("")
    open_bugs = [b for b in records(BUG_DIR) + records(BUG_DRAFTS) if b.get("status", "").lower() in OPEN_BUG]
    if open_bugs:
        seen = set()
        lines += ["## Open bugs (just bugs)", ""]
        for b in open_bugs:
            if b["id"] in seen:
                continue
            seen.add(b["id"])
            lines.append(f"- {b['id']} [{b.get('severity', '-')}] {b['title']} (feature {b.get('feature', '-')})")
        lines.append("")

    integ, rel = branches()
    base = next((b for b in (integ, rel) if ref_exists(b)), "")
    if base:
        inflight = []
        for b in git("for-each-ref", "--format=%(refname:short)", "refs/heads").split():
            if b in (integ, rel) or subprocess.run(["git", "merge-base", "--is-ancestor", b, base], cwd=ROOT).returncode == 0:
                continue
            changed = [l.split("\t") for l in git("diff", "--name-status", f"{base}...{b}").splitlines() if l]
            changed = [(c[0][0], c[-1]) for c in changed if matches(c[-1], src_globs)][:15]
            if not changed:
                continue
            verbs = {"A": "added", "M": "changed", "D": "deleted", "R": "renamed"}
            inflight.append(f"### {b} (last commit {git('log', '-1', '--format=%cr', b).strip()})")
            inflight += [f"- {verbs.get(st, st)} {p}" for st, p in changed]
            if route_re:
                new = sorted({m.group(1) for l in git("diff", f"{base}...{b}").splitlines() if l.startswith("+")
                              for m in route_re.finditer(l)})
                if new:
                    inflight.append("- new routes: " + ", ".join(new))
            inflight.append("")
        if inflight:
            lines += [f"## In-flight (not merged into {base} yet)", ""] + inflight

    tmp = out_path.with_suffix(".tmp")
    tmp.write_text("\n".join(lines).rstrip() + "\n")
    tmp.replace(out_path)
    print(f"codemap: {len(lines)} lines -> {out_path.relative_to(ROOT)}")
    return 0




# ---------------------------------------------------------------- review policy

SENSITIVE_DEFAULT = ["**/auth/**", "**/*auth*", "**/*security*", "**/*secret*", "**/*token*", "**/*crypt*",
                     "**/*permission*", "**/config/**", "**/*config*", "**/settings*", "**/migrations/**", "**/*.sql",
                     ".github/**", "**/Dockerfile", "Dockerfile", ".env.example", "**/deny.toml", "**/*.lock"]
TEST_KINDS = ("feat", "fix", "refactor", "perf", "test")
# docs-only changes get a light review (team.md -> Review policy -> light_review_docs). Files that steer agents,
# CI or settings are never "light", even when they are markdown.
LIGHT_GLOBS = ["docs/**", "**/*.md", "*.md", "**/*.rst", "**/*.txt", "LICENSE*", "**/*.png", "**/*.jpg", "**/*.svg"]
NEVER_LIGHT = [".claude/**", ".agents/**", ".github/**", "CLAUDE.md", "AGENTS.md", "**/CLAUDE.md", "**/AGENTS.md",
               "**/requirements*.txt", "**/constraints*.txt"]


def pr_refs(info: dict) -> tuple:
    """Local refs for a PR's base and head (fetches them when a remote exists)."""
    base, head = info["baseRefName"], info["headRefName"]
    subprocess.run(["git", "fetch", "-q", "origin", base, head], cwd=ROOT, capture_output=True)
    b = f"origin/{base}" if ref_exists(f"origin/{base}") else base
    h = f"origin/{head}" if ref_exists(f"origin/{head}") else head
    return b, h


def diff_stats(base: str, head: str) -> list:
    out = []
    for line in git("diff", "--numstat", f"{base}...{head}").splitlines():
        a, d, path = (line.split("\t") + ["", "", ""])[:3]
        out.append((int(a) if a.isdigit() else 0, int(d) if d.isdigit() else 0, path))
    return out


def required_reviews(kind: str, stats: list) -> tuple:
    s = stack()
    large = int(team_setting("large_pr_lines", "150"))
    sec_policy = team_setting("security_review", "sensitive-or-large").lower()
    test_policy = team_setting("tester_review", "behaviour-change").lower()
    tester_min = int(team_setting("tester_min_source_lines", "20"))
    sensitive = SENSITIVE_DEFAULT + s.get("security_sensitive_globs", [])
    src_globs = s.get("test_check", {}).get("source_globs", [])
    test_globs = s.get("test_check", {}).get("test_globs", [])
    total = sum(a + d for a, d, _ in stats)
    src_lines = sum(a + d for a, d, p in stats if matches(p, src_globs) and not matches(p, test_globs))
    sens = [p for _, _, p in stats if matches(p, sensitive)]
    if team_setting("light_review_docs", "yes").lower() == "yes" and stats and not sens \
            and all(matches(p, LIGHT_GLOBS) and not matches(p, NEVER_LIGHT) for _, _, p in stats):
        return ["light"], [f"docs only ({len(stats)} files, {total} lines): no reviewer launch; tech lead reads the diff"]
    req, why = ["code"], [f"{total} changed lines ({src_lines} production)"]
    if sec_policy == "always" or sens or total >= large:
        req.append("security")
        why.append("security: " + ("always" if sec_policy == "always" else f"sensitive files {', '.join(sens[:4])}" if sens else f">= {large} lines"))
    if kind in TEST_KINDS and (test_policy == "always" or (test_policy == "behaviour-change" and src_lines >= tester_min)
                               or (test_policy == "large" and total >= large)):
        req.append("tests")
        why.append(f"tests: {test_policy}")
    for dr in domain_reviews():  # stack-specific reviewers chosen by the paths touched (stack.json -> domain_reviews)
        hit = [p for _, _, p in stats if matches(p, dr.get("globs", []))]
        if hit:
            req.append(dr["name"])
            why.append(f"{dr['name']} ({dr['agent']}): {', '.join(hit[:3])}")
    return req, why


def domain_reviews() -> list:
    return [d for d in stack().get("domain_reviews", []) if d.get("name") and d.get("agent")]


def review_file(name: str) -> str:
    base = {"code": "code-review.md", "security": "security.md", "tests": "tests-<area>.md"}
    return base.get(name) or next((d.get("file", f"{name}.md") for d in domain_reviews() if d["name"] == name), f"{name}.md")


def current_branch() -> str:
    return git("rev-parse", "--abbrev-ref", "HEAD").strip()


def review_plan(argv: list, show: bool = True) -> tuple:
    integ, _ = branches()
    if "--diff" in argv:
        # local review without a PR: --diff <base>..<head> or --diff <base> (head = HEAD); used in in-place mode
        rng = argv[argv.index("--diff") + 1]
        base, _, headref = rng.partition("..")
        headref = headref or "HEAD"
        head = current_branch() if headref == "HEAD" else headref
        for r in (base, headref):
            if not ref_exists(r):
                sys.exit(f"kit: unknown revision {r!r} in --diff {rng}")
        kind, fid = classify(head)
        if "--feature" in argv:
            fid = argv[argv.index("--feature") + 1]
        stats = diff_stats(base, headref)
        req, why = required_reviews(kind, stats)
        if show:
            print(f"reviews needed for {rng}: {', '.join(req)}  ({'; '.join(why)})")
        return req, why, base, headref, head, kind, fid, stats
    if "--branch" in argv:
        head = argv[argv.index("--branch") + 1]
        base, headref = base_ref(), head
    else:
        info = pr_info(argv[0])
        head = info["headRefName"]
        base, headref = pr_refs(info)
    kind, fid = classify(head)
    stats = diff_stats(base, headref)
    req, why = required_reviews(kind, stats)
    if show:
        print(f"reviews needed for {head}: {', '.join(req)}  ({'; '.join(why)})")
    return req, why, base, headref, head, kind, fid, stats

# ---------------------------------------------------------------- envelope reviews (no reviewer agents)

ENVELOPE_EXCLUDE = ["**/migrations/**", "**/*.sql", ".github/**", "**/Dockerfile", "Dockerfile", "**/*.lock", ".env.example",
                    "**/auth/**", "**/*secret*", "**/*crypt*", ".claude/**", "CLAUDE.md", "AGENTS.md", "**/CLAUDE.md",
                    "**/package.json", "**/Cargo.toml", "**/pyproject.toml", "**/requirements*.txt", "**/go.mod", "**/pom.xml",
                    "**/build.gradle*"]  # new dependencies always get the full security reviewer
ID_PREFIX = {"code": "CR", "security": "SEC", "tests": "TS"}
SEVERITIES = {"security": "critical | high | medium | low | info"}
VERDICTS = {"security": ("PASS", "BLOCK")}


def skill_sections(skill: str, pattern: str) -> str:
    f = ROOT / ".claude" / "skills" / skill / "SKILL.md"
    if not f.exists():
        return ""
    text = re.sub(r"^---\n.*?\n---\n", "", f.read_text(errors="replace"), count=1, flags=re.S)
    parts = re.split(r"(?m)^(## .*)$", text)
    rx = re.compile(pattern, re.I)
    return "".join(parts[i] + parts[i + 1] for i in range(1, len(parts), 2) if rx.search(parts[i])).strip()


def envelope_checklist(name: str) -> str:
    roles = stack().get("skill_roles", {})
    if name == "code":
        return "\n\n".join(x for x in (skill_sections("code-review", r"red-flag|checklist|severity"),
                                       skill_sections(roles.get("core", ""), r"golden rules")) if x)
    if name == "security":
        return "\n\n".join(x for x in (skill_sections("security-review", r"secrets|baseline|severity"),
                                       skill_sections(roles.get("security", ""), r"golden rules|red.flag")) if x)
    dr = next((d for d in domain_reviews() if d["name"] == name), {})
    out = [skill_sections(c["skill"], c.get("sections", ".")) for c in dr.get("envelope_checklist", [])]
    return "\n\n".join(x for x in out if x) or f"Review as {dr.get('agent', name)} would: correctness of this area."


def agent_model_effort(names: list) -> tuple:
    team = (ROOT / ".agents" / "team.md").read_text() if (ROOT / ".agents" / "team.md").exists() else ""
    order = ["low", "medium", "high", "xhigh", "max"]
    model, effort = "", "medium"
    for n in names:
        m = re.search(rf"^\|\s*{re.escape(n)}\s*\|\s*([^|]+?)\s*\|\s*([^|]+?)\s*\|", team, re.M)
        if m:
            model = model or (m.group(1) if m.group(1) not in ("-", "inherit") else "")
            if m.group(2) in order and order.index(m.group(2)) > order.index(effort):
                effort = m.group(2)
    return model, effort


def envelope_schema() -> dict:
    finding = {"type": "object", "properties": {k: {"type": "string"} for k in ("severity", "file_line", "issue", "why", "fix")},
               "required": ["severity", "file_line", "issue", "fix"]}
    prev = {"type": "object", "properties": {"id": {"type": "string"}, "status": {"type": "string", "enum": ["fixed", "not fixed"]}},
            "required": ["id", "status"]}
    review = {"type": "object", "properties": {"name": {"type": "string"}, "verdict": {"type": "string"},
                                               "previous": {"type": "array", "items": prev}, "findings": {"type": "array", "items": finding}},
              "required": ["name", "verdict", "findings"]}
    return {"type": "object", "properties": {"need_full_review": {"type": "boolean"}, "reason": {"type": "string"},
                                             "reviews": {"type": "array", "items": review}},
            "required": ["need_full_review", "reviews"]}


ENVELOPE_SYSTEM = """You are an independent code reviewer for a software team. Everything you may use is in the message:
the change (diff with surrounding lines), the checklists to apply, project rules, and earlier findings if this is a
re-review. You have no tools and cannot open files.

Rules:
- Review only the change and what it affects. Report real problems with a concrete reason; no style preferences.
- Apply each requested review's checklist and severity scale exactly. One entry in "reviews" per requested review,
  with "name" exactly as given.
- Verdict per review: code and domain reviews APPROVE or REQUEST CHANGES; security PASS or BLOCK. A blocking finding
  (blocker/major, or security critical/high) means REQUEST CHANGES / BLOCK.
- file_line is "path:line" using new-file line numbers from the diff.
- Re-review: give every earlier finding id a status (fixed / not fixed) in "previous", and add only new findings.
- If you cannot judge something important without seeing code that is not in the message (a caller, a type, a test
  file), set need_full_review to true and name what is missing in "reason". Never guess about code you cannot see.
"""


def envelope(argv: list) -> int:
    if not argv:
        sys.exit("usage: kit.py envelope <pr> | --branch <b> | --diff <base>..<head> [--feature ID] [--dry-run]")
    dry = "--dry-run" in argv
    req, why, base, headref, head, kind, fid, stats = review_plan(argv, show=False)
    feature = argv[argv.index("--feature") + 1] if "--feature" in argv else fid
    pr_no = argv[0] if argv and not argv[0].startswith("--") else ""
    ref = review_ref(pr=pr_no, branch=head)
    total = sum(a + d for a, d, _ in stats)
    limit = int(team_setting("envelope_max_lines", "150"))
    excl = ENVELOPE_EXCLUDE + stack().get("envelope_exclude_globs", [])
    blocked = [p for _, _, p in stats if matches(p, excl)]
    covered = [r for r in req if r != "tests" and r != "light"]
    agents = {"code": "code-reviewer", "security": "security-reviewer", "tests": "tester", **{d["name"]: d["agent"] for d in domain_reviews()}}
    if not feature:
        sys.exit("kit: pass --feature <id> (branch name has no feature id)")
    if "light" in req or not covered:
        print(f"envelope: nothing to review by envelope ({', '.join(req) or 'none'} required)")
        return 0
    reasons = []
    if team_setting("envelope_reviews", "yes").lower() != "yes":
        reasons.append("envelope_reviews is off in team.md")
    if total > limit:
        reasons.append(f"{total} changed lines > envelope_max_lines {limit}")
    if blocked:
        reasons.append("files that always get full reviewers: " + ", ".join(blocked[:4]))
    cmd0 = team_setting("envelope_command", "claude")
    if not shutil.which(cmd0):
        reasons.append(f"`{cmd0}` not found on PATH")
    if reasons:
        print(f"envelope: not eligible ({'; '.join(reasons)}). Launch the reviewers instead: "
              + ", ".join(agents.get(r, r) for r in covered))
        return 3
    d_out = ROOT / ".agents" / "reviews" / feature / ref
    rules = ROOT / ".agents" / "owner-rules.md"
    owner = re.sub(r"<!--.*?-->", "", rules.read_text(errors="replace"), flags=re.S) if rules.exists() else ""
    owner = "\n".join(l for l in owner.splitlines() if re.match(r"^\s*-\s+\S", l))
    plan = ROOT / ".agents" / "plans" / f"{feature}.md"
    acc = ""
    if plan.exists():
        m = re.search(r"^## Acceptance criteria\s*\n(.*?)(?=^## |\Z)", plan.read_text(), re.S | re.M)
        acc = m.group(1).strip() if m else ""
    max_chars = int(team_setting("envelope_max_diff_chars", "60000"))
    diff = git("diff", "-U12", f"{base}...{headref}")
    if len(diff) > max_chars:
        print(f"envelope: not eligible (diff with context is {len(diff)} chars > {max_chars}). Launch the reviewers instead: "
              + ", ".join(agents.get(r, r) for r in covered))
        return 3
    parts = [f"# Change to review\nBranch {head} -> {base}; feature {feature}; {len(stats)} files, {total} changed lines.",
             "Requested reviews: " + ", ".join(covered)]
    if acc:
        parts.append("## Acceptance criteria\n" + acc)
    if owner:
        parts.append("## Owner rules (apply everywhere)\n" + owner)
    for r in covered:
        sev = SEVERITIES.get(r, "blocker | major | minor | nit")
        v = VERDICTS.get(r, ("APPROVE", "REQUEST CHANGES"))
        parts.append(f"## Review '{r}' ({agents.get(r, r)}): severities {sev}; verdict {v[0]} or {v[1]}\n" + envelope_checklist(r))
        f = d_out / review_file(r)
        if f.exists():
            _, rows = findings(f)
            open_rows = [x for x in rows if x.get("status", "").lower() in OPEN]
            if open_rows:
                parts.append(f"### Earlier findings for '{r}' (give each a status in 'previous')\n" + "\n".join(
                    f"- {x.get('id')} [{x.get('severity')}] {x.get('file:line')}: {x.get('issue')}" for x in open_rows))
    parts.append("## Diff\n```diff\n" + diff + "\n```")
    message = "\n\n".join(parts)
    if dry:
        print(message)
        print(f"\n[envelope: ~{len(message) // 4:,} tokens of content; would write {', '.join(str(d_out / review_file(r)) for r in covered)}]")
        return 0
    model, effort = agent_model_effort([agents.get(r, r) for r in covered])
    import tempfile
    with tempfile.TemporaryDirectory() as td:
        sp = pathlib.Path(td) / "system.md"
        sp.write_text(ENVELOPE_SYSTEM)
        base_cmd = [shutil.which(cmd0), "-p", "--tools", "", "--system-prompt-file", str(sp), "--output-format", "json",
                    "--json-schema", json.dumps(envelope_schema()), "--no-session-persistence", "--effort", effort]
        if model:
            base_cmd += ["--model", model]
        env = {k: v for k, v in os.environ.items() if k not in ("CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT")}
        r = None
        for extra in (["--safe-mode"], ["--settings", json.dumps({"disableAllHooks": True})]):
            r = subprocess.run(base_cmd + extra, input=message, capture_output=True, text=True, cwd=ROOT, env=env, timeout=900)
            if r.returncode == 0 or "safe-mode" not in (r.stderr + r.stdout).lower():
                break
    try:
        out = json.loads(r.stdout)
    except (ValueError, AttributeError):
        out = {}
    data = out.get("structured_output") if isinstance(out, dict) else None
    if r.returncode != 0 or not isinstance(data, dict):
        print(f"envelope: the review call failed ({(r.stderr or r.stdout or '').strip()[-300:]}). Launch the reviewers instead: "
              + ", ".join(agents.get(x, x) for x in covered))
        return 3
    u = out.get("usage") or {}
    inp = sum(int(u.get(k) or 0) for k in ("input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"))
    used = inp + int(u.get("output_tokens") or 0)
    try:
        import importlib.util
        spec = importlib.util.spec_from_file_location("kit_budget", ROOT / ".claude" / "kit" / "budget.py")
        b = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(b)
        now = dt.datetime.now()
        b.write_row({"date": now.date().isoformat(), "time": now.strftime("%H:%M"), "task": b.current_task(),
                     "agent": "envelope-review", "tokens": used, "processed": used, "source": "auto", "start": inp,
                     "session": "", "note": f"{ref}: {', '.join(covered)}"})
    except Exception:  # noqa: BLE001
        pass
    if data.get("need_full_review"):
        print(f"envelope: the reviewer needs more context ({data.get('reason', '?')}). Launch the reviewers instead: "
              + ", ".join(agents.get(x, x) for x in covered) + f"  [{used:,} tokens]")
        return 3
    d_out.mkdir(parents=True, exist_ok=True)
    summary = []
    for rv in data.get("reviews", []):
        name = rv.get("name", "")
        if name not in covered:
            continue
        f = d_out / review_file(name)
        prefix = ID_PREFIX.get(name, name.upper()[:4]) + f"-{feature}-"
        rows, round_no = [], 1
        if f.exists():
            _, rows = findings(f)
            m = re.search(r"^Round:\s*(\d+)", f.read_text(), re.M)
            round_no = int(m.group(1)) + 1 if m else 2
            status = {p.get("id"): p.get("status") for p in rv.get("previous", [])}
            for x in rows:
                if x.get("id") in status and x.get("status", "").lower() in OPEN:
                    x["status"] = status[x["id"]]
        nums = [int(n) for x in rows for n in re.findall(r"-(\d+)$", x.get("id", ""))]
        nxt = max(nums, default=0)
        for fd in rv.get("findings", []):
            nxt += 1
            rows.append({"id": f"{prefix}{nxt}", "severity": fd.get("severity", "").lower(), "file:line": fd.get("file_line", ""),
                         "issue": fd.get("issue", ""), "why it matters": fd.get("why", ""), "suggested fix": fd.get("fix", ""),
                         "status": "open", "tech-lead decision": ""})
        block = {"critical", "high"} if name == "security" else {"blocker", "major", "critical", "high"}
        yes, no = VERDICTS.get(name, ("APPROVE", "REQUEST CHANGES"))
        model_ok = str(rv.get("verdict", "")).upper().startswith(("APPROVE", "PASS"))
        still_open = [x for x in rows if x.get("status", "").lower() in OPEN]
        verdict = no if open_blockers(rows, block) else yes if (model_ok or not still_open) else no
        cols = ["id", "severity", "file:line", "issue", "why it matters", "suggested fix", "status", "tech-lead decision"]
        clean = lambda s: str(s).replace("|", "/").replace("\n", " ").strip()
        table = ["| " + " | ".join(cols) + " |", "|" + "---|" * len(cols)]
        table += ["| " + " | ".join(clean(x.get(c, "")) for c in cols) + " |" for x in rows]
        f.write_text(f"# {feature} - {name} review ({agents.get(name, name)}, by envelope) - {head}\n\n"
                     f"Verdict: {verdict}\nRound: {round_no}\nReviewed by: envelope (one tool-free Claude call)\n\n" + "\n".join(table) + "\n")
        n_open = len([x for x in rows if x.get("status", "").lower() in OPEN])
        summary.append(f"{name} {verdict} ({n_open} open)")
    missing = [x for x in covered if x not in {rv.get("name") for rv in data.get("reviews", [])}]
    print(f"envelope {ref}: " + "; ".join(summary) + f"  [{used:,} tokens, no reviewer agents]"
          + (f"; missing from the answer: {', '.join(missing)} - launch those reviewers" if missing else "")
          + (" ; tester still required (runs tests)" if "tests" in req else ""))
    print(f"files: {d_out.relative_to(ROOT)}/  -> next: `just triage {feature} --ref {ref}` if anything is open")
    return 0 if not missing else 3

# ---------------------------------------------------------------- the library: one section on request

def _clean_heading(h: str) -> str:
    h = re.sub(r"^#+\s*", "", h)
    h = re.sub(r"^\d+[a-z]?(\.\d+)*\.?\s*", "", h)
    return re.sub(r"\s+", " ", re.sub(r"\(.*?\)|`", "", h)).strip(" :-")


def doc_sections() -> list:
    """Every '## ' section of every skill and reference file (long sections also split at '### ')."""
    out = []
    for f in sorted((ROOT / ".claude" / "skills").glob("*/SKILL.md")) + sorted((ROOT / ".claude" / "skills").glob("*/references/*.md")):
        skill = f.parent.name if f.name == "SKILL.md" else f.parent.parent.name
        where = "SKILL" if f.name == "SKILL.md" else f.stem
        text = re.sub(r"^---\n.*?\n---\n", "", f.read_text(errors="replace"), count=1, flags=re.S)
        lines = text.splitlines()
        fence, heads = False, []
        for i, l in enumerate(lines):
            if l.lstrip().startswith("```"):
                fence = not fence
            elif not fence and l.startswith("## "):
                heads.append(i)
        if not heads:  # a reference file without '## ' sections is one section, titled by its '# ' line
            title = next((i for i, l in enumerate(lines) if l.startswith("# ")), None)
            heads = [title] if title is not None else []
            if title is not None:
                lines[title] = "## " + lines[title][2:]
        for k, i in enumerate(heads):
            j = heads[k + 1] if k + 1 < len(heads) else len(lines)
            body = lines[i:j]
            if re.search(r"^(Full detail|In short:).*references/", "\n".join(body), re.M) and len(body) <= 8:
                continue  # a stub that points to a reference file: the reference itself is indexed
            out.append((skill, where, lines[i], "\n".join(body), f, i + 1))
            if len("\n".join(body)) > 6000:
                subs = [x for x in range(i + 1, j) if lines[x].startswith("### ")]
                for m, s in enumerate(subs):
                    e = subs[m + 1] if m + 1 < len(subs) else j
                    out.append((skill, where, lines[s], "\n".join(lines[s:e]), f, s + 1))
    return out


def docs_cmd(argv: list) -> int:
    secs = doc_sections()
    if not argv:
        sys.exit('usage: kit.py docs "<topic>"  |  docs "<skill>: <topic>"  |  docs --list [skill]')
    if argv[0] == "--list":
        want = argv[1] if len(argv) > 1 else ""
        cur = None
        for skill, where, head, body, f, line in secs:
            if want and skill != want:
                continue
            if (skill, where) != cur:
                cur = (skill, where)
                print(f"\n{skill}" + ("" if where == "SKILL" else f" > {where}"))
            print(f"  {_clean_heading(head)}  (~{len(body) // 4} tok)")
        return 0
    query = " ".join(a for a in argv if not a.startswith("--")).lower()
    only = ""
    m = re.match(r"^\s*([a-z0-9-]+)\s*:\s*(.+)$", query)
    if m and any(s[0] == m.group(1) for s in secs):
        only, query = m.group(1), m.group(2)
    for short, longer in stack().get("docs_synonyms", {}).items():  # e.g. {"iv": "implied volatility"}
        if re.search(rf"\b{re.escape(short.lower())}\b", query):
            query += " " + longer.lower()
    words = [w for w in re.findall(r"[a-z0-9&+#.-]{2,}", query) if w not in ("the", "and", "for", "how", "to", "of", "in", "a")]
    if not words:
        sys.exit("kit: give a topic, e.g. docs \"greeks\"")
    scored = []
    for sec in secs:
        skill, where, head, body, f, line = sec
        if only and skill != only:
            continue
        h, name, b = _clean_heading(head).lower(), f"{skill} {where}".lower(), body.lower()
        def hit(w, s):  # short words (iv, pr, ui) must match whole words, not inside "live"
            return bool(re.search(rf"\b{re.escape(w)}\b", s)) if len(w) <= 3 else w in s
        def count(w, s):
            return len(re.findall(rf"\b{re.escape(w)}\b", s)) if len(w) <= 3 else s.count(w)
        score = sum(4 * hit(w, h) + 2 * hit(w, name) + min(count(w, b), 5) * 0.3 for w in words)
        score += 5 * (query in h) + 0.1 * (head.startswith("## "))
        if score > 0:
            scored.append((score, -len(body), sec))
    if not scored:
        print(f'docs: nothing matches "{query}". Try fewer words, or `just docs --list` to see every section.')
        return 1
    scored.sort(key=lambda x: (x[0], x[1]), reverse=True)
    skill, where, head, body, f, line = scored[0][2]
    alts = [f'"{_clean_heading(s[2][2])}" ({s[2][0]})' for s in scored[1:5] if s[2][3] != body]
    lines = body.splitlines()
    limit = int(team_setting("docs_max_lines", "120"))
    print(f"{skill} > {'' if where == 'SKILL' else where + ' > '}{_clean_heading(head)}   "
          f"({len(lines)} lines, ~{len(body) // 4} tok; {f.relative_to(ROOT)}:{line})")
    if alts:
        print("also: " + "; ".join(alts[:4]))
    print()
    print("\n".join(lines[:limit]))
    if len(lines) > limit:
        print(f"\n... {len(lines) - limit} more lines: Read {f.relative_to(ROOT)} offset {line + limit} limit {len(lines) - limit}")
    return 0

# ---------------------------------------------------------------- what an agent reads before it starts

def _tok(chars: int) -> int:
    return (chars + 3) // 4  # rough: ~4 characters per token


def _frontmatter(text: str) -> dict:
    m = re.match(r"^---\n(.*?)\n---\n", text, re.S)
    fm = m.group(1) if m else ""
    skills = re.findall(r"^\s+-\s+([\w-]+)\s*$", fm.split("skills:", 1)[1].split("\n", 1)[1] if "skills:" in fm else "", re.M)
    skills = skills[:len(skills)]
    return {"skills": [s for s in skills if (ROOT / ".claude" / "skills" / s / "SKILL.md").exists()],
            "memory": (re.search(r"^memory:\s*(\S+)", fm, re.M) or [None, ""])[1],
            "omit": bool(re.search(r"^omitClaudeMd:\s*true", fm, re.M)), "body": len(text) - (m.end() if m else 0)}


def claude_md_files() -> list:
    """Every CLAUDE.md level Claude Code gives a sub-agent (that this script can see)."""
    home = pathlib.Path.home()
    out = [home / ".claude" / "CLAUDE.md"] + sorted((home / ".claude" / "rules").glob("*.md"))
    d = ROOT.resolve()
    ups = []
    for parent in d.parents:
        if parent == home or parent == parent.parent:
            break
        ups += [parent / "CLAUDE.md", parent / "AGENTS.md"]
    out += ups + [ROOT / "CLAUDE.md", ROOT / ".claude" / "CLAUDE.md", ROOT / "CLAUDE.local.md", ROOT / "AGENTS.md"]
    for r in sorted((ROOT / ".claude" / "rules").glob("*.md")):
        if not re.match(r"^---\n(?:.*\n)*?paths:", r.read_text(errors="replace")):  # rules with paths load only when matching files are read
            out.append(r)
    return [f for f in out if f.is_file()]


def memory_file(agent: str, scope: str) -> pathlib.Path:
    base = {"project": ROOT / ".claude" / "agent-memory", "local": ROOT / ".claude" / "agent-memory-local",
            "user": pathlib.Path.home() / ".claude" / "agent-memory"}.get(scope)
    return base / agent / "MEMORY.md" if base else pathlib.Path("/nonexistent")


def start_parts(agent: str) -> list:
    f = ROOT / ".claude" / "agents" / f"{agent}.md"
    fm = _frontmatter(f.read_text(errors="replace"))
    parts = [("its own instructions (agent file)", fm["body"])]
    for s in fm["skills"]:
        parts.append((f"handbook: {s}", len((ROOT / ".claude" / "skills" / s / "SKILL.md").read_text(errors="replace"))))
    if fm["omit"]:
        parts.append(("CLAUDE.md levels (skipped: omitClaudeMd)", 0))
    else:
        for c in claude_md_files():
            label = str(c).replace(str(pathlib.Path.home()), "~") if not str(c).startswith(str(ROOT)) else str(c.relative_to(ROOT))
            parts.append((f"house rules: {label}", len(c.read_text(errors="replace"))))
    if fm["memory"] and fm["memory"] != "none":
        mf = memory_file(agent, fm["memory"])
        mem = "\n".join(mf.read_text(errors="replace").splitlines()[:200])[:25000] if mf.exists() else ""
        parts.append((f"notebook: {mf.relative_to(ROOT) if str(mf).startswith(str(ROOT)) else mf} ({len(mem.splitlines())} lines)", len(mem)))
    st = git("status", "--short")[:2000] + git("log", "--oneline", "-5")
    parts.append(("desk snapshot: git status (approx.)", len(st)))
    rules = ROOT / ".agents" / "owner-rules.md"
    if rules.exists():
        own = "\n".join(l for l in re.sub(r"<!--.*?-->", "", rules.read_text(), flags=re.S).splitlines() if re.match(r"^\s*-\s+\S", l))
        parts.append(("owner rules (added by the start hook)", len(own)))
    return parts


def measured_start(agent: str) -> tuple:
    led = ROOT / ".agents" / "budget.csv"
    if not led.exists():
        return 0, 0, 0
    import csv
    with led.open() as fh:
        rows = [r for r in csv.DictReader(fh) if r.get("agent") == agent and r.get("start")]
    if not rows:
        return 0, 0, 0
    starts = [int(r["start"]) for r in rows]
    prompts = [int(m.group(1)) for r in rows for m in [re.search(r"prompt (\d+) tok", r.get("note", ""))] if m]
    return sum(starts) // len(starts), len(starts), (sum(prompts) // len(prompts) if prompts else 0)


def start_report(argv: list) -> int:
    agents_dir = ROOT / ".claude" / "agents"
    names = [argv[0]] if argv else sorted(p.stem for p in agents_dir.glob("*.md"))
    if argv and not (agents_dir / f"{argv[0]}.md").exists():
        sys.exit(f"kit: no agent {argv[0]!r} (just agents lists them)")
    if not argv:
        print(f"{'agent':20} {'known parts':>11} {'measured start':>15}  (just start-report <agent> for the breakdown)")
        for n in names:
            known = sum(_tok(c) for _, c in start_parts(n))
            avg, runs, _ = measured_start(n)
            print(f"{n:20} {known:>10,}  {(f'{avg:,} ({runs} runs)' if runs else '-'):>15}")
        return 0
    n = names[0]
    parts = start_parts(n)
    avg, runs, prompt = measured_start(n)
    print(f"What {n} reads before it starts working (tokens are estimates: ~4 characters per token)\n")
    for label, chars in parts:
        print(f"  {_tok(chars):>7,}  {label}")
    known = sum(_tok(c) for _, c in parts)
    if prompt:
        print(f"  {prompt:>7,}  task message from the manager (average of measured runs)")
        known += prompt
    print(f"  {'-' * 7}\n  {known:>7,}  known parts")
    if runs:
        rest = avg - known
        print(f"  {avg:>7,}  measured start (average of {runs} runs)")
        print(f"  {rest:>7,}  the rest: Claude Code's tool manuals and environment details" + (" (plus anything above that was larger at the time)" if rest > 0 else ""))
    else:
        print("\n  No measured runs yet: after this agent runs once, the measured start and the rest appear here.")
    tips = []
    if any(l.startswith("house rules:") and _tok(c) > 2500 for l, c in parts):
        tips.append("a CLAUDE.md level is large: every agent reads it on every start; keep it an index and move detail to skills")
    if any(l.startswith("notebook:") and _tok(c) > 1000 for l, c in parts):
        tips.append("its notebook is long: trim it, or set memory to none for this agent in team.md")
    if tips:
        print("\nTips: " + "; ".join(tips))
    return 0

# ---------------------------------------------------------------- triage without an agent

AUTO_FIX = {"blocker", "critical", "high", "major", "medium"}
CHEAP = {"minor", "nit", "low", "info"}


def triage(argv: list) -> int:
    """Apply delivery-process triage defaults to undecided open findings. The tech lead is needed only for what this
    prints under 'needs the tech lead' (disputed findings, two reviewers suggesting different fixes for the same spot,
    findings without a clear fix, or unknown severities)."""
    if not argv or argv[0].startswith("--"):
        sys.exit("usage: kit.py triage <feature> [--ref pr-N|branch] [--dry-run]")
    feature, dry = argv[0], "--dry-run" in argv
    base = ROOT / ".agents" / "reviews" / feature
    if "--ref" in argv:
        dirs = [base / argv[argv.index("--ref") + 1]]
    else:
        dirs = [base] + sorted(d for d in base.glob("*") if d.is_dir()) if base.exists() else []
    files = [f for d in dirs if d.exists() for f in sorted(d.glob("*.md")) if f.parent == d]
    if not files:
        print(f"triage {feature}: no findings files under {base.relative_to(ROOT)}")
        return 0
    pending, spots = [], {}
    for f in files:
        _, rows = findings(f)
        for r in rows:
            status = r.get("status", "").lower()
            decision = r.get("tech-lead decision", r.get("decision", "")).strip()
            if status not in OPEN or decision:
                continue
            spot = (r.get("file:line") or "").strip()
            fix = (r.get("suggested fix") or r.get("fix") or "").strip()
            pending.append((f, r, spot, fix))
            if spot:
                spots.setdefault(spot, set()).add(fix.lower())
    if not pending:
        print(f"triage {feature}: nothing undecided")
        return 0
    auto, judge = [], []
    for f, r, spot, fix in pending:
        sev, status = r.get("severity", "").lower(), r.get("status", "").lower()
        why = ("disputed by the developer" if status == "disputed"
               else "reviewers suggest different fixes for the same spot" if spot and len(spots.get(spot, ())) > 1
               else "no clear fix suggested" if sev in AUTO_FIX and len(fix) < 4
               else "unknown severity" if sev not in AUTO_FIX | CHEAP else "")
        (judge if why else auto).append((f, r, "fix now (auto)" if sev in AUTO_FIX else "fix now if cheap (auto)", why))
    if not dry:
        by_file = {}
        for f, r, decision, _ in auto:
            by_file.setdefault(f, {})[r.get("id", "")] = decision
        for f, ids in by_file.items():
            out = []
            for line in f.read_text().splitlines():
                cells = line.strip().strip("|").split("|") if line.strip().startswith("|") else []
                if cells and cells[0].strip() in ids and len(cells) >= 2:
                    cells[-1] = f" {ids[cells[0].strip()]} "
                    line = "|" + "|".join(cells) + "|"
                out.append(line)
            f.write_text("\n".join(out) + "\n")
    rel = lambda f: str(f.relative_to(ROOT))
    print(f"triage {feature}: {len(auto)} decided by the defaults{' (dry run)' if dry else ''}, {len(judge)} need the tech lead")
    groups = {}
    for f, r, decision, _ in auto:
        groups.setdefault(rel(f.parent), []).append(f"{r.get('id', '?')} ({r.get('severity', '?')})")
    for d, ids in groups.items():
        print(f"  fix now -> developer of {d}: {', '.join(ids)}")
    for f, r, _, why in judge:
        print(f"  needs tech lead: {r.get('id', '?')} in {rel(f)} - {why}")
    if not judge:
        print("  no judgment needed: send the fix list straight to the developer(s); no tech-lead launch")
    return 0

# ---------------------------------------------------------------- waiting, shipping, status

def pr_wait(argv: list, quiet: bool = False) -> int:
    """Wait for a PR's CI in the script (no agent turns). Exit 0 passed, 1 failed, 2 still pending at timeout."""
    import time
    if not argv or argv[0].startswith("--"):
        sys.exit("usage: kit.py pr-wait <pr> [--timeout seconds] [--interval seconds]")
    timeout = int(argv[argv.index("--timeout") + 1]) if "--timeout" in argv else 540
    interval = int(argv[argv.index("--interval") + 1]) if "--interval" in argv else 20
    start, state, detail = time.time(), "none", ""
    while True:
        state, detail = ci_state(pr_info(argv[0]))
        if state in ("passed", "failed"):
            break
        if time.time() - start + interval > timeout:
            break
        time.sleep(interval)
    waited = int(time.time() - start)
    if state == "none" and team_setting("require_ci_checks", "yes").lower() == "no":
        state, detail = "passed", "no CI checks (not required)"
    msg = {"passed": "CI PASSED", "failed": "CI FAILED", "pending": "CI STILL RUNNING", "none": "NO CI REPORTED"}[state]
    hint = ""
    if state == "pending":
        hint = " - run `just pr-wait` again (one call per ~9 minutes is fine)"
    elif state == "none":
        hint = (" - if this repo has no CI workflow yet, ask cicd, or set require_ci_checks | no in team.md")
    if not quiet or state != "passed":
        print(f"PR {argv[0]}: {msg} after {waited}s ({detail}){hint}")
    return {"passed": 0, "failed": 1}.get(state, 2)


def update_integration_checkout() -> str:
    """After a merge: fetch; fast-forward the integration branch if this checkout is on it (never switches)."""
    integ, _ = branches()
    subprocess.run(["git", "fetch", "-q", "origin"], cwd=ROOT, capture_output=True)
    if current_branch() == integ and ref_exists(f"origin/{integ}"):
        r = subprocess.run(["git", "merge", "--ff-only", "-q", f"origin/{integ}"], cwd=ROOT, capture_output=True, text=True)
        return f"{integ} updated" if r.returncode == 0 else f"{integ} not fast-forwarded ({r.stderr.strip()[:80]})"
    return f"fetched (checkout is on {current_branch()}, not {integ}: switch with `git switch {integ}` when free)"


def ship(argv: list) -> int:
    """Tech lead's one command: wait for CI, merge-check, merge, update the integration branch, refresh the codemap."""
    if "--branch" in argv:
        rc = merge(argv)
        if rc == 0:
            codemap()
            print("shipped: merged locally; codemap refreshed")
        return rc
    if not argv or argv[0].startswith("--"):
        sys.exit("usage: kit.py ship <pr> | --branch <b> --title '<conventional title>'")
    w = pr_wait([argv[0]] + (["--timeout", argv[argv.index("--timeout") + 1]] if "--timeout" in argv else []), quiet=True)
    if w == 1:
        return 1
    if w == 2:
        print(f"not shipped: CI still running or not reported - run `just ship {argv[0]}` again later")
        return 2
    rc = merge(argv)
    if rc:
        return rc
    note = update_integration_checkout()
    codemap()
    print(f"shipped PR {argv[0]}: merged; {note}; codemap refreshed. Ask other open branches to rebase if they touch the same files.")
    return 0


def status_cmd(argv: list) -> int:
    """Everything the manager needs at session start, in one short output (instead of reading several files)."""
    integ, rel = branches()
    dirty = [l for l in git("status", "--porcelain").splitlines() if l and not l[3:].startswith(".agents/")]
    print(f"checkout: {current_branch() or '?'}{' (uncommitted changes: ' + str(len(dirty)) + ')' if dirty else ''} | "
          f"integration {integ}, release {rel} | work mode {team_setting('work_mode', 'worktrees')}")
    pf = ROOT / PREFLIGHT
    try:
        pj = json.loads(pf.read_text()) if pf.exists() else {}
        print("preflight: " + ("; ".join(pj["problems"]) + " (just preflight)" if pj.get("problems") else f"ok ({pj.get('date', '?')})")
              if pj else "preflight: not run yet (just preflight)")
    except (ValueError, KeyError):
        pass
    r = subprocess.run([sys.executable, str(ROOT / ".claude" / "kit" / "budget.py"), "status"], cwd=ROOT, capture_output=True, text=True)
    task = (ROOT / ".agents" / "current-task")
    print(f"task: {task.read_text().strip() if task.exists() else '-'} | budget: {r.stdout.strip() or '-'}")
    board = ROOT / ".agents" / "board.md"
    if board.exists():
        rows, header = [], None
        for l in board.read_text().splitlines():
            if not l.strip().startswith("|"):
                continue
            cells = [c.strip() for c in l.strip().strip("|").split("|")]
            if header is None and "status" in [c.lower() for c in cells]:
                header = [c.lower() for c in cells]
                continue
            if header and len(cells) == len(header) and not set("".join(cells)) <= set("-: "):
                rows.append(dict(zip(header, cells)))
        done_words = ("merged", "done", "closed", "released", "fixed", "won't fix", "wont fix")
        open_rows = [r_ for r_ in rows if r_.get("status", "").lower() not in done_words]
        counts = {}
        for r_ in rows:
            counts[r_.get("status", "-") or "-"] = counts.get(r_.get("status", "-") or "-", 0) + 1
        print(f"board: {len(rows)} rows ({', '.join(f'{k} {v}' for k, v in counts.items()) or 'empty'})")
        for r_ in open_rows[:8]:
            print("  - " + " | ".join(v for v in r_.values() if v)[:150])
        if len(open_rows) > 8:
            print(f"  ... {len(open_rows) - 8} more open rows in .agents/board.md")
    if shutil.which("gh") and git("remote").strip():
        out = subprocess.run(["gh", "pr", "list", "--state", "open", "--limit", "15", "--json",
                              "number,title,headRefName,baseRefName,isDraft,statusCheckRollup"], cwd=ROOT, capture_output=True, text=True)
        try:
            prs = json.loads(out.stdout) if out.returncode == 0 else []
        except ValueError:
            prs = []
        print(f"open PRs: {len(prs)}" + ("" if out.returncode == 0 else " (gh unavailable)"))
        for pr in prs:
            state, _ = ci_state(pr)
            print(f"  #{pr['number']} {pr['headRefName']} -> {pr['baseRefName']}  CI {state}{' (draft)' if pr.get('isDraft') else ''}  {pr['title'][:60]}")
    bugs = [b for b in records(BUG_DIR) + records(BUG_DRAFTS) if b.get("status", "").lower() in OPEN_BUG]
    rules = ROOT / ".agents" / "owner-rules.md"
    n_rules = len([l for l in re.sub(r"<!--.*?-->", "", rules.read_text(), flags=re.S).splitlines()
                   if re.match(r"^\s*-\s+\S", l)]) if rules.exists() else 0
    print(f"open bugs: {len({b['id'] for b in bugs})} | features: {len(records(FEATURE_DIR))} | owner rules: {n_rules}")
    return 0

# ---------------------------------------------------------------- preflight and work mode

PREFLIGHT = ".agents/preflight.json"


def worktree_probe() -> tuple:
    import tempfile
    if not git("rev-parse", "--verify", "-q", "HEAD").strip():
        return False, "no commits yet"
    d = pathlib.Path(tempfile.mkdtemp(prefix="kit-wt-")) / "probe"
    r = subprocess.run(["git", "worktree", "add", "--detach", str(d), "HEAD"], cwd=ROOT, capture_output=True, text=True)
    if r.returncode:
        return False, (r.stderr.strip().splitlines() or ["git worktree add failed"])[-1][:160]
    subprocess.run(["git", "worktree", "remove", "--force", str(d)], cwd=ROOT, capture_output=True)
    return True, "ok"


def preflight(argv: list) -> int:
    quiet, cached = "--quiet" in argv, "--cached" in argv
    out = ROOT / PREFLIGHT
    if cached and out.exists():
        try:
            prev = json.loads(out.read_text())
            if prev.get("date") == dt.date.today().isoformat() and prev.get("head") == git("rev-parse", "HEAD").strip():
                if quiet and prev.get("problems"):
                    print("Kit preflight: " + "; ".join(prev["problems"]) + ". Run `just preflight` for fixes.")
                return 1 if prev.get("problems") else 0
        except Exception:  # noqa: BLE001
            pass
    integ, rel = branches()
    checks, problems, fixes = [], [], []
    is_repo = (ROOT / ".git").exists() or bool(git("rev-parse", "--git-dir").strip())
    checks.append(("git repository", is_repo, ""))
    has_remote = bool(git("remote").strip())
    local_branches = git("branch", "--format=%(refname:short)").split()
    for name, label in ((integ, "integration branch"), (rel, "release branch")):
        here = ref_exists(name) or ref_exists(f"origin/{name}")
        checks.append((f"{label} '{name}' exists", here, ""))
        if not here:
            similar = [b for b in local_branches if b != name and (b.startswith(name[:3]) or name.startswith(b[:3]))]
            problems.append(f"{label} '{name}' not found")
            fixes.append(f"- {label}: if your team uses another name ({', '.join(similar) or 'existing: ' + ', '.join(local_branches[:6])}), "
                         f"set it in .agents/team.md -> Branches; otherwise create it with `just setup-branches` (asks you first).")
    checks.append(("remote 'origin'", has_remote, "" if has_remote else "no remote: work stays local, merges use `just merge --branch`"))
    gh_ok = bool(shutil.which("gh")) and subprocess.run(["gh", "auth", "status"], cwd=ROOT, capture_output=True).returncode == 0
    checks.append(("gh installed and logged in", gh_ok, "" if gh_ok else "no PRs: reviews use --branch/--diff, merges use `just merge --branch`"))
    kit_committed = bool(git("ls-files", "--", ".claude/settings.json", ".claude/agents").strip())
    checks.append(("kit files committed", kit_committed, "" if kit_committed else "worktree and cloud sessions won't see the kit"))
    if not kit_committed:
        problems.append("kit files not committed")
        fixes.append("- commit the kit on a branch: git switch -c chore/agent-kit && git add -A && git commit -m \"chore: add agent kit\"")
    wt_ok, wt_msg = worktree_probe() if is_repo else (False, "not a git repository")
    checks.append(("git worktrees work", wt_ok, "" if wt_ok else wt_msg))
    mode_now = team_setting("work_mode", "worktrees")
    recommended = "worktrees" if wt_ok else "in-place"
    if mode_now == "worktrees" and not wt_ok:
        problems.append("worktrees fail but work_mode is worktrees")
        fixes.append(f"- worktrees fail ({wt_msg}). Either fix that, or switch to in-place mode: `just work-mode in-place` "
                     "(one developer at a time in this checkout; reviews and gates unchanged). Never swap in general-purpose agents.")
    review_path = "PR" if (gh_ok and has_remote) else "local branch (--branch / --diff)"
    width = max(len(c[0]) for c in checks)
    if not quiet:
        for name, good, note in checks:
            print(f"  {'ok  ' if good else 'FAIL'} {name:{width}}" + (f"  - {note}" if note and not good else ""))
        print(f"\nwork mode: {mode_now} (recommended here: {recommended}); reviews and merges via: {review_path}")
        if fixes:
            print("\nFix:\n" + "\n".join(fixes))
    elif problems:
        print("Kit preflight: " + "; ".join(problems) + ". Run `just preflight` for fixes.")
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps({"date": dt.date.today().isoformat(), "head": git("rev-parse", "HEAD").strip(),
                               "work_mode": mode_now, "recommended": recommended, "review_path": review_path,
                               "problems": problems}, indent=1) + "\n")
    return 1 if problems else 0


def work_mode(argv: list) -> int:
    team = ROOT / ".agents" / "team.md"
    cur = team_setting("work_mode", "worktrees")
    if not argv:
        print(f"work mode: {cur}")
        print("  worktrees - developers run in parallel, each in its own git worktree (default)")
        print("  in-place  - one developer at a time in this checkout; for repos where worktrees fail")
        return 0
    new = argv[0]
    if new not in ("worktrees", "in-place"):
        sys.exit("kit: work mode must be worktrees or in-place")
    text = team.read_text()
    if re.search(r"^\|\s*work_mode\s*\|", text, re.M):
        text = re.sub(r"^(\|\s*work_mode\s*\|\s*)[^|]*?(\s*\|)", lambda m: m.group(1) + new + m.group(2), text, count=1, flags=re.M)
    else:
        text = re.sub(r"(## Parallelism.*?\n\| setting \| value \|\n\|---\|---\|\n)", lambda m: m.group(1) + f"| work_mode | {new} |\n",
                      text, count=1, flags=re.S)
    team.write_text(text)
    r = subprocess.run([sys.executable, str(ROOT / ".claude" / "kit" / "agents_sync.py")], cwd=ROOT)
    print(f"work mode: {cur} -> {new}. Start a new Claude Code session so agents reload.")
    return r.returncode

# ---------------------------------------------------------------- feature and bug records

FEATURE_DIR, BUG_DIR, BUG_DRAFTS = "docs/features", "docs/bugs", ".agents/bugs"
OPEN_BUG = ("reported", "triaged", "fixing", "")
ID_RE = re.compile(r"^[A-Za-z]+\d+$")


def parse_record(text: str) -> dict:
    rec = {}
    m = re.search(r"^#\s+([A-Za-z]+\d+)\s*[-:]\s*(.+?)\s*$", text, re.M)
    if m:
        rec["id"], rec["title"] = m.group(1).upper(), m.group(2)
    for key in ("Status", "Feature", "Severity", "Summary", "Reported", "Owner area", "PR", "Regression test"):
        m = re.search(rf"^{key}:[ \t]*(.*?)\s*$", text, re.M)
        if m:
            rec[key.lower().replace(" ", "_")] = m.group(1)
    return rec


def record_files(directory: str) -> list:
    d = ROOT / directory
    return sorted(p for p in d.glob("*.md") if p.name.lower() != "readme.md") if d.is_dir() else []


def records(directory: str) -> list:
    out = []
    for p in record_files(directory):
        rec = parse_record(p.read_text(errors="replace"))
        if rec.get("id"):
            rec["path"] = p
            out.append(rec)
    return out


def find_record(directory: str, rid: str):
    for rec in records(directory):
        if rec["id"].upper() == rid.upper():
            return rec
    return None


def section(text: str, heading: str, limit: int = 12) -> list:
    m = re.search(rf"^##\s+{re.escape(heading)}[^\n]*\n(.*?)(?=^##\s|\Z)", text, re.S | re.M)
    if not m:
        return []
    lines = [l for l in m.group(1).strip().splitlines()
             if l.strip() and not l.strip().startswith("<!--") and not re.match(r"^\s*-\s*(<[^>]*>)?\s*$", l)]
    return lines[:limit]


def bugs_for(fid: str) -> list:
    return [b for b in records(BUG_DIR) + records(BUG_DRAFTS) if b.get("feature", "").upper() == fid.upper()]


def agents_cmd(argv: list) -> int:
    d = ROOT / ".claude" / "agents"
    rows = []
    for p in sorted(d.glob("*.md")) if d.is_dir() else []:
        m = re.match(r"^---\n(.*?)\n---", p.read_text(errors="replace"), re.S)
        fm = dict(re.findall(r"^(\w+):[ \t]*(.*)$", m.group(1), re.M)) if m else {}
        rows.append((fm.get("name", p.stem), fm.get("model", "-"), fm.get("effort", "-"), fm.get("maxTurns", "-"),
                     fm.get("description", "").strip("\"' ")[:70], bool(m and fm.get("name"))))
    if not rows:
        print("No agents in .claude/agents/ - is the kit installed in this folder?")
        return 1
    try:
        main_agent = json.loads((ROOT / ".claude" / "settings.json").read_text()).get("agent", "")
    except Exception:  # noqa: BLE001
        main_agent = ""
    print(f"{'agent':20} {'model':20} {'effort':7} {'turns':6} role")
    for name, model, effort, turns, desc, ok in rows:
        print(f"{name:20} {model:20} {effort:7} {turns:6} {desc}" + ("" if ok else "  <- BROKEN frontmatter, agent will not load"))
    print(f"\n{len(rows)} agents. Main session runs as: {main_agent or '(none - plain Claude Code)'} (.claude/settings.json -> agent)")
    if shutil.which("claude"):
        try:
            r = subprocess.run(["claude", "plugin", "validate", str(d)], cwd=ROOT, capture_output=True, text=True,
                               timeout=30, stdin=subprocess.DEVNULL)
        except (subprocess.TimeoutExpired, OSError):
            r = subprocess.CompletedProcess([], 0, "", "unknown")
            r.returncode = 99
        if r.returncode == 0:
            print("claude plugin validate: agent files OK")
        elif "unknown" not in (r.stderr + r.stdout).lower():
            print("claude plugin validate:", (r.stdout + r.stderr).strip()[-400:])
    print("In Claude Code: the terminal header shows @" + (main_agent or "<agent>") + " when it is active; or ask \"who are you?\".")
    return 0


def features_cmd(argv: list) -> int:
    feats = records(FEATURE_DIR)
    if not feats:
        print("No feature records yet (docs/features/). New features get one in their first PR; ask the manager to backfill older ones.")
        return 0
    print(f"{'id':6} {'status':12} {'open bugs':9} title")
    for f in feats:
        open_bugs = sum(1 for b in bugs_for(f["id"]) if b.get("status", "").lower() in OPEN_BUG)
        print(f"{f['id']:6} {f.get('status', '-')[:12]:12} {open_bugs:<9} {f['title']}")
    print(f"\n{len(feats)} features. Details: just feature <id>")
    return 0


def commits_for(rid: str) -> list:
    base = base_ref()
    out = git("log", base, "--format=%h|%s|%cs", "-E", "-i",
              f"--grep=^Refs:.*\\b{rid}\\b", f"--grep=\\b{rid}\\b")
    return [l.split("|", 2) for l in out.splitlines() if l]


def feature_cmd(argv: list) -> int:
    if not argv:
        sys.exit("usage: kit.py feature <id>")
    rid = argv[0].upper()
    rec = find_record(FEATURE_DIR, rid)
    plan = ROOT / ".agents" / "plans" / f"{rid}.md"
    if not rec and not plan.exists():
        print(f"No feature {rid}: no docs/features/{rid}-*.md and no local plan. `just features` lists the known ones.")
        return 1
    if rec:
        text = rec["path"].read_text(errors="replace")
        print(f"# {rid} - {rec['title']}   [{rec.get('status', '-')}]   {rec['path'].relative_to(ROOT)}")
        if rec.get("summary"):
            print(rec["summary"])
        for heading in ("What was built", "Known limits", "Deferred"):
            lines = section(text, heading)
            if lines:
                print(f"\n## {heading}\n" + "\n".join(lines))
    else:
        print(f"# {rid}   [no committed record yet]   plan: {plan.relative_to(ROOT)}")
    commits = commits_for(rid)
    if commits:
        print(f"\n## Merged changes ({len(commits)})")
        for sha, subj, day in commits[:15]:
            print(f"- {day} {sha} {subj}")
        files = sorted({f for sha, _, _ in commits for f in git("show", "--name-only", "--format=", sha).split()})
        print(f"\n## Files touched ({len(files)})\n" + "\n".join(f"- {f}" for f in files[:30]) + ("\n- ..." if len(files) > 30 else ""))
        tags = git("tag", "--contains", commits[0][0], "--sort=v:refname").split()
        first_tag = git("tag", "--contains", commits[-1][0], "--sort=v:refname").split()
        print(f"\n## Releases\nfirst shipped in: {first_tag[0] if first_tag else 'not released yet'}; latest change in: {tags[0] if tags else 'not released yet'}")
    else:
        print("\n## Merged changes\nnone found on the integration branch yet")
    bugs = bugs_for(rid)
    if bugs:
        print("\n## Bugs")
        for b in bugs:
            where = "draft" if BUG_DRAFTS in str(b["path"]) else "committed"
            print(f"- {b['id']} [{b.get('severity', '-')}, {b.get('status', '-')}, {where}] {b['title']}")
    extra = [str(x.relative_to(ROOT)) for x in (plan, ROOT / ".agents" / "reviews" / rid) if x.exists()]
    if extra:
        print("\nLocal working files: " + ", ".join(extra))
    return 0


def slugify(s: str) -> str:
    return re.sub(r"[^a-z0-9]+", "-", s.lower()).strip("-")[:40] or "bug"


def bug_new(argv: list) -> int:
    if not argv or argv[0].startswith("--"):
        sys.exit('usage: kit.py bug-new "<title>" [--feature F7] [--severity high] [--reported-by name]')
    title = argv[0]
    opt = lambda k, d: argv[argv.index(k) + 1] if k in argv and argv.index(k) + 1 < len(argv) else d
    feature, severity, who = opt("--feature", "unknown").upper(), opt("--severity", "medium").lower(), opt("--reported-by", "user")
    if severity not in ("critical", "high", "medium", "low"):
        sys.exit("kit: --severity must be critical, high, medium or low")
    nums = [int(m.group(1)) for d in (BUG_DIR, BUG_DRAFTS) for p in record_files(d)
            for m in [re.match(r"B(\d+)", p.name, re.I)] if m]
    bid = f"B{max(nums, default=0) + 1}"
    tmpl = ROOT / ".agents" / "templates" / "bug-report.md"
    body = tmpl.read_text() if tmpl.exists() else "# {id} - {title}\n\nStatus: reported\nFeature: {feature}\nSeverity: {severity}\nReported: {date} by {who}\n"
    body = body.replace("{id}", bid).replace("{title}", title).replace("{feature}", feature).replace("{severity}", severity) \
               .replace("{date}", dt.date.today().isoformat()).replace("{who}", who)
    d = ROOT / BUG_DRAFTS
    d.mkdir(parents=True, exist_ok=True)
    path = d / f"{bid}-{slugify(title)}.md"
    path.write_text(body)
    print(f"{bid} drafted: {path.relative_to(ROOT)}")
    print(f"Next: fill 'What happens', 'Expected', 'How to reproduce'; fix on branch fix/{bid}-{slugify(title)[:30]};")
    print(f"the fix PR commits it as {BUG_DIR}/{path.name} with root cause and regression test.")
    return 0


def bugs_cmd(argv: list) -> int:
    show_all = "--all" in argv
    rows = [(b, "committed") for b in records(BUG_DIR)] + [(b, "draft") for b in records(BUG_DRAFTS)]
    committed_ids = {b["id"] for b, w in rows if w == "committed"}
    rows = [(b, w) for b, w in rows if not (w == "draft" and b["id"] in committed_ids)]
    if not show_all:
        rows = [(b, w) for b, w in rows if b.get("status", "").lower() in OPEN_BUG]
    if not rows:
        print("No open bugs." if not show_all else "No bug reports yet.")
        return 0
    order = {"critical": 0, "high": 1, "medium": 2, "low": 3}
    rows.sort(key=lambda r: (order.get(r[0].get("severity", "").lower(), 9), r[0]["id"]))
    print(f"{'id':6} {'severity':9} {'status':9} {'feature':8} title")
    for b, where in rows:
        print(f"{b['id']:6} {b.get('severity', '-'):9} {b.get('status', '-')[:9]:9} {b.get('feature', '-'):8} {b['title']}"
              + ("  (draft)" if where == "draft" else ""))
    return 0


def board_archive(argv: list) -> int:
    board = ROOT / ".agents" / "board.md"
    if not board.exists():
        print("no board yet")
        return 0
    lines = board.read_text().splitlines()
    header, keep, done = None, [], []
    for l in lines:
        cells = [c.strip().lower() for c in l.strip().strip("|").split("|")] if l.strip().startswith("|") else []
        if cells and "status" in cells and header is None:
            header = cells
            keep.append(l)
            continue
        if header and cells and not set("".join(cells)) <= set("-: ") and len(cells) == len(header):
            if dict(zip(header, cells)).get("status", "") in ("merged", "done", "closed", "released", "won't fix", "wont fix", "fixed"):
                done.append(l)
                continue
        keep.append(l)
    if not done:
        print("board: nothing finished to archive")
        return 0
    arch = ROOT / ".agents" / "board-archive" / f"{dt.date.today():%Y-%m}.md"
    arch.parent.mkdir(parents=True, exist_ok=True)
    head_lines = [l for l in lines if l.strip().startswith("|")][:2]
    prefix = "" if arch.exists() else f"# Board archive {dt.date.today():%Y-%m}\n\n" + "\n".join(head_lines) + "\n"
    with arch.open("a") as f:
        f.write(prefix + "\n".join(done) + "\n")
    board.write_text("\n".join(keep) + "\n")
    print(f"board: archived {len(done)} finished rows -> {arch.relative_to(ROOT)}")
    return 0


def record_checks(kind: str, fid: str, headref: str, baseref: str = "") -> list:
    """Feature PRs need docs/features/<id>-*.md on the PR head or already on the integration branch
    (another slice of the same feature may have added it); bug-fix PRs need docs/bugs/<id>-*.md with a regression test."""
    res = []
    if not fid or not ID_RE.match(fid):
        return res
    names = git("ls-tree", "-r", "--name-only", headref, "--", FEATURE_DIR, BUG_DIR).split()
    if baseref:
        names += git("ls-tree", "-r", "--name-only", baseref, "--", FEATURE_DIR).split()
    fid_u = fid.upper()
    has = lambda d: next((n for n in names if n.startswith(d + "/") and pathlib.Path(n).name.upper().startswith(fid_u + "-")), None)
    if fid_u.startswith("F") and kind in ("feat", "fix", "refactor", "perf") and team_setting("require_feature_record", "yes").lower() != "no":
        path = has(FEATURE_DIR)
        res.append(("feature record present", bool(path), f"add {FEATURE_DIR}/{fid_u}-<slug>.md from .agents/templates/feature-record.md in this PR"))
    if fid_u.startswith("B") and kind == "fix" and team_setting("require_bug_record", "yes").lower() != "no":
        path = has(BUG_DIR)
        ok_test = False
        if path:
            rec = parse_record(git("show", f"{headref}:{path}"))
            ok_test = bool(rec.get("regression_test", "").strip(" -<>"))
        res.append(("bug record with regression test", bool(path) and ok_test,
                    f"commit {BUG_DIR}/{fid_u}-<slug>.md (from the draft in {BUG_DRAFTS}/) with 'Regression test:' filled"))
    return res


# ---------------------------------------------------------------- merge gate

TITLE_RE = re.compile(r"^(feat|fix|perf|refactor|docs|test|chore|ci|build|style|revert)(\([^)]+\))?!?: .+")
OPEN = {"open", "not fixed", "disputed", ""}
CLEARED = ("reject", "defer", "accepted")


def findings(path: pathlib.Path) -> tuple:
    """Return (verdict, rows) from a findings file written with the kit templates."""
    text = path.read_text(errors="replace")
    m = re.search(r"^Verdict:\s*(.+)$", text, re.M)
    verdict = m.group(1).strip().upper() if m else ""
    rows, header = [], None
    for line in text.splitlines():
        if not line.strip().startswith("|"):
            header = None if header and not line.strip() else header
            continue
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if header is None:
            if cells and cells[0].lower() == "id":
                header = [c.lower() for c in cells]
            continue
        if set("".join(cells)) <= set("-: "):
            continue
        rows.append(dict(zip(header, cells)))
    return verdict, rows


def open_blockers(rows: list, severities: set) -> list:
    out = []
    for r in rows:
        sev = r.get("severity", "").lower()
        status = r.get("status", "").lower()
        decision = r.get("tech-lead decision", r.get("decision", "")).lower()
        if sev in severities and status in OPEN and not decision.startswith(CLEARED):
            out.append(r.get("id", "?"))
    return out


def review_ref(pr: str = "", branch: str = "") -> str:
    return f"pr-{pr}" if pr else re.sub(r"[^A-Za-z0-9._-]+", "-", branch).strip("-")


def review_dir(feature: str, ref: str) -> pathlib.Path:
    """One folder per PR (or local branch): .agents/reviews/<feature>/<pr-N|branch>/ - slices of one feature
    never overwrite each other's verdicts. Falls back to the old per-feature folder for work started before."""
    base = ROOT / ".agents" / "reviews" / feature
    d = base / ref if ref else base
    if ref and not d.exists() and (base / "code-review.md").exists():
        return base
    return d


def review_checks(feature: str, kind: str, required: list = None, ref: str = "") -> list:
    required = required if required is not None else (["code", "security"] + (["tests"] if kind in TEST_KINDS else []))
    res = []
    d = review_dir(feature, ref)
    if "light" in required:
        return [("light change (docs only): no reviewer needed; the tech lead reads the diff before merging", True, "")]
    cr = d / "code-review.md"
    if cr.exists():
        v, rows = findings(cr)
        bad = open_blockers(rows, {"blocker", "major"})
        res.append(("code review approved", v.startswith("APPROVE") and not bad, f"verdict {v or 'missing'}; open: {', '.join(bad) or 'none'}"))
    else:
        res.append(("code review approved", False, f"no {cr.relative_to(ROOT)}"))
    sec = d / "security.md"
    if "security" not in required:
        res.append(("security review (not required: small, no sensitive files)", True, ""))
    elif sec.exists():
        v, rows = findings(sec)
        bad = open_blockers(rows, {"critical", "high"})
        res.append(("security review passed", v.startswith("PASS") and not bad, f"verdict {v or 'missing'}; open: {', '.join(bad) or 'none'}"))
    else:
        res.append(("security review passed", False, f"no {sec.relative_to(ROOT)}"))
    for dr in domain_reviews():
        if dr["name"] not in required:
            continue
        f = d / review_file(dr["name"])
        ok_words = tuple(w.upper() for w in dr.get("pass", ["PASS", "APPROVE"]))
        if f.exists():
            v, rows = findings(f)
            bad = open_blockers(rows, set(dr.get("block_severities", ["blocker", "critical", "high"])))
            res.append((f"{dr['name']} review ({dr['agent']}) passed", v.startswith(ok_words) and not bad,
                        f"verdict {v or 'missing'}; open: {', '.join(bad) or 'none'}"))
        else:
            res.append((f"{dr['name']} review ({dr['agent']}) passed", False, f"no {f.relative_to(ROOT)}"))
    if "tests" in required:
        tests = sorted(d.glob("tests*.md"))
        signed = [p.name for p in tests if findings(p)[0].startswith("SIGN-OFF") and not open_blockers(findings(p)[1], {"blocker", "major", "critical", "high"})]
        res.append(("tester sign-off", bool(signed), ", ".join(signed) or "no tests-*.md with Verdict: SIGN-OFF"))
    return res


def pr_info(pr: str) -> dict:
    if not shutil.which("gh"):
        sys.exit("kit: gh is not installed; use --branch for local merges")
    out = subprocess.run(["gh", "pr", "view", pr, "--json",
                          "number,title,baseRefName,headRefName,isDraft,state,reviewDecision,statusCheckRollup"],
                         cwd=ROOT, capture_output=True, text=True)
    if out.returncode != 0:
        sys.exit(f"kit: cannot read PR {pr}: {out.stderr.strip()[:200]}")
    return json.loads(out.stdout)


def ci_state(info: dict) -> tuple:
    """(state, detail): state is passed | pending | failed | none (no checks reported yet)."""
    roll = info.get("statusCheckRollup") or []
    if not roll:
        return "none", "no CI checks reported yet"
    pending, failed = [], []
    for c in roll:
        name = c.get("name") or c.get("context") or "?"
        concl = (c.get("conclusion") or c.get("state") or "").upper()
        status = (c.get("status") or "").upper()
        if (status and status != "COMPLETED" and not concl) or concl in ("PENDING", "EXPECTED", "QUEUED", "IN_PROGRESS"):
            pending.append(name)
        elif concl not in ("SUCCESS", "NEUTRAL", "SKIPPED"):
            failed.append(f"{name}={concl or status}")
    if failed:
        return "failed", f"failed: {', '.join(failed)}" + (f"; running: {', '.join(pending)}" if pending else "")
    if pending:
        return "pending", f"running: {', '.join(pending)}"
    return "passed", "all passed"


PENDING = "pending"  # third result state: not failed, just not finished


def ci_check(info: dict) -> tuple:
    required = team_setting("require_ci_checks", "yes").lower() != "no"
    state, detail = ci_state(info)
    if state == "passed":
        return ("CI checks green", True, detail)
    if state == "none":
        # right after a push nothing is reported yet: pending, not failed (pr-wait times out if the repo has no CI)
        return ("CI checks green", PENDING if required else True, detail + ("" if required else " (not required)"))
    if state == "pending":
        return ("CI checks green", PENDING, detail)
    return ("CI checks green", False, detail)


def classify(head: str) -> tuple:
    m = re.match(r"^(feat|fix|test|ci|chore|refactor|perf|docs|build)/([A-Za-z]*\d+|[A-Za-z0-9]+)", head)
    return (m.group(1), m.group(2).upper() if re.match(r"^[A-Za-z]+\d+$", m.group(2)) else m.group(2)) if m else ("", "")


def merge_check(argv: list, quiet: bool = False) -> int:
    integ, rel = branches()
    feature = argv[argv.index("--feature") + 1] if "--feature" in argv else ""
    results = []
    if "--branch" in argv:
        head = argv[argv.index("--branch") + 1]
        kind, fid = classify(head)
        feature = feature or fid
        if not feature:
            sys.exit("kit: pass --feature <id> (branch name has no feature id)")
        req = review_plan(["--branch", head], show=False)[0]
        results += review_checks(feature, kind, req, review_ref(branch=head))
        results += record_checks(kind, feature, head, base_ref())
        if team_setting("require_ci_checks", "yes").lower() != "no":
            results.append(("quality gate passed at branch head", gate_passed(head),
                            f"no PR/CI here: run `just gate` on {head} with a clean tree (developers' gate hook does it)"))
        target = f"{integ} (local)"
    else:
        if not argv or argv[0].startswith("--"):
            sys.exit("usage: kit.py merge-check <pr-number> [--feature ID]")
        info = pr_info(argv[0])
        head, base = info["headRefName"], info["baseRefName"]
        target = base
        kind, fid = classify(head)
        feature = feature or fid
        results.append(("PR is open and not a draft", info.get("state") == "OPEN" and not info.get("isDraft"), f"state {info.get('state')}, draft {info.get('isDraft')}"))
        results.append(ci_check(info))
        if base == integ:
            if head == rel:
                results.append(("back-merge from release branch", True, f"{rel} -> {integ}"))
            else:
                results.append(("title is a Conventional Commit", bool(TITLE_RE.match(info["title"])), info["title"][:80]))
                if not feature:
                    results.append(("feature id known", False, "branch has no feature id; pass --feature <id>"))
                else:
                    b_ref, h_ref = pr_refs(info)
                    req = required_reviews(kind, diff_stats(b_ref, h_ref))[0]
                    results += review_checks(feature, kind, req, review_ref(pr=str(info["number"])))
                    results += record_checks(kind, feature, h_ref, b_ref)
        elif base == rel:
            allowed = head == integ or head.startswith("release-please--")
            results.append(("promotion or release PR into the release branch", allowed, f"{head} -> {base}"))
        else:
            results.append(("targets the integration branch", False, f"PR targets {base}; feature PRs must target {integ}"))
    failed = [r for r in results if r[1] is False]
    pending = [r for r in results if r[1] == PENDING]
    verdict = "FAIL" if failed else "PENDING" if pending else "PASS"
    if not quiet:
        print(f"merge-check {argv[0] if argv and not argv[0].startswith('--') else head} -> {target}: {verdict}"
              + ("  (only CI is unfinished: `just pr-wait <pr>` waits for free; don't re-check by hand)" if verdict == "PENDING" else ""))
        for name, good, detail in results:
            mark = "ok  " if good is True else "WAIT" if good == PENDING else "FAIL"
            print(f"  {mark} {name}" + (f" ({detail})" if detail and good is not True else ""))
    return 1 if failed else 2 if pending else 0


def merge(argv: list) -> int:
    integ, rel = branches()
    if "--branch" in argv:  # local fallback without GitHub
        head = argv[argv.index("--branch") + 1]
        title = argv[argv.index("--title") + 1] if "--title" in argv else ""
        if not TITLE_RE.match(title):
            sys.exit("kit: local merge needs --title '<conventional commit title>'")
        if merge_check(argv):
            return 1
        if head in (integ, rel):
            sys.exit(f"kit: {head} is a protected branch and can't be merged/deleted this way")
        rid = classify(head)[1] or (argv[argv.index("--feature") + 1] if "--feature" in argv else "")
        refs = ["-m", f"Refs: {rid}"] if rid else []  # links the commit to its feature/bug record (just feature <id>)
        for cmd in (["git", "switch", integ], ["git", "merge", "--squash", head], ["git", "commit", "-m", title, *refs], ["git", "branch", "-D", head]):
            if subprocess.run(cmd, cwd=ROOT).returncode:
                return 1
        print(f"merged {head} into {integ}")
        return 0
    info = pr_info(argv[0])
    if info["baseRefName"] == rel:
        sys.exit(f"kit: PR targets {rel}; promotions and releases go through `just promote {argv[0]}` (asks the user)")
    rc_check = merge_check(argv)
    if rc_check:
        return rc_check
    head = info["headRefName"]
    style = "--merge" if head == rel else "--squash"
    delete = [] if head in (integ, rel) else ["--delete-branch"]  # policy: never delete protected branches
    rid = classify(head)[1] or (argv[argv.index("--feature") + 1] if "--feature" in argv else "")
    body = ["--body", f"Refs: {rid}\n\nPR #{argv[0]} from {head}"] if rid and style == "--squash" else []
    rc = subprocess.run(["gh", "pr", "merge", argv[0], style, *delete, *body], cwd=ROOT).returncode
    if rc == 0:
        subprocess.run(["git", "fetch", "-q", "origin"], cwd=ROOT, capture_output=True)
    return rc


def promote(argv: list) -> int:
    integ, rel = branches()
    if not argv:
        sys.exit("usage: kit.py promote <pr-number>")
    info = pr_info(argv[0])
    if info["baseRefName"] != rel:
        sys.exit(f"kit: promote is only for PRs into {rel}")
    if merge_check(argv):
        return 1
    style = "--squash" if info["headRefName"].startswith("release-please--") else "--merge"  # keep feature commits for the changelog
    return subprocess.run(["gh", "pr", "merge", argv[0], style], cwd=ROOT).returncode


def new_branch(argv: list) -> int:
    if not argv:
        sys.exit("usage: kit.py new-branch <type/id-slug>")
    subprocess.run(["git", "fetch", "-q", "origin"], cwd=ROOT, capture_output=True)
    start = base_ref()
    if not ref_exists(start):
        integ, rel = branches()
        sys.exit(f"kit: neither {integ} nor {rel} exists here (team.md -> Branches). Run `just preflight`; "
                 f"existing branches: {', '.join(git('branch', '--format=%(refname:short)').split()[:8]) or 'none'}")
    dirty = [l for l in git("status", "--porcelain").splitlines() if l and not l[3:].startswith(".agents/")]
    if dirty:
        sys.exit("kit: uncommitted changes in this checkout - commit or stash them first (in-place mode shares the user's checkout):\n"
                 + "\n".join(dirty[:10]))
    rc = subprocess.run(["git", "switch", "-c", argv[0], start], cwd=ROOT).returncode
    if rc == 0:
        print(f"created {argv[0]} from {start}; open the PR with: gh pr create --base {branches()[0]}")
    return rc


def setup_branches() -> int:
    integ, rel = branches()
    if not ref_exists(integ):
        src = f"origin/{rel}" if ref_exists(f"origin/{rel}") else rel
        if subprocess.run(["git", "branch", integ, src], cwd=ROOT).returncode:
            return 1
        print(f"created local {integ} from {src}")
    if git("remote").strip():
        if not git("ls-remote", "--heads", "origin", integ).strip():
            if subprocess.run(["git", "push", "-u", "origin", integ], cwd=ROOT).returncode:
                return 1
            print(f"pushed {integ} to origin")
        else:
            print(f"origin/{integ} already exists")
    print(f"integration branch: {integ}; release branch: {rel}")
    return 0


def protect_branches(argv: list) -> int:
    integ, rel = branches()
    approvals = int(argv[argv.index("--approvals") + 1]) if "--approvals" in argv else 0
    wf = ROOT / ".github" / "workflows" / "ci.yml"
    contexts = []
    if wf.exists():
        m = re.search(r"^jobs:\s*\n(.*)", wf.read_text(), re.S | re.M)
        contexts = re.findall(r"^  ([A-Za-z0-9_-]+):\s*$", m.group(1), re.M) if m else []
    contexts = [c for c in contexts if c != "pr-title"] + (["pr-title"] if "pr-title" in contexts else [])
    def body(n):
        return {"required_status_checks": {"strict": True, "contexts": contexts},
                "enforce_admins": False,
                "required_pull_request_reviews": {"required_approving_review_count": n},
                "restrictions": None, "allow_force_pushes": False, "allow_deletions": False}  # never wipe these branches
    plans = [(rel, body(approvals)), (integ, body(0))]
    repo = subprocess.run(["gh", "repo", "view", "--json", "nameWithOwner", "-q", ".nameWithOwner"], cwd=ROOT,
                          capture_output=True, text=True).stdout.strip() if shutil.which("gh") else ""
    for branch, b in plans:
        print(f"{branch}: PR required, {b['required_pull_request_reviews']['required_approving_review_count']} approvals, "
              f"checks: {', '.join(contexts) or 'none found'}, no force-push/delete")
    if "--apply" not in argv:
        print("(dry run - re-run with --apply; solo repos keep 0 approvals because GitHub forbids approving your own PR)")
        return 0
    if not repo:
        sys.exit("kit: gh not installed/authenticated or no GitHub remote")
    for branch, b in plans:
        r = subprocess.run(["gh", "api", "-X", "PUT", f"repos/{repo}/branches/{branch}/protection", "--input", "-"],
                           input=json.dumps(b), text=True, capture_output=True, cwd=ROOT)
        print(f"{branch}: {'protected' if r.returncode == 0 else 'FAILED ' + r.stderr.strip()[:200]}")
    return 0


def review_pack(argv: list) -> int:
    req, why, base, headref, head, kind, fid, stats = review_plan(argv, show=False)
    feature = argv[argv.index("--feature") + 1] if "--feature" in argv else fid
    s = stack()
    max_lines = int(team_setting("review_pack_max_diff_lines", "1200"))
    total = sum(a + d for a, d, _ in stats)
    print(f"# Review pack: {head} -> {base}  (feature {feature or '?'}, {len(stats)} files, {total} lines)")
    print(f"Reviews needed: {', '.join(req)}  ({'; '.join(why)})")
    pr_no = argv[0] if argv and not argv[0].startswith("--") else ""
    ref = review_ref(pr=pr_no, branch=head)
    if feature:
        d_out = ROOT / ".agents" / "reviews" / feature / ref
        agents = {"code": "code-reviewer", "security": "security-reviewer", "tests": "tester", **{d["name"]: d["agent"] for d in domain_reviews()}}
        print("Write findings to exactly (merge-check reads these paths):")
        for r in req:
            if r in agents:
                print(f"  {agents[r]}: {d_out / review_file(r)}")
    plan = ROOT / ".agents" / "plans" / f"{feature}.md" if feature else None
    if plan and plan.exists():
        m = re.search(r"^## Acceptance criteria\s*\n(.*?)(?=^## |\Z)", plan.read_text(), re.S | re.M)
        if m:
            print("\n## Acceptance criteria\n" + "\n".join(m.group(1).strip().splitlines()[:25]))
    if feature:
        d = review_dir(feature, ref)
        prior = []
        for f in sorted(d.glob("*.md")) if d.exists() else []:
            v, rows = findings(f)
            open_ids = [r.get("id", "?") for r in rows if r.get("status", "").lower() in OPEN]
            prior.append(f"- {f.name}: verdict {v or '-'}; open: {', '.join(open_ids) or 'none'}")
        if prior:
            print("\n## Previous findings (re-review: update these in place)\n" + "\n".join(prior))
    print("\n## Files")
    for a, d, p in sorted(stats, key=lambda x: -(x[0] + x[1])):
        print(f"- {p} (+{a} -{d})")
    diff = git("diff", "-U3", f"{base}...{headref}")
    flags_rx = s.get("review_redflags")
    if flags_rx:
        rx, hits, path, line_no = re.compile(flags_rx), [], "", 0
        for l in diff.splitlines():
            if l.startswith("+++ b/"):
                path = l[6:]
            elif l.startswith("@@"):
                m = re.search(r"\+(\d+)", l)
                line_no = int(m.group(1)) - 1 if m else 0
            elif l.startswith("+") and not l.startswith("+++"):
                line_no += 1
                if rx.search(l):
                    hits.append(f"- {path}:{line_no}: {l[1:].strip()[:120]}")
            elif not l.startswith("-"):
                line_no += 1
        print("\n## Red-flag hits (added lines; justify each or file a finding)\n" + ("\n".join(hits[:40]) if hits else "none"))
    cm = ROOT / ".agents" / "codemap.md"
    if cm.exists():
        touched = {p for _, _, p in stats}
        lines = [l for l in cm.read_text().splitlines() if any(f"`{p}`" in l for p in touched)]
        if lines:
            print("\n## Codemap entries for touched files\n" + "\n".join(lines[:30]))
    dl = diff.splitlines()
    print(f"\n## Diff ({len(dl)} lines" + (f", first {max_lines} shown - open other parts with Read offset/limit" if len(dl) > max_lines else "") + ")")
    print("```diff")
    print("\n".join(dl[:max_lines]))
    print("```")
    return 0


def main(argv: list) -> int:
    if not argv or argv[0] in ("-h", "--help"):
        print(__doc__)
        return 0
    cmd, rest = argv[0], argv[1:]
    if cmd == "gate":
        return gate()
    if cmd == "test":
        return gate(only="test")
    if cmd == "check":
        return gate(extra=True)
    if cmd == "db-check":
        return db_check()
    if cmd == "test-check":
        return test_check(rest[0] if rest else None)
    if cmd == "codemap":
        return codemap(rest[0] if rest else None)
    if cmd == "base":
        print(branches()[0])
        return 0
    if cmd == "branches":
        integ, rel = branches()
        print(f"integration={integ} release={rel} base_ref={base_ref()}")
        return 0
    if cmd == "base-ref":
        print(base_ref())
        return 0
    if cmd == "new-branch":
        return new_branch(rest)
    if cmd == "merge-check":
        return merge_check(rest)
    if cmd == "review-plan":
        review_plan(rest)
        return 0
    if cmd == "review-pack":
        return review_pack(rest)
    if cmd == "merge":
        return merge(rest)
    if cmd == "promote":
        return promote(rest)
    if cmd == "setup-branches":
        return setup_branches()
    if cmd == "docs":
        return docs_cmd(rest)
    if cmd == "start-report":
        return start_report(rest)
    if cmd == "envelope":
        return envelope(rest)
    if cmd == "triage":
        return triage(rest)
    if cmd == "status":
        return status_cmd(rest)
    if cmd == "pr-wait":
        return pr_wait(rest)
    if cmd == "ship":
        return ship(rest)
    if cmd == "preflight":
        return preflight(rest)
    if cmd == "work-mode":
        return work_mode(rest)
    if cmd == "agents":
        return agents_cmd(rest)
    if cmd == "features":
        return features_cmd(rest)
    if cmd == "feature":
        return feature_cmd(rest)
    if cmd == "bug-new":
        return bug_new(rest)
    if cmd == "bugs":
        return bugs_cmd(rest)
    if cmd == "board-archive":
        return board_archive(rest)
    if cmd == "protect-branches":
        return protect_branches(rest)
    sys.exit(f"kit: unknown command {cmd!r}")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
