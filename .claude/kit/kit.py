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
  kit.py merge-check <pr> --wait [--timeout N]   poll CI in the script (pending, not failure) before checking
  kit.py merge <pr>           tech lead: merge-check, then squash-merge into the integration branch
  kit.py ship <pr> | --branch <b> --title T   tech lead: wait for CI, merge-check, merge, pull the integration branch
  kit.py status               preflight line, open board rows, merge-check line per open PR
  kit.py promote <pr>         user-approved: merge a promotion/release PR into the release branch
  kit.py review-plan <pr> | --branch <b> | --files a,b [--feature ID] [--kind K]   reviews needed + output path of each
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


LIGHT_GLOBS = [".agents/**", "docs/**", "**/*.md", "*.md"]
REVIEW_FILES = {"code": "code-review.md", "security": "security.md", "tests": "tests-<area>.md"}


def required_reviews(kind: str, stats: list) -> tuple:
    s = stack()
    light = team_setting("light_lane", "skip").lower()
    if light != "off" and stats and all(matches(p, LIGHT_GLOBS) for _, _, p in stats):
        # review effort scales with the risk of the paths touched: records/docs/config-of-agents only
        return (["code"] if light == "quick" else []), [f"light lane: only .agents/docs/markdown ({light})"]
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
    req, why = ["code"], [f"{total} changed lines ({src_lines} production)"]
    if sec_policy == "always" or sens or total >= large:
        req.append("security")
        why.append("security: " + ("always" if sec_policy == "always" else f"sensitive files {', '.join(sens[:4])}" if sens else f">= {large} lines"))
    if kind in TEST_KINDS and (test_policy == "always" or (test_policy == "behaviour-change" and src_lines >= tester_min)
                               or (test_policy == "large" and total >= large)):
        req.append("tests")
        why.append(f"tests: {test_policy}")
    return req, why


def current_branch() -> str:
    return git("rev-parse", "--abbrev-ref", "HEAD").strip()


def review_paths(req: list, fid: str) -> list:
    d = f".agents/reviews/{fid or '<feature-id>'}"
    return [f"  {r} -> {d}/{REVIEW_FILES[r]}" for r in req]


def review_plan(argv: list, show: bool = True) -> tuple:
    integ, _ = branches()
    if "--files" in argv:
        # plan before dispatch: reviews implied by the planned file list (~10 lines per file; sizes unknown)
        files = [f for f in argv[argv.index("--files") + 1].split(",") if f]
        kind = argv[argv.index("--kind") + 1] if "--kind" in argv else "feat"
        fid = argv[argv.index("--feature") + 1] if "--feature" in argv else ""
        stats = [(10, 0, f) for f in files]
        req, why = required_reviews(kind, stats)
        if show:
            print(f"reviews needed for {len(files)} planned files: {', '.join(req) or 'none'}  ({'; '.join(why)})")
            print("\n".join(review_paths(req, fid)))
        return req, why, "", "", "", kind, fid, stats
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
            print(f"reviews needed for {rng}: {', '.join(req) or 'none'}  ({'; '.join(why)})")
            print("\n".join(review_paths(req, fid)))
        return req, why, base, headref, head, kind, fid, stats
    if "--branch" in argv:
        head = argv[argv.index("--branch") + 1]
        base, headref = base_ref(), head
    else:
        info = pr_info(argv[0])
        head = info["headRefName"]
        base, headref = pr_refs(info)
    kind, fid = classify(head)
    if "--feature" in argv:
        fid = argv[argv.index("--feature") + 1]
    stats = diff_stats(base, headref)
    req, why = required_reviews(kind, stats)
    if show:
        print(f"reviews needed for {head}: {', '.join(req) or 'none'}  ({'; '.join(why)})")
        print("\n".join(review_paths(req, fid)))
    return req, why, base, headref, head, kind, fid, stats

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
    cur = current_branch() if is_repo else ""
    main_checkout = git("rev-parse", "--git-dir").strip() == git("rev-parse", "--git-common-dir").strip()
    if (mode_now == "worktrees" and main_checkout and cur and cur not in (integ, "HEAD")
            and git("worktree", "list", "--porcelain").count("worktree ") > 1):
        problems.append(f"main checkout is on '{cur}', not '{integ}' (feature branches belong to agent worktrees)")
        fixes.append(f"- main checkout: `git switch {integ}`")
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


def find_review(feature: str, name: str, refs: list) -> pathlib.Path:
    """Expected path first; else any review file of that kind whose header names the same branch/PR."""
    d = ROOT / ".agents" / "reviews" / feature
    hit = sorted(d.glob(name.replace("<area>", "*"))) if "<area>" in name else ([d / name] if (d / name).exists() else [])
    if hit or not refs:
        return hit
    stem = name.split(".")[0].replace("-<area>", "")
    out = []
    for f in sorted((ROOT / ".agents" / "reviews").glob(f"*/{stem}*.md")):
        head = "\n".join(f.read_text(errors="replace").splitlines()[:15])
        if any(re.search(rf"(?<![\w/-]){re.escape(r)}(?![\w-])", head) for r in refs):
            out.append(f)
    return out


def review_checks(feature: str, kind: str, required: list = None, refs: list = None) -> list:
    required = ["code", "security"] + (["tests"] if kind in TEST_KINDS else []) if required is None else required
    refs = refs or []
    res = []
    rel = lambda n: f".agents/reviews/{feature}/{n}"  # noqa: E731
    if "code" not in required:
        res.append(("code review (not required: light lane)", True, ""))
    else:
        cr = find_review(feature, "code-review.md", refs)
        if cr:
            v, rows = findings(cr[0])
            bad = open_blockers(rows, {"blocker", "major"})
            res.append(("code review approved", v.startswith("APPROVE") and not bad, f"verdict {v or 'missing'}; open: {', '.join(bad) or 'none'}"))
        else:
            res.append(("code review approved", False, f"missing {rel('code-review.md')}"))
    if "security" not in required:
        res.append(("security review (not required: small, no sensitive files)", True, ""))
    else:
        sec = find_review(feature, "security.md", refs)
        if sec:
            v, rows = findings(sec[0])
            bad = open_blockers(rows, {"critical", "high"})
            res.append(("security review passed", v.startswith("PASS") and not bad, f"verdict {v or 'missing'}; open: {', '.join(bad) or 'none'}"))
        else:
            res.append(("security review passed", False, f"missing {rel('security.md')}"))
    if "tests" in required:
        tests = find_review(feature, "tests-<area>.md", refs)
        signed = [p.name for p in tests if findings(p)[0].startswith("SIGN-OFF") and not open_blockers(findings(p)[1], {"blocker", "major", "critical", "high"})]
        res.append(("tester sign-off", bool(signed), ", ".join(signed) or f"missing {rel('tests-<area>.md')} with Verdict: SIGN-OFF"))
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


def ci_check(info: dict) -> tuple:
    roll = info.get("statusCheckRollup") or []
    if not roll:
        required = team_setting("require_ci_checks", "yes").lower() != "no"
        return ("CI checks green", not required, "no CI checks reported" + ("" if required else " (not required)"))
    pending, failed = [], []
    for c in roll:
        name = c.get("name") or c.get("context") or "?"
        concl = (c.get("conclusion") or c.get("state") or "").upper()
        status = (c.get("status") or "").upper()
        if status and status != "COMPLETED" and not concl:
            pending.append(name)
        elif concl not in ("SUCCESS", "NEUTRAL", "SKIPPED"):
            failed.append(f"{name}={concl or status}")
    ok_ = not pending and not failed
    return ("CI checks green", ok_, "all passed" if ok_ else f"failed: {', '.join(failed) or '-'}; running: {', '.join(pending) or '-'}")


def ci_pending(info: dict) -> bool:
    roll = info.get("statusCheckRollup") or []
    if not roll:
        return team_setting("require_ci_checks", "yes").lower() != "no"  # not reported yet
    return any((c.get("status") or "").upper() not in ("", "COMPLETED") and not c.get("conclusion") for c in roll)


def wait_ci(pr: str, argv: list) -> None:
    """Poll in the script (free) until CI finished; 'no checks yet' is pending, but only for a grace period."""
    import time
    timeout = int(argv[argv.index("--timeout") + 1]) if "--timeout" in argv else 1200
    start, grace, said = time.time(), 120, False
    while True:
        info = pr_info(pr)
        age = time.time() - start
        if not ci_pending(info) or age >= timeout or (not info.get("statusCheckRollup") and age >= grace):
            if said:
                print(f"CI wait over after {int(age)}s")
            return
        if not said:
            print(f"waiting for CI on PR {pr} (timeout {timeout}s)...", flush=True)
            said = True
        time.sleep(15)


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
        if not feature and required_reviews(kind, diff_stats(base_ref(), head))[0]:
            sys.exit("kit: pass --feature <id> (branch name has no feature id)")
        feature = feature or "light"
        req = review_plan(["--branch", head], show=False)[0]
        results += review_checks(feature, kind, req, [head])
        results += record_checks(kind, feature, head, base_ref())
        if team_setting("require_ci_checks", "yes").lower() != "no":
            results.append(("quality gate passed at branch head", gate_passed(head),
                            f"no PR/CI here: run `just gate` on {head} with a clean tree (developers' gate hook does it)"))
        target = f"{integ} (local)"
    else:
        if not argv or argv[0].startswith("--"):
            sys.exit("usage: kit.py merge-check <pr-number> [--feature ID]")
        if "--wait" in argv:
            wait_ci(argv[0], argv)
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
                b_ref, h_ref = pr_refs(info)
                req = required_reviews(kind, diff_stats(b_ref, h_ref))[0]
                if not feature and req:
                    results.append(("feature id known", False, "branch has no feature id; pass --feature <id>"))
                else:
                    feature = feature or "light"
                    results += review_checks(feature, kind, req, [head, f"#{argv[0]}", f"PR {argv[0]}"])
                    results += record_checks(kind, feature, h_ref, b_ref)
        elif base == rel:
            allowed = head == integ or head.startswith("release-please--")
            results.append(("promotion or release PR into the release branch", allowed, f"{head} -> {base}"))
        else:
            results.append(("targets the integration branch", False, f"PR targets {base}; feature PRs must target {integ}"))
    ok_all = all(r[1] for r in results)
    if not quiet:
        print(f"merge-check {argv[0] if argv and not argv[0].startswith('--') else head} -> {target}: {'PASS' if ok_all else 'FAIL'}")
        for name, good, detail in results:
            print(f"  {'ok  ' if good else 'FAIL'} {name}" + (f" ({detail})" if detail and not good else ""))
    return 0 if ok_all else 1


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
    if merge_check(argv):
        return 1
    head = info["headRefName"]
    style = "--merge" if head == rel else "--squash"
    delete = [] if head in (integ, rel) else ["--delete-branch"]  # policy: never delete protected branches
    rid = classify(head)[1] or (argv[argv.index("--feature") + 1] if "--feature" in argv else "")
    body = ["--body", f"Refs: {rid}\n\nPR #{argv[0]} from {head}"] if rid and style == "--squash" else []
    rc = subprocess.run(["gh", "pr", "merge", argv[0], style, *delete, *body], cwd=ROOT).returncode
    if rc == 0:
        subprocess.run(["git", "fetch", "-q", "origin"], cwd=ROOT, capture_output=True)
    return rc


def ship(argv: list) -> int:
    """One command for the mechanical tail: wait for CI, merge-check, merge, pull the integration branch."""
    if "--branch" in argv:
        return merge(argv)
    if not argv or argv[0].startswith("--"):
        sys.exit("usage: kit.py ship <pr> [--timeout N] | --branch <b> --title T")
    integ = branches()[0]
    wait_ci(argv[0], argv)
    rc = merge(argv)
    if rc:
        return rc
    r = subprocess.run(["git", "switch", "-q", integ], cwd=ROOT, capture_output=True, text=True)
    if r.returncode == 0:
        r = subprocess.run(["git", "pull", "-q", "--ff-only"], cwd=ROOT, capture_output=True, text=True)
    print(f"shipped PR {argv[0]}; {integ} " + ("updated" if r.returncode == 0 else f"not updated ({r.stderr.strip()[:120]})"))
    return 0


def status(argv: list) -> int:
    import io
    import contextlib
    integ = branches()[0]
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        preflight(["--quiet", "--cached"])
    msg = buf.getvalue().strip().removeprefix("Kit preflight: ").split(". Run `just")[0]
    print(f"preflight: {msg or 'ok'} (mode {team_setting('work_mode', 'worktrees')}, base {integ})")
    rows = []
    board = ROOT / ".agents" / "board.md"
    header = None
    for l in board.read_text().splitlines() if board.exists() else []:
        if not l.strip().startswith("|"):
            continue
        cells = [c.strip() for c in l.strip().strip("|").split("|")]
        if header is None:
            header = [c.lower() for c in cells] if "status" in [c.lower() for c in cells] else None
            continue
        if set("".join(cells)) <= set("-: ") or len(cells) != len(header):
            continue
        if dict(zip(header, cells)).get("status", "").lower() not in ("merged", "done", "closed", "released", "deferred", "won't fix", "wont fix", "fixed", "dropped"):
            rows.append(" | ".join(c for c in cells if c))
    print("board: " + ("none open" if not rows else f"{len(rows)} open"))
    for r in rows:
        print("  " + r)
    prs = []
    if shutil.which("gh"):
        out = subprocess.run(["gh", "pr", "list", "--json", "number,headRefName", "--limit", "20"], cwd=ROOT, capture_output=True, text=True)
        prs = json.loads(out.stdout) if out.returncode == 0 and out.stdout.strip() else []
    print("prs: " + ("none open" if not prs else f"{len(prs)} open"))
    for pr in prs:
        buf = io.StringIO()
        try:
            with contextlib.redirect_stdout(buf):
                rc = merge_check([str(pr["number"])])
        except SystemExit as e:
            buf.write(f"merge-check error: {e}")
            rc = 1
        lines = buf.getvalue().splitlines()
        bad = [l.strip()[5:] for l in lines[1:] if l.strip().startswith("FAIL")]
        print(f"  #{pr['number']} {pr['headRefName']}: {'PASS' if rc == 0 else 'FAIL - ' + '; '.join(bad or lines[:1])[:300]}")
    return 0


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
    if feature:
        print(f"Findings dir: {ROOT / '.agents' / 'reviews' / feature}/  (code-review.md, security.md, tests-<area>.md)")
    plan = ROOT / ".agents" / "plans" / f"{feature}.md" if feature else None
    if plan and plan.exists():
        m = re.search(r"^## Acceptance criteria\s*\n(.*?)(?=^## |\Z)", plan.read_text(), re.S | re.M)
        if m:
            print("\n## Acceptance criteria\n" + "\n".join(m.group(1).strip().splitlines()[:25]))
    if feature:
        d = ROOT / ".agents" / "reviews" / feature
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
    if cmd == "ship":
        return ship(rest)
    if cmd == "status":
        return status(rest)
    if cmd == "promote":
        return promote(rest)
    if cmd == "setup-branches":
        return setup_branches()
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
