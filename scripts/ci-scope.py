#!/usr/bin/env python3
"""Decide which CI jobs a change needs. Same logic for CI and `just ci-scope [base]`.

usage: ci-scope.py <base-ref> [--full]
Prints key=value lines (GITHUB_OUTPUT format): full, rust, web, packages.
  full      yes = run the whole workspace (push, promotion, workflow_dispatch, shared files changed)
  rust      yes = run Rust lint/test jobs
  web       yes = run the web job
  packages  space-separated `-p <crate>` list for scoped runs (empty when full)
Fails safe: any doubt (cannot diff, cannot read metadata) means full.
"""
import json
import os
import subprocess
import sys

RUST_PREFIX = ("crates/", "config/", "fixtures/", "scripts/")
RUST_FILES = {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "rustfmt.toml", "clippy.toml", "deny.toml"}
# changes here can affect every crate: run everything
FULL_FILES = {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "rustfmt.toml", "clippy.toml", "deny.toml"}
FULL_PREFIX = (".github/workflows/", "scripts/", "config/", "fixtures/")
WEB_PREFIX = ("web/",)
TEST_DIRS = ("tests/", "benches/", "examples/")


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
        files = os.environ["CI_SCOPE_FILES"].split() if "CI_SCOPE_FILES" in os.environ else run("git", "diff", "--name-only", f"{base}...HEAD").split()  # env: test hook
    except Exception:
        return emit(True, True, True, [])
    if not files:
        return emit(False, False, False, [])

    web = any(f.startswith(WEB_PREFIX) for f in files) or any(f.startswith(".github/workflows/ci") for f in files)
    full = any(f in FULL_FILES or f.startswith(FULL_PREFIX) for f in files)
    crate_files = [f for f in files if f.startswith("crates/")]
    rust = full or bool(crate_files)
    pkgs = set()
    if rust and not full:
        try:
            meta = json.loads(run("cargo", "metadata", "--format-version", "1", "--no-deps", "--locked"))
        except Exception:
            return emit(True, True, web, [])
        ws = {p["name"]: p for p in meta["packages"]}
        by_dir = {p["manifest_path"].rsplit("/", 1)[0].split("/crates/")[-1]: p["name"] for p in meta["packages"]
                  if "/crates/" in p["manifest_path"]}
        direct, src_changed = set(), set()
        for f in crate_files:
            d, _, rest = f[len("crates/"):].partition("/")
            name = by_dir.get(d)
            if name is None:
                return emit(True, True, web, [])  # unknown layout
            direct.add(name)
            if not rest.startswith(TEST_DIRS):
                src_changed.add(name)  # dependents only care about non-test changes
        # reverse dependencies of crates whose non-test code changed
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
    emit(full, rust, web, pkgs)


main()
