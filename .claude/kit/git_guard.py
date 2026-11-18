#!/usr/bin/env python3
"""Git guard for PreToolUse (Bash) hooks. Reads the hook JSON on stdin; exit 2 = blocked (reason on stderr).

    git_guard.py branch            all agents: protected branches change only through checked PRs
    git_guard.py dev [--for NAME]  developers/testers/cicd: additionally no merging, promoting or branch setup
    git_guard.py manager [--for manager]  keeps raw diffs, lockfiles and generated files out of the manager's
                                   conversation (it is re-sent on every step); summaries come from kit commands

The command is parsed (quotes, `&&`, `;`, `|`, `cd`, `bash -c`, `git -C`), so words inside strings, grep patterns,
commit messages or PR bodies never trigger the guard - only commands that would actually run.
"""
import json
import os
import pathlib
import re
import shlex
import subprocess
import sys

SEPARATORS = {"&&", "||", ";", "|", "&", "|&", "\n", "(", ")", "{", "}", "!", "then", "do", "else", "elif", "fi", "done"}
WRAPPERS = {"sudo", "command", "time", "nice", "nohup", "exec", "env", "xargs", "timeout", "builtin"}
SHELLS = {"bash", "sh", "zsh", "dash"}
GIT_OPTS_WITH_ARG = {"-C", "-c", "--git-dir", "--work-tree", "--namespace", "--exec-path", "--config-env"}
JUST_OPTS_WITH_ARG = {"-f", "--justfile", "-d", "--working-directory", "--set", "--dotenv-path", "--dotenv-filename",
                      "--shell", "--shell-arg", "--chooser", "--color", "--dump-format", "--list-heading", "--list-prefix",
                      "--command", "-c", "--unstable"}
LEAD_ONLY = {"merge", "ship", "promote", "setup-branches", "protect-branches"}


class Blocked(Exception):
    pass


def tokens(cmd: str) -> list:
    """Shell-like split that keeps quoted text as one word and turns operators/newlines into separators."""
    cmd = strip_heredocs(cmd.replace("\\\n", " ")).replace("\n", " ; ")
    lex = shlex.shlex(cmd, posix=True, punctuation_chars=";&|()")
    lex.whitespace_split = True
    lex.commenters = "#"
    out = []
    try:
        for t in lex:
            out.append(t)
    except ValueError:  # unbalanced quotes: fall back to a plain split, still separator-aware
        out = re.findall(r"[;&|()\n]+|[^\s;&|()]+", cmd)
    return out


def strip_heredocs(cmd: str) -> str:
    """Drop here-document bodies (commit messages, PR bodies): they are data, not commands."""
    lines, out, end = cmd.split("\n"), [], None
    for line in lines:
        if end is not None:
            if line.strip() == end:
                end = None
            continue
        out.append(line)
        m = re.search(r"<<-?\s*['\"]?([A-Za-z_][A-Za-z0-9_]*)['\"]?", line)
        if m:
            end = m.group(1)
    return "\n".join(out)


def simple_commands(cmd: str, depth: int = 0) -> list:
    """List of argv lists, with `bash -c '<script>'` / `sh -c` / `eval` expanded in place."""
    out, cur = [], []
    for t in tokens(cmd) + [";"]:
        if t in SEPARATORS or set(t) <= set(";&|()\n"):
            if cur:
                out.append(cur)
            cur = []
            continue
        cur.append(t)
    expanded = []
    for argv in out:
        argv = strip_wrappers(argv)
        if not argv:
            continue
        name = os.path.basename(argv[0])
        if depth < 3 and name in SHELLS and "-c" in argv[1:] and argv.index("-c") + 1 < len(argv):
            expanded += simple_commands(argv[argv.index("-c") + 1], depth + 1)
        elif depth < 3 and name == "eval":
            expanded += simple_commands(" ".join(argv[1:]), depth + 1)
        else:
            expanded.append(argv)
    return expanded


def strip_wrappers(argv: list) -> list:
    i = 0
    while i < len(argv):
        a = argv[i]
        if re.match(r"^[A-Za-z_][A-Za-z0-9_]*=", a):  # FOO=bar cmd
            i += 1
        elif os.path.basename(a) in WRAPPERS:
            i += 1
            while i < len(argv) and argv[i].startswith("-"):
                i += 2 if argv[i] in ("-u", "-n", "-s", "-k", "-I") else 1
            if os.path.basename(a) == "timeout" and i < len(argv) and re.match(r"^\d", argv[i]):
                i += 1
        else:
            break
    return argv[i:]


def git_parts(argv: list) -> tuple:
    """('push', [args...], cwd_override or None) for a git argv."""
    i, cwd = 1, None
    while i < len(argv) and argv[i].startswith("-"):
        opt = argv[i].split("=", 1)[0]
        if opt in GIT_OPTS_WITH_ARG and "=" not in argv[i]:
            if opt == "-C" and i + 1 < len(argv):
                cwd = argv[i + 1]
            i += 2
        else:
            i += 1
    return (argv[i] if i < len(argv) else "", argv[i + 1:], cwd)


def current_branch(cwd: str) -> str:
    try:
        return subprocess.run(["git", "rev-parse", "--abbrev-ref", "HEAD"], cwd=cwd or None, capture_output=True,
                              text=True, timeout=10).stdout.strip()
    except Exception:  # noqa: BLE001
        return ""


def short(ref: str) -> str:
    ref = ref.lstrip("+")
    for p in ("refs/heads/", "refs/remotes/origin/", "origin/", "heads/"):
        if ref.startswith(p):
            return ref[len(p):]
    return ref


class Guard:
    def __init__(self, mode: str, root: pathlib.Path, cwd: str, protected: tuple):
        self.mode, self.root, self.cwd = mode, root, cwd
        self.integ, self.rel = protected
        self.prot = {self.integ, self.rel} | ({"main", "master"} if mode == "dev" else set())
        self.pr_merges = []
        self.switched = {}  # cwd -> branch after a `git switch X` earlier in the same command line

    def branch_in(self, cwd: str) -> str:
        return self.switched.get(os.path.abspath(cwd or ".")) or current_branch(cwd)

    def wipe(self):
        raise Blocked(f"never delete, rename, force-move or reset {self.integ}/{self.rel} (policy). "
                      "Ask the user if you believe this is needed.")

    def check(self, argv: list):
        name = os.path.basename(argv[0])
        if name == "cd" and len(argv) > 1:
            target = os.path.expanduser(argv[1])
            self.cwd = target if os.path.isabs(target) else os.path.normpath(os.path.join(self.cwd or ".", target))
            return
        if name == "git":
            return self.check_git(argv)
        if name == "gh":
            return self.check_gh(argv)
        if self.mode == "dev":
            self.check_lead_only(argv)

    def check_git(self, argv: list):
        sub, args, cwd = git_parts(argv)
        cwd = os.path.join(self.cwd or ".", cwd) if cwd and not os.path.isabs(cwd) else (cwd or self.cwd)
        pos = [a for a in args if not a.startswith("-")]
        flags = [a for a in args if a.startswith("-")]
        if sub == "push":
            if any(f in ("--force", "-f") or (re.match(r"^-[a-zA-Z]*f", f) and not f.startswith("--")) for f in flags):
                raise Blocked("force push (use --force-with-lease on your own work branch)")
            if any(f.split("=")[0] in ("--mirror", "--prune") for f in flags):
                raise Blocked("mirror/prune pushes can delete remote branches")
            refspecs = pos[1:]
            deleting = any(f in ("--delete", "-d") for f in flags)
            for r in refspecs:
                src, _, dst = r.partition(":")
                target = short(dst or src)
                if src == "HEAD" and not dst:
                    target = self.branch_in(cwd)
                if target in self.prot:
                    if deleting or r.startswith("+") or (":" in r and not src):
                        self.wipe()
                    raise Blocked(f"pushing to {target}. Push your work branch and open a PR to {self.integ}")
            if not refspecs and self.branch_in(cwd) in self.prot:
                raise Blocked(f"pushing {self.branch_in(cwd)} directly. Create a work branch and open a PR")
        elif sub == "branch":
            if any(f.split("=")[0] in ("--delete", "--move", "--force", "--copy") or
                   (re.match(r"^-[a-zA-Z]+$", f) and set(f[1:]) & set("dDmMfcC")) for f in flags):
                if any(short(p) in self.prot for p in pos):
                    self.wipe()
        elif sub == "update-ref":
            if any(short(p) in self.prot for p in pos):
                self.wipe()
        elif sub in ("checkout", "switch"):
            if pos and "--" not in args and not any(f in ("-b", "-c", "--create", "--detach", "--orphan") for f in flags):
                self.switched[os.path.abspath(cwd or ".")] = short(pos[0])
            if any(f in ("-B", "-C", "--force-create") for f in flags):
                i = next(i for i, a in enumerate(args) if a in ("-B", "-C", "--force-create"))
                if i + 1 < len(args) and short(args[i + 1]) in self.prot:
                    self.wipe()
        if sub in ("commit", "merge", "rebase", "cherry-pick", "am", "revert", "reset"):
            if sub == "merge" and any(f in ("--abort", "--quit") for f in flags):
                return
            if sub == "rebase" and any(f in ("--abort", "--quit") for f in flags):
                return
            cur = self.branch_in(cwd)
            if cur in self.prot:
                raise Blocked(f"changing {cur} directly. Create a work branch (just new-branch <type/id-slug>) and open a PR")

    def check_gh(self, argv: list):
        a = argv[1:]
        if a[:1] == ["repo"] and len(a) > 1 and a[1] in ("delete", "archive", "rename"):
            raise Blocked("deleting, archiving or renaming the repository")
        if a[:1] == ["api"]:
            text = " ".join(a)
            method = ""
            for i, x in enumerate(a):
                if x in ("-X", "--method") and i + 1 < len(a):
                    method = a[i + 1].upper()
                elif x.startswith(("-X", "--method=")) and len(x) > 2:
                    method = x.split("=", 1)[-1].lstrip("-X").upper()
            forced = any(x in ("force=true",) for x in a)
            hits = [p for p in self.prot if re.search(rf"(heads|branches)/{re.escape(p)}(\b|$)", text)]
            if hits and (method in ("DELETE", "PATCH") or forced):
                self.wipe()
        if a[:2] == ["pr", "merge"]:
            if self.mode == "dev":
                raise Blocked("merging PRs (only the tech lead merges, with `just ship <pr>`)")
            nums = [x.lstrip("#") for x in a[2:] if re.fullmatch(r"#?\d+", x)]
            if not nums:
                raise Blocked("gh pr merge needs an explicit PR number so it can be checked. Use: just ship <pr>")
            self.pr_merges.append(nums[0])

    def check_lead_only(self, argv: list):
        name = os.path.basename(argv[0])
        recipe = ""
        if name == "just":
            i = 1
            while i < len(argv) and argv[i].startswith("-"):
                i += 2 if argv[i].split("=")[0] in JUST_OPTS_WITH_ARG and "=" not in argv[i] else 1
            recipe = argv[i] if i < len(argv) else ""
        elif name.startswith("python") or name == "kit.py" or argv[0].endswith("kit.py"):
            idx = next((i for i, x in enumerate(argv) if x.endswith("kit.py")), None)
            if idx is None and "-m" in argv:
                idx = argv.index("-m") + 1
            recipe = argv[idx + 1] if idx is not None and idx + 1 < len(argv) else ""
        if recipe in LEAD_ONLY:
            raise Blocked("merging, promoting or changing branch setup (tech lead / user only)")


BULKY = re.compile(r"(^|/)(Cargo\.lock|package-lock\.json|pnpm-lock\.yaml|yarn\.lock|poetry\.lock|uv\.lock|go\.sum|"
                   r"Gemfile\.lock|composer\.lock|[^/]+\.min\.(js|css)|[^/]+\.map|[^/]+\.snap)$|(^|/)(generated|dist|target)/")
SUMMARY_FLAGS = {"--stat", "--shortstat", "--numstat", "--name-only", "--name-status", "--dirstat", "--summary", "--quiet",
                 "--exit-code", "--check"}
MANAGER_HINT = ("the manager's conversation is re-sent on every step, so raw diffs and lock/generated files stay out of it. "
                "Use `git diff --stat`, `just review-plan <pr>` (sizes and needed reviews), `just status`, or let the "
                "reviewers read the diff")


def check_manager(argv: list):
    name = os.path.basename(argv[0])
    if name == "git":
        sub, args, _ = git_parts(argv)
        flags = {a.split("=")[0] for a in args if a.startswith("-")}
        if flags & SUMMARY_FLAGS:
            return
        refs = [a for a in args if not a.startswith("-")]
        if sub == "show" and refs and all(":" in r for r in refs):  # git show <ref>:<path> reads one file
            if any(BULKY.search(r.split(":", 1)[1]) for r in refs):
                raise Blocked(f"reading a lock/generated file in the manager - {MANAGER_HINT}")
            return
        if sub in ("diff", "show") or (sub == "log" and flags & {"-p", "--patch", "-u"}):
            raise Blocked(f"`git {sub}` with full patches in the manager - {MANAGER_HINT}")
    if name == "gh" and argv[1:3] == ["pr", "diff"] and "--name-only" not in argv:
        raise Blocked(f"`gh pr diff` in the manager - {MANAGER_HINT}")
    if name in ("cat", "less", "more", "head", "tail", "bat") and any(BULKY.search(a) for a in argv[1:] if not a.startswith("-")):
        raise Blocked(f"reading a lock/generated file in the manager - {MANAGER_HINT}")


def kit_module(root: pathlib.Path):
    import importlib.util
    os.environ.setdefault("KIT_ROOT", str(root))
    spec = importlib.util.spec_from_file_location("kit_engine", root / ".claude" / "kit" / "kit.py")
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def main(argv: list) -> int:
    mode = argv[0] if argv else "branch"
    for_agent = argv[argv.index("--for") + 1] if "--for" in argv and argv.index("--for") + 1 < len(argv) else ""
    try:
        data = json.loads(sys.stdin.read() or "{}")
    except ValueError:
        return 0
    caller = data.get("agent_type") or ""
    if for_agent and caller and caller != for_agent:  # agent-attached hooks can fire for other agents' calls
        return 0
    if mode == "manager" and data.get("tool_name") == "Read":
        path = (data.get("tool_input") or {}).get("file_path") or ""
        if BULKY.search(path):
            print(f"Blocked by manager guard: reading {os.path.basename(path)} - {MANAGER_HINT}.", file=sys.stderr)
            return 2
        return 0
    cmd = (data.get("tool_input") or {}).get("command") or ""
    if mode == "manager":
        try:
            for c in simple_commands(cmd):
                check_manager(c)
        except Blocked as e:
            print(f"Blocked by manager guard: {e}.", file=sys.stderr)
            return 2
        return 0
    words = r"git|gh|bash|sh|zsh|eval" + (r"|just|kit\.py" if mode == "dev" else "")
    if not re.search(rf"\b({words})\b", cmd):  # fast exit for the many commands that can't touch branches
        return 0
    cwd = data.get("cwd") or os.getcwd()
    root = pathlib.Path(os.environ.get("CLAUDE_PROJECT_DIR") or cwd)
    try:
        kit = kit_module(root)
        protected = kit.branches()
    except BaseException:  # noqa: BLE001 - a missing kit must not block every command
        kit, protected = None, ("develop", "main")
    g = Guard(mode, root, cwd, protected)
    label = "git guard" if mode == "dev" else "branch guard"
    try:
        for c in simple_commands(cmd):
            g.check(c)
        for pr in g.pr_merges:
            base = subprocess.run(["gh", "pr", "view", pr, "--json", "baseRefName", "-q", ".baseRefName"], cwd=root,
                                  capture_output=True, text=True).stdout.strip()
            if base == g.rel:
                raise Blocked(f"PR #{pr} targets {g.rel}. Promotions and releases go through 'just promote {pr}', which asks the user")
            r = subprocess.run([sys.executable, str(root / ".claude" / "kit" / "kit.py"), "merge-check", pr], cwd=root,
                               capture_output=True, text=True, env=dict(os.environ, KIT_ROOT=str(root)))
            if r.returncode:
                raise Blocked(f"merge-check failed for PR #{pr}:\n{(r.stdout + r.stderr).strip()}")
    except Blocked as e:
        tail = " Push your own branch and open a PR; only the tech lead merges." if mode == "dev" else ""
        print(f"Blocked by {label}: {e}.{tail}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
