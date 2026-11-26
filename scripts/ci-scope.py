#!/usr/bin/env python3
"""Decide which CI jobs a change needs. Same logic for CI and `just ci-scope [base]`.

usage: ci-scope.py <base-ref> [--full]
Prints key=value lines (GITHUB_OUTPUT format): full, rust, web, packages.
  full      yes = run the whole workspace (push, promotion, workflow_dispatch, shared or unknown paths changed)
  rust      yes = run Rust lint/test jobs
  web       yes = run the web job
  packages  space-separated `-p <crate>` list for scoped runs (empty when full)
Fails safe: any doubt or crash means full (reason on stderr).
Test hooks: CI_SCOPE_FILES="a b" replaces the git diff; CI_SCOPE_CRASH=1 forces a crash.
"""
import json
import os
import subprocess
import sys

# Allowlist: only these paths may be skipped or scoped. Any other path (root files, .cargo/, justfile, scripts/,
# config/, fixtures/, workflows, manifests, anything unknown) means a FULL run.
WEB_PREFIX = ("web/",)
DOCS_PREFIX = ("docs/", ".agents/", ".claude/")
DOCS_FILES = {"CHANGELOG.md", "README.md", "AGENT-GUIDE.md", "AGENT-KIT.md", "AGENT-SETTINGS.md", "CLAUDE.md",
              "version.txt", ".release-please-manifest.json"}
TEST_DIRS = ("tests/", "benches/", "examples/")
# Web files that Rust tests read -> crates/<dir> whose tests read it, so a change runs them. A kit test
# (`just kit-test`) fails when a Rust source names a web/ file that is missing here.
WEB_READ_BY_RUST = {"web/package.json": "cli", "web/src/shared/disclaimer.ts": "cli",
                    "web/src/api/schema.d.ts": "server"}


def why(msg):
    print(f"ci-scope: {msg}", file=sys.stderr)


def run(*cmd):
    return subprocess.run(cmd, capture_output=True, text=True, check=True).stdout


def emit(full, rust, web, pkgs):
    print(f"full={'yes' if full else 'no'}")
    print(f"rust={'yes' if rust or full else 'no'}")
    print(f"web={'yes' if web or full else 'no'}")
    print(f"packages={'' if full else ' '.join('-p ' + p for p in sorted(pkgs))}")


def main():
    base = sys.argv[1] if len(sys.argv) > 1 else "origin/develop"
    if "--full" in sys.argv:
        return emit(True, True, True, [])
    try:
        if "CI_SCOPE_FILES" in os.environ:
            files = os.environ["CI_SCOPE_FILES"].split()
        else:
            files = run("git", "diff", "--name-only", "--no-renames", f"{base}...HEAD").split()
    except Exception as e:
        why(f"full run: cannot diff against {base} ({type(e).__name__})")
        return emit(True, True, True, [])
    if not files:
        return emit(False, False, False, [])
    if os.environ.get("CI_SCOPE_CRASH"):
        raise RuntimeError("forced crash (test)")

    web = any(f.startswith(WEB_PREFIX) for f in files)
    crate_files = [f for f in files if f.startswith("crates/")]
    other = [f for f in files if not (f.startswith(WEB_PREFIX + DOCS_PREFIX + ("crates/",)) or f in DOCS_FILES)]
    if other:
        why(f"full run: shared or unknown paths changed: {' '.join(other[:5])}")
        return emit(True, True, True, [])

    readers = {WEB_READ_BY_RUST[f] for f in files if f in WEB_READ_BY_RUST}
    pkgs = set()
    if crate_files or readers:
        try:
            meta = json.loads(run("cargo", "metadata", "--format-version", "1", "--no-deps", "--locked"))
        except Exception as e:
            why(f"full run: cargo metadata failed ({type(e).__name__}; stale Cargo.lock with --locked?)")
            return emit(True, True, True, [])
        ws = {p["name"]: p for p in meta["packages"]}
        by_dir = {p["manifest_path"].rsplit("/", 1)[0].split("/crates/")[-1]: p["name"] for p in meta["packages"]
                  if "/crates/" in p["manifest_path"]}
        direct, src_changed = set(), set()
        for f in crate_files:
            d, _, rest = f[len("crates/"):].partition("/")
            name = by_dir.get(d)
            if name is None:
                why(f"full run: unknown crate dir {d}")
                return emit(True, True, True, [])
            direct.add(name)
            if not rest.startswith(TEST_DIRS):
                src_changed.add(name)  # dependents only care about non-test changes
        rev = {n: set() for n in ws}
        for n, p in ws.items():
            for dep in p["dependencies"]:
                if dep["name"] in ws:
                    rev[dep["name"]].add(n)
        pkgs = set(direct)
        todo = list(src_changed)
        while todo:
            for r in rev[todo.pop()]:
                if r not in pkgs:
                    pkgs.add(r)
                    todo.append(r)
        for d in readers:
            if d not in by_dir:
                why(f"full run: unknown crate dir {d}")
                return emit(True, True, True, [])
            pkgs.add(by_dir[d])  # only its own tests read the web file: no dependents
    emit(False, bool(pkgs), web, pkgs)  # rust=yes never comes with an empty package list


try:
    main()
except BaseException as e:  # fail safe: a crash means a full run, never empty outputs
    why(f"full run: scope script crashed ({type(e).__name__}: {e})")
    emit(True, True, True, [])
