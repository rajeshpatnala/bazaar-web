#!/usr/bin/env python3
"""Decision tests for the kit engine with fake repos, fake review files and fake ledgers - no GitHub, no agents.

  python3 .claude/kit/tests/test_kit.py        (or: just kit-test)

Each test builds a throwaway git repo with the kit's scripts, a small stack.json and team.md, then runs kit.py the
way agents do (`--branch` mode, which needs no PR). Run after any change to .claude/kit/*.py."""
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile
import textwrap
import unittest

KIT = pathlib.Path(__file__).resolve().parents[1]          # .claude/kit
REPO = KIT.parents[1]                                       # the project (for team.md / templates)

STACK = {
    "id": "test", "title": "test",
    "gate": {"steps": []},
    "test_check": {"source_globs": ["src/**", "risk/**"], "test_globs": ["tests/**", "**/*_test.rs"]},
    "domain_reviews": [{"name": "risk", "agent": "risk-guardian", "file": "risk.md", "globs": ["risk/**"],
                        "envelope": False, "safety": True,
                        "block_severities": ["blocker", "critical", "high", "major"]}],
    "live_path_review": "risk",
}
FINDINGS_HEAD = "| id | severity | file:line | issue | why it matters | suggested fix | status | tech-lead decision |\n|---|---|---|---|---|---|---|---|\n"


def team_md() -> str:
    src = REPO / ".agents" / "team.md"
    if not src.exists():  # running inside the kit factory: use the template
        src = REPO / ".agents" / "team.md.tmpl"
    text = src.read_text()
    return text.replace("{{INTEGRATION_BRANCH}}", "develop").replace("{{RELEASE_BRANCH}}", "main")


class KitRepo:
    def __init__(self):
        self.dir = pathlib.Path(tempfile.mkdtemp(prefix="kit-test-"))
        (self.dir / ".claude").mkdir()
        shutil.copytree(KIT, self.dir / ".claude" / "kit", ignore=shutil.ignore_patterns("tests", "__pycache__", "installer"))
        (self.dir / ".agents" / "reviews").mkdir(parents=True)
        (self.dir / ".agents" / "plans").mkdir(parents=True)
        (self.dir / ".agents" / "stack.json").write_text(json.dumps(STACK))
        (self.dir / ".agents" / "team.md").write_text(team_md())
        (self.dir / ".gitignore").write_text(".agents/reviews/*\n.agents/plans/*\n.agents/budget.csv\n")
        self.git("init", "-q", "-b", "develop")
        self.git("config", "user.email", "t@t")
        self.git("config", "user.name", "t")
        self.write("src/lib.rs", "pub fn a() -> i32 { 1 }\n")
        self.write("docs/features/F1-thing.md", "# F1 thing\n\nStatus: in progress\n")
        self.commit("chore: init")
        self.git("branch", "main")

    # --- helpers
    def git(self, *args) -> str:
        return subprocess.run(["git", *args], cwd=self.dir, capture_output=True, text=True, check=True).stdout.strip()

    def write(self, rel: str, text: str) -> None:
        p = self.dir / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text)

    def commit(self, msg: str) -> str:
        self.git("add", "-A")
        self.git("commit", "-qm", msg)
        return self.git("rev-parse", "HEAD")

    def kit(self, *args, stdin: str = "") -> tuple:
        r = subprocess.run([sys.executable, str(self.dir / ".claude" / "kit" / "kit.py"), *args], cwd=self.dir,
                           capture_output=True, text=True, input=stdin, env=dict(os.environ, KIT_ROOT=str(self.dir)))
        return r.returncode, r.stdout + r.stderr

    def set(self, name: str, value: str) -> None:
        import re
        t = (self.dir / ".agents" / "team.md").read_text()
        t2 = re.sub(rf"^(\|\s*{name}\s*\|\s*)[^|]*(\|)", rf"\g<1>{value} \2", t, flags=re.M)
        assert re.search(rf"^\|\s*{name}\s*\|", t, re.M), f"no row for {name}"
        (self.dir / ".agents" / "team.md").write_text(t2)
        if self.git("status", "--porcelain"):
            self.commit(f"chore: set {name}")  # on the integration branch, so feature branches start from it

    def gate_pass(self, ref: str) -> None:
        sha = self.git("rev-parse", ref)
        f = self.dir / ".git" / "kit-gate-passed"
        f.write_text((f.read_text() if f.exists() else "") + sha + "\n")

    def branch(self, name: str, files: dict) -> str:
        self.git("switch", "-q", "-c", name, "develop")
        for k, v in files.items():
            self.write(k, v)
        sha = self.commit(f"feat: {name}")
        self.git("switch", "-q", "develop")
        return sha

    def review(self, branch: str, name: str, verdict: str, rows: list, commit: str = "", rnd: int = 0,
               save: bool = True) -> tuple:
        """Write a findings file for `branch` the way a reviewer does, pack first, then save-review."""
        ref = branch.replace("/", "-")
        if save:
            self.kit("review-pack", "--branch", branch, "--feature", "F1")
        commit = commit or self.git("rev-parse", branch)
        body = f"# F1 - review\n\nVerdict: {verdict}\nCommit: {commit}\n" + (f"Round: {rnd}\n" if rnd else "") + "\n" + FINDINGS_HEAD
        body += "".join("| " + " | ".join(r) + " |\n" for r in rows)
        path = f".agents/reviews/F1/{ref}/{name}"
        self.write(path, body)
        return self.kit("save-review", path) if save else (0, "")

    def approve(self, branch: str) -> tuple:
        return self.kit("approve", "--branch", branch, "--feature", "F1")

    def close(self):
        shutil.rmtree(self.dir, ignore_errors=True)


def row(i, sev, loc="src/lib.rs:1", issue="a problem", fix="change it", status="open", dec=""):
    return [i, sev, loc, issue, "why", fix, status, dec]


class Base(unittest.TestCase):
    def setUp(self):
        self.r = KitRepo()

    def tearDown(self):
        self.r.close()


class SafetyTests(Base):
    def test_risk_paths_never_take_the_envelope(self):
        self.r.branch("feat/F1-risk", {"risk/limits.rs": "pub fn limit() -> i32 { 5 }\n"})
        rc, out = self.r.kit("envelope", "--branch", "feat/F1-risk", "--feature", "F1", "--dry-run")
        self.assertEqual(rc, 3, out)
        self.assertIn("risk-guardian always reviews its own paths", out)

    def test_policy_files_get_security_and_no_envelope(self):
        self.r.branch("feat/F1-policy", {".agents/team.md": team_md() + "\n"})
        rc, out = self.r.kit("review-plan", "--branch", "feat/F1-policy", "--feature", "F1")
        self.assertIn("security", out)
        rc, out = self.r.kit("envelope", "--branch", "feat/F1-policy", "--feature", "F1", "--dry-run")
        self.assertEqual(rc, 3, out)
        self.assertIn(".agents/team.md", out)

    def test_push_after_review_blocks_merge_and_clean_base_merge_does_not(self):
        b = "feat/F1-a"
        self.r.branch(b, {"src/lib.rs": "pub fn a() -> i32 { 2 }\n"})
        self.assertEqual(self.r.review(b, "code-review.md", "APPROVE", [])[0], 0)
        self.r.approve(b)
        self.r.gate_pass(b)
        rc, out = self.r.kit("merge-check", "--branch", b, "--feature", "F1")
        self.assertEqual(rc, 0, out)
        # base moves on, merged into the branch cleanly: still fine
        self.r.write("src/other.rs", "pub fn o() {}\n")
        self.r.commit("feat: other")
        self.r.git("switch", "-q", b)
        self.r.git("merge", "-q", "--no-edit", "develop")
        self.r.git("switch", "-q", "develop")
        self.r.gate_pass(b)
        rc, out = self.r.kit("merge-check", "--branch", b, "--feature", "F1")
        self.assertEqual(rc, 0, out)
        # the developer pushes a change of its own after approval: refused
        self.r.git("switch", "-q", b)
        self.r.write("src/lib.rs", "pub fn a() -> i32 { 3 }\n")
        self.r.commit("feat: sneaky")
        self.r.git("switch", "-q", "develop")
        self.r.gate_pass(b)
        rc, out = self.r.kit("merge-check", "--branch", b, "--feature", "F1")
        self.assertEqual(rc, 1, out)
        self.assertIn("reviews cover the current commit", out)
        self.assertIn("approval covers the current commit", out)

    def test_rewritten_history_needs_full_rereview(self):
        b = "feat/F1-b"
        self.r.branch(b, {"src/lib.rs": "pub fn a() -> i32 { 2 }\n"})
        self.r.review(b, "code-review.md", "APPROVE", [])
        self.r.git("switch", "-q", b)
        self.r.write("src/lib.rs", "pub fn a() -> i32 { 4 }\n")
        self.r.git("add", "-A")
        self.r.git("commit", "-q", "--amend", "-m", "feat: amended")
        self.r.git("switch", "-q", "develop")
        rc, out = self.r.kit("merge-check", "--branch", b, "--feature", "F1")
        self.assertIn("history was rewritten", out)

    def test_open_security_medium_blocks(self):
        b = "feat/F1-auth"
        self.r.branch(b, {"src/auth.rs": "pub fn login() {}\n"})
        self.r.review(b, "code-review.md", "APPROVE", [])
        self.r.review(b, "security.md", "PASS", [row("SEC-x-1", "medium", "src/auth.rs:1", dec="fix now (auto)")])
        self.r.approve(b)
        self.r.gate_pass(b)
        rc, out = self.r.kit("merge-check", "--branch", b, "--feature", "F1")
        self.assertEqual(rc, 1, out)
        self.assertIn("security review passed", out)

    def test_open_fix_now_minor_blocks_until_verified_and_safety_cannot_be_verified_by_lead(self):
        b = "feat/F1-c"
        self.r.branch(b, {"src/lib.rs": "pub fn a() -> i32 { 2 }\n"})
        self.r.review(b, "code-review.md", "APPROVE", [row("CR-1-1", "minor", dec="fix now if cheap (auto)"),
                                                     row("CR-1-2", "minor", issue="token leaks in a security log", dec="fix now (auto)")])
        self.r.approve(b)
        self.r.gate_pass(b)
        rc, out = self.r.kit("merge-check", "--branch", b, "--feature", "F1")
        self.assertEqual(rc, 1, out)
        self.assertIn("fix-now findings confirmed fixed", out)
        rc, out = self.r.kit("verify-fixes", "--branch", b, "--feature", "F1", "CR-1-1", "CR-1-2")
        self.assertIn("verified CR-1-1", out)
        self.assertIn("refused CR-1-2", out)

    def test_save_review_refuses_bad_files(self):
        b = "feat/F1-d"
        self.r.branch(b, {"src/lib.rs": "pub fn a() -> i32 { 2 }\n"})
        self.r.kit("review-pack", "--branch", b, "--feature", "F1")
        ref = "feat-F1-d"
        cases = {
            "no commit": "Verdict: APPROVE\n\n" + FINDINGS_HEAD,
            "bullets": f"Verdict: APPROVE\nCommit: {self.r.git('rev-parse', b)}\n\n- CR-1 major: missing test\n",
            "blank severity": f"Verdict: APPROVE\nCommit: {self.r.git('rev-parse', b)}\n\n" + FINDINGS_HEAD + "| CR-1 |  | a:1 | x | y | z | open | |\n",
            "invented commit": "Verdict: APPROVE\nCommit: " + "a" * 40 + "\n\n" + FINDINGS_HEAD,
        }
        for name, body in cases.items():
            self.r.write(f".agents/reviews/F1/{ref}/code-review.md", body)
            rc, out = self.r.kit("save-review", f".agents/reviews/F1/{ref}/code-review.md")
            self.assertEqual(rc, 1, f"{name}: {out}")

    def test_rereview_in_a_second_file_is_refused(self):
        b = "feat/F1-h"
        self.r.branch(b, {"src/lib.rs": "pub fn a() -> i32 { 2 }\n"})
        self.r.review(b, "code-review.md", "REQUEST CHANGES", [row("CR-1-1", "major")])
        rc, out = self.r.review(b, "code-review-2.md", "APPROVE", [row("CR-1-1", "major", status="fixed")])
        self.assertEqual(rc, 1, out)
        self.assertIn("update code-review.md in place", out)

    def test_save_review_stamps_round_and_first_seen(self):
        b = "feat/F1-e"
        self.r.branch(b, {"src/lib.rs": "pub fn a() -> i32 { 2 }\n"})
        self.r.review(b, "code-review.md", "REQUEST CHANGES", [row("CR-1-1", "major", dec="")])
        self.r.git("switch", "-q", b)
        self.r.write("src/lib.rs", "pub fn a() -> i32 { 3 }\n")
        self.r.commit("fix: CR-1-1")
        self.r.git("switch", "-q", "develop")
        rc, out = self.r.review(b, "code-review.md", "APPROVE", [row("CR-1-1", "major", status="fixed"), row("CR-1-2", "nit")])
        self.assertEqual(rc, 0, out)
        text = (self.r.dir / ".agents/reviews/F1/feat-F1-e/code-review.md").read_text()
        self.assertIn("Round: 2", text)
        self.assertIn('"CR-1-2":2', text)
        self.assertIn('"CR-1-1":1', text)

    def test_live_path_slice_requires_risk_review(self):
        self.r.write(".agents/plans/F1-thing.md", textwrap.dedent("""\
            # F1 thing
            ## Slices (L only)
            | slice | developer type | scope | est. lines | live path |
            |---|---|---|---|---|
            | 1 | dev-core | wiring | 200 | yes |
            """))
        self.r.branch("feat/F1-s1-wire", {"src/wire.rs": "pub fn w() {}\n"})
        rc, out = self.r.kit("review-plan", "--branch", "feat/F1-s1-wire", "--feature", "F1")
        self.assertIn("risk", out)
        self.assertIn("live or money path", out)


class TriageTests(Base):
    def setup_pr(self, rows, name="code-review.md", files=None):
        b = "feat/F1-t"
        self.r.branch(b, files or {"src/lib.rs": "pub fn a() -> i32 { 2 }\n"})
        self.r.review(b, name, "REQUEST CHANGES", rows)
        return b, "feat-F1-t"

    def decision(self, ref, rid, name="code-review.md"):
        sys.path.insert(0, str(self.r.dir / ".claude" / "kit"))
        text = (self.r.dir / f".agents/reviews/F1/{ref}/{name}").read_text()
        for line in text.splitlines():
            if line.startswith(f"| {rid} |"):
                return re.split(r"(?<!\\)\|", line.strip().strip("|"))[-1].strip()
        return None

    def test_reviewer_deferral_is_respected_for_minor_and_escalated_for_blocker(self):
        b, ref = self.setup_pr([row("CR-1-1", "minor", fix="In slice 7, add it to Provenance"),
                                row("CR-1-2", "major", loc="src/lib.rs:9", fix="defer to slice 7: rework the gate")])
        rc, out = self.r.kit("triage", "F1", "--ref", ref)
        self.assertEqual(self.decision(ref, "CR-1-1"), "defer: slice 7 (reviewer suggested)")
        self.assertEqual(self.decision(ref, "CR-1-2"), "fix now (auto)")  # deferring a blocker is the tech lead's call

    def test_safety_minor_is_fixed_not_left_to_if_cheap_or_deferred(self):
        b, ref = self.setup_pr([row("RISK-1-1", "minor", loc="risk/x.rs:1", fix="make it pub(crate)"),
                                row("RISK-1-2", "minor", loc="risk/x.rs:5", fix="next slice: add a test")],
                               name="risk.md", files={"risk/x.rs": "pub fn x() {}\n"})
        rc, out = self.r.kit("triage", "F1", "--ref", ref)
        self.assertEqual(self.decision(ref, "RISK-1-1", "risk.md"), "fix now (auto)")
        self.assertEqual(self.decision(ref, "RISK-1-2", "risk.md"), "fix now (auto)")  # never deferred by a script

    def test_no_approval_while_a_required_review_is_missing(self):
        b, ref = self.setup_pr([], files={"risk/y.rs": "pub fn y() {}\n"})  # needs code + risk; only code is in
        rc, out = self.r.kit("triage", "F1", "--ref", ref)
        self.assertIn("waiting for risk", out)
        self.assertFalse((self.r.dir / f".agents/reviews/F1/{ref}/approval.md").exists())

    def test_new_minor_after_round_one_is_deferred_only_outside_the_fix(self):
        self.r.set("defer_minor_after_round", "1")
        self.r.set("nit_policy", "fix")
        b = "feat/F1-r"
        self.r.branch(b, {"src/lib.rs": "pub fn a() -> i32 { 2 }\n", "src/util.rs": "pub fn u() {}\n"})
        self.r.review(b, "code-review.md", "REQUEST CHANGES", [row("CR-1-1", "major", dec="fix now (auto)")])
        self.r.git("switch", "-q", b)
        self.r.write("src/lib.rs", "pub fn a() -> i32 { 3 }\n")
        self.r.commit("fix: CR-1-1")
        self.r.git("switch", "-q", "develop")
        self.r.review(b, "code-review.md", "REQUEST CHANGES", [row("CR-1-1", "major", status="fixed", dec="fix now (auto)"),
                                                     row("CR-1-2", "nit", loc="src/util.rs:1"),
                                                     row("CR-1-3", "nit", loc="src/lib.rs:1")])
        self.r.kit("triage", "F1", "--ref", "feat-F1-r")
        self.assertTrue(self.decision("feat-F1-r", "CR-1-2").startswith("defer: next slice"))
        self.assertEqual(self.decision("feat-F1-r", "CR-1-3"), "fix now if cheap (auto)")


class RoundTwoTests(Base):
    def test_fixes_only_pack_shows_fix_diff_and_unasked_changes(self):
        self.r.set("review_rounds", "fixes-only")
        b = "feat/F1-f"
        self.r.branch(b, {"src/lib.rs": "pub fn a() -> i32 { 2 }\n", "src/big.rs": "pub fn big() {}\n"})
        self.r.review(b, "code-review.md", "REQUEST CHANGES", [row("CR-1-1", "major", dec="fix now (auto)")])
        self.r.git("switch", "-q", b)
        self.r.write("src/lib.rs", "pub fn a() -> i32 { 3 }\n")
        self.r.write("src/big.rs", "pub fn big() { println!(\"unasked\"); }\n")
        self.r.commit("fix: CR-1-1")
        self.r.git("switch", "-q", "develop")
        rc, out = self.r.kit("review-pack", "--branch", b, "--feature", "F1")
        self.assertIn("Fix diff since the last review", out)
        self.assertIn("src/big.rs", out.split("Changes no finding asked for")[1])
        rc, out = self.r.kit("fix-scope", "--branch", b, "--feature", "F1")
        self.assertEqual(rc, 1, out)
        self.assertIn("outside src/big.rs", out)

    def test_fix_list_is_only_the_fix_now_items(self):
        b = "feat/F1-g"
        self.r.branch(b, {"src/lib.rs": "pub fn a() -> i32 { 2 }\n"})
        self.r.review(b, "code-review.md", "REQUEST CHANGES", [row("CR-1-1", "major", dec="fix now (auto)"),
                                                             row("CR-1-2", "nit", dec="defer: slice 3")])
        rc, out = self.r.kit("fix-list", "--branch", b, "--feature", "F1")
        self.assertIn("CR-1-1", out)
        self.assertNotIn("CR-1-2", out)
        self.assertIn("Fix-round rules", out)


class TableSafetyTests(Base):
    """P0 (kit recommendations 2026-11-26): an escaped pipe or a malformed row can never hide an open finding."""
    PIPE_ISSUE = r"`a \| b` is parsed as two cells"
    PIPE_FIX = r"split on `(?<!\\)\|` only"

    def approval(self, ref):
        return self.r.dir / f".agents/reviews/F1/{ref}/approval.md"

    def test_escaped_pipe_major_stays_open_in_save_review_triage_fix_list_and_merge_check(self):
        b, ref = "feat/F1-pipe", "feat-F1-pipe"
        self.r.branch(b, {"src/lib.rs": "pub fn a() -> i32 { 2 }\n"})
        rc, out = self.r.review(b, "code-review.md", "REQUEST CHANGES",
                                [row("CR-1-1", "major", issue=self.PIPE_ISSUE, fix=self.PIPE_FIX)])
        self.assertEqual(rc, 0, out)
        self.assertIn("1 finding(s)", out)
        rc, out = self.r.kit("triage", "F1", "--ref", ref)
        self.assertIn("1 decided by the defaults", out)
        self.assertIn("waiting for fix round", out)
        self.assertNotIn("approved to merge", out)
        self.assertFalse(self.approval(ref).exists())
        text = (self.r.dir / f".agents/reviews/F1/{ref}/code-review.md").read_text()
        self.assertIn(r"a \| b", text)  # triage rewrote the row and kept the escape
        rc, out = self.r.kit("fix-list", "--branch", b, "--feature", "F1")
        self.assertIn("CR-1-1", out)
        self.r.approve(b)
        self.r.gate_pass(b)
        rc, out = self.r.kit("merge-check", "--branch", b, "--feature", "F1")
        self.assertEqual(rc, 1, out)
        self.assertIn("FAIL code review approved", out)

    def test_hidden_major_under_an_approving_verdict_fails_merge_check(self):
        b = "feat/F1-hidden"
        self.r.branch(b, {"src/lib.rs": "pub fn a() -> i32 { 2 }\n"})
        rc, out = self.r.review(b, "code-review.md", "APPROVE",
                                [row("CR-1-1", "major", issue=self.PIPE_ISSUE, fix=self.PIPE_FIX, dec="fix now (auto)")])
        self.r.approve(b)
        self.r.gate_pass(b)
        rc, out = self.r.kit("merge-check", "--branch", b, "--feature", "F1")
        self.assertEqual(rc, 1, out)
        self.assertRegex(out, r"FAIL code review approved \(verdict APPROVE; open: CR-1-1")

    def test_malformed_row_is_refused_by_save_review_and_blocks_triage_and_merge(self):
        b, ref = "feat/F1-bad", "feat-F1-bad"
        self.r.branch(b, {"src/lib.rs": "pub fn a() -> i32 { 2 }\n"})
        bad = row("CR-1-1", "major", issue="a | b unescaped", status="open")
        rc, out = self.r.review(b, "code-review.md", "APPROVE", [bad])
        self.assertEqual(rc, 1, out)
        self.assertIn("wrong number of cells in CR-1-1", out)
        # a file that bypassed save-review (hand edit) still can't be approved or merged
        rc, out = self.r.kit("triage", "F1", "--ref", ref)
        self.assertIn("malformed row: CR-1-1", out)
        self.assertFalse(self.approval(ref).exists())
        self.r.approve(b)
        self.r.gate_pass(b)
        rc, out = self.r.kit("merge-check", "--branch", b, "--feature", "F1")
        self.assertEqual(rc, 1, out)
        self.assertIn("CR-1-1 (malformed row)", out)

    def test_empty_or_unknown_status_is_refused(self):
        b = "feat/F1-st"
        self.r.branch(b, {"src/lib.rs": "pub fn a() -> i32 { 2 }\n"})
        for status, msg in (("", "empty status on CR-1-1"), ("fxied", "unknown status on CR-1-1")):
            rc, out = self.r.review(b, "code-review.md", "REQUEST CHANGES", [row("CR-1-1", "major", status=status)])
            self.assertEqual(rc, 1, out)
            self.assertIn(msg, out)

    def test_triage_writes_no_approval_while_a_review_requests_changes(self):
        b, ref = "feat/F1-rc", "feat-F1-rc"
        self.r.branch(b, {"src/lib.rs": "pub fn a() -> i32 { 2 }\n"})
        self.r.review(b, "code-review.md", "REQUEST CHANGES", [row("CR-1-1", "minor", status="fixed", dec="fix now (auto)")])
        rc, out = self.r.kit("triage", "F1", "--ref", ref)
        self.assertIn("waiting for fix round", out)
        self.assertIn("Verdict REQUEST CHANGES", out)
        self.assertFalse(self.approval(ref).exists())
        self.r.review(b, "code-review.md", "APPROVE", [row("CR-1-1", "minor", status="fixed", dec="fix now (auto)")])
        rc, out = self.r.kit("triage", "F1", "--ref", ref)
        self.assertIn("approved to merge", out)
        self.assertTrue(self.approval(ref).exists())


    def test_fix_list_prints_decided_findings_and_lists_undecided_apart(self):
        b = "feat/F1-u"
        self.r.branch(b, {"src/lib.rs": "pub fn a() -> i32 { 2 }\n"})
        self.r.review(b, "code-review.md", "REQUEST CHANGES", [row("CR-1-1", "major", dec="fix now (auto)"),
                                                             row("CR-1-2", "question", issue="should we keep it?")])
        rc, out = self.r.kit("fix-list", "--branch", b, "--feature", "F1")
        self.assertEqual(rc, 0, out)
        self.assertIn("Fix exactly these 1 finding(s)", out)
        self.assertIn("Not in this brief: 1 finding(s) not decided yet (CR-1-2)", out)


class ToolingTests(Base):
    def test_help_never_runs_a_command_and_unknown_flags_are_refused(self):
        rc, out = self.r.kit("records-pr", "--help")
        self.assertEqual(rc, 0)
        self.assertIn("records-pr", out)
        rc, out = self.r.kit("records-pr", "--force")
        self.assertNotEqual(rc, 0)
        self.assertIn("nothing was done", out)

    def test_milestone_from_a_file_keeps_punctuation(self):
        (self.r.dir / ".agents" / "board.md").write_text("# Board\n\n| id | status |\n|---|---|\n")
        note = self.r.dir / "note.txt"
        note.write_text("F9 merged; next: slice 7b3 (golden) - don't \"rebase\"")
        rc, out = self.r.kit("milestone", "--file", str(note))
        self.assertEqual(rc, 0, out)
        self.assertIn('next: slice 7b3 (golden) - don\'t "rebase"', (self.r.dir / ".agents" / "board.md").read_text())

    def test_records_pr_picks_up_files_in_new_folders(self):
        self.r.write(".agents/owner-input/F2-charges.md", "charges\n")
        self.r.write(".agents/handoff-2026-11-20.md", "handoff\n")
        rc, out = self.r.kit("records-pr", "--no-push")
        self.assertIn("committed on docs/agents-records", out)
        files = self.r.git("show", "--name-only", "--format=", self.r.git("branch", "--list", "docs/agents-records-*").strip(" *"))
        self.assertIn(".agents/owner-input/F2-charges.md", files)
        self.assertIn(".agents/handoff-2026-11-20.md", files)

    def test_export_kit_includes_installer_and_engine(self):
        rc, out = self.r.kit("export-kit", "--list")
        self.assertIn(".claude/kit/kit.py", out.split())
        self.assertIn("not found", out)
        self.assertIn("install.py, install.sh", out)
        self.r.write(".claude/kit/installer/install.py", "# kit installer\n")
        self.r.write(".claude/kit/installer/install.sh", "#!/bin/sh\n")
        self.r.write("install.py", "# an older hand-copied installer\n")
        rc, out = self.r.kit("export-kit", "--list")
        listed = out.split()
        self.assertEqual(listed.count("install.py"), 1)
        self.assertIn("install.sh", listed)
        self.assertNotIn(".claude/kit/installer/install.py", listed)
        self.assertNotIn("not found", out)
        z = self.r.dir.parent / f"{self.r.dir.name}-test.zip"
        self.r.kit("export-kit", "--out", str(z))
        import zipfile
        with zipfile.ZipFile(z) as f:
            self.assertEqual(f.read("install.py").decode(), "# kit installer\n")  # the installed kit's copy wins
        z.unlink()


class ReportTests(Base):
    def test_review_wait_returns_when_reviews_are_in(self):
        b = "feat/F1-w"
        self.r.branch(b, {"src/lib.rs": "pub fn a() -> i32 { 2 }\n"})
        self.r.review(b, "code-review.md", "APPROVE", [])
        rc, out = self.r.kit("review-wait", "--branch", b, "--feature", "F1", "--timeout", "1")
        self.assertEqual(rc, 0, out)
        self.assertIn("all in", out)

    def test_deferred_items_not_in_the_feature_record_are_reported(self):
        b = "feat/F1-x"
        self.r.branch(b, {"src/lib.rs": "pub fn a() -> i32 { 2 }\n"})
        self.r.review(b, "code-review.md", "APPROVE", [row("CR-9-1", "nit", dec="defer: slice 3"),
                                                     row("CR-9-2", "nit", loc="src/lib.rs:4", dec="defer: slice 3")])
        self.r.write("docs/features/F1-thing.md", "# F1\n\n## Deferred\n- CR-9-2: slice 3\n")
        self.r.commit("docs: carry CR-9-2")
        rc, out = self.r.kit("deferred", "F1")
        self.assertIn("MISSING CR-9-1", out)
        self.assertIn("carried CR-9-2", out)

    def test_efficiency_leaves_envelope_out_of_cold_starts_and_counts_rounds(self):
        led = self.r.dir / ".agents" / "budget.csv"
        led.write_text("date,time,task,agent,tokens,note,source,processed,start,session,start_cached,cost_eq\n"
                       "2026-11-20,10:00,F1,dev-core,100000,\"50 steps, id a1\",auto,1,10000,s,0,500000\n"
                       "2026-11-20,10:05,F1,envelope-review,30000,pr-1: code,auto,30000,30000,,,40000\n"
                       "2026-11-20,10:09,F1,code-reviewer,40000,\"7 steps, id a2\",auto,1,8000,s,0,90000\n")
        rc, out = self.r.kit("efficiency", "F1")
        self.assertEqual(rc, 0, out)
        report = next((self.r.dir / ".agents" / "reports").glob("efficiency-F1-*.md")).read_text()
        self.assertIn("| cold-start share (agents; envelope left out) | 12%", report)  # 18k of 140k
        self.assertIn("Spend by role", report)
        self.assertNotIn("code-reviewer relaunched", report)

    def test_efficiency_takes_any_task_id_and_session_mode_counts_only_its_reviews(self):
        led = self.r.dir / ".agents" / "budget.csv"
        led.write_text("date,time,task,agent,tokens,note,source,processed,start,session,start_cached,cost_eq\n"
                       "2026-11-20,10:00,LIVE,dev-core,100000,\"50 steps, id a1\",auto,1,10000,s1,0,500000\n"
                       "2026-11-20,11:00,F1,dev-core,100000,\"50 steps, id a3\",auto,1,10000,s2,0,500000\n")
        rc, out = self.r.kit("efficiency", "LIVE")
        self.assertEqual(rc, 0, out)
        self.assertTrue(list((self.r.dir / ".agents" / "reports").glob("efficiency-LIVE-*.md")))
        for i in (1, 2, 3):  # an older feature's review rounds must not count for session s1
            self.r.write(f".agents/reviews/F9/pr-{i}/code-review.md", "Verdict: APPROVE\nRound: 3\n")
        rc, out = subprocess.run([sys.executable, str(self.r.dir / ".claude/kit/kit.py"), "efficiency", "--session"],
                                 cwd=self.r.dir, capture_output=True, text=True,
                                 env=dict(os.environ, KIT_ROOT=str(self.r.dir), CLAUDE_CODE_SESSION_ID="s1")).returncode, ""
        self.assertEqual(rc, 0)
        report = max((self.r.dir / ".agents" / "reports").glob("efficiency-*.md"), key=lambda f: f.stat().st_mtime).read_text()
        self.assertIn("| PR review folders | 0;", report)


class NitCostTests(Base):
    LIB = "".join(f"pub fn f{i}() -> i32 {{ {i} }}\n" for i in range(30))

    def decision(self, ref, rid, fname="code-review.md"):
        text = (self.r.dir / f".agents/reviews/F1/{ref}/{fname}").read_text()
        for line in text.splitlines():
            if line.startswith(f"| {rid} "):
                return re.split(r"(?<!\\)\|", line.strip().strip("|"))[-1].strip()
        return None

    def edit(self, b, files):
        self.r.git("switch", "-q", b)
        for k, v in files.items():
            self.r.write(k, v)
        sha = self.r.commit("fix: small")
        self.r.git("switch", "-q", "develop")
        return sha

    def approved_branch(self, rows, name="feat/F1-n", extra=None):
        self.r.branch(name, {"src/lib.rs": self.LIB, **(extra or {})})
        self.r.review(name, "code-review.md", "APPROVE", rows)
        ref = name.replace("/", "-")
        rc, out = self.r.kit("triage", "F1", "--ref", ref)
        if any(r[7].lower().startswith("fix now") for r in rows):
            # P0 (2026-11-26): no approval while a fix-now finding is unconfirmed; triage approves after the fix push
            self.assertIn("waiting for fix round", out)
            self.assertFalse((self.r.dir / f".agents/reviews/F1/{ref}/approval.md").exists())
        else:
            self.assertIn("approved to merge", out)
        return name, ref

    def test_nit_is_deferred_by_default_minor_and_security_nit_are_not(self):
        b = "feat/F1-nd"
        self.r.branch(b, {"src/auth.rs": "pub fn login() {}\n"})
        self.r.review(b, "code-review.md", "APPROVE", [row("CR-1-1", "nit"), row("CR-1-2", "minor")])
        self.r.review(b, "security.md", "PASS", [row("SEC-1-1", "nit", "src/auth.rs:1")])
        self.r.kit("triage", "F1", "--ref", "feat-F1-nd")
        self.assertTrue(self.decision("feat-F1-nd", "CR-1-1").startswith("defer: next slice"))
        self.assertEqual(self.decision("feat-F1-nd", "CR-1-2"), "fix now if cheap (auto)")
        self.assertEqual(self.decision("feat-F1-nd", "SEC-1-1", "security.md"), "fix now (auto)")

    def test_nit_policy_fix_restores_fix_now(self):
        self.r.set("nit_policy", "fix")
        b = "feat/F1-nf"
        self.r.branch(b, {"src/lib.rs": "pub fn a() -> i32 { 2 }\n"})
        self.r.review(b, "code-review.md", "APPROVE", [row("CR-1-1", "nit")])
        self.r.kit("triage", "F1", "--ref", "feat-F1-nf")
        self.assertEqual(self.decision("feat-F1-nf", "CR-1-1"), "fix now if cheap (auto)")

    def test_save_review_rejects_wrong_verdict_word(self):
        b = "feat/F1-v"
        self.r.branch(b, {"src/lib.rs": "pub fn a() -> i32 { 2 }\n"})
        rc, out = self.r.review(b, "code-review.md", "LGTM", [])
        self.assertEqual(rc, 1, out)
        self.assertIn("APPROVE | REQUEST CHANGES", out)

    def test_save_review_round2_approve_with_open_finding_is_refused(self):
        b = "feat/F1-sr"
        self.r.branch(b, {"src/lib.rs": "pub fn a() -> i32 { 2 }\n"})
        self.r.review(b, "code-review.md", "REQUEST CHANGES", [row("CR-1-1", "major", dec="fix now (auto)")])
        self.edit(b, {"src/lib.rs": "pub fn a() -> i32 { 3 }\n"})
        rc, out = self.r.review(b, "code-review.md", "APPROVE", [row("CR-1-1", "major", dec="fix now (auto)")])
        self.assertEqual(rc, 1, out)
        self.assertIn("CR-1-1", out)
        self.assertIn("keep the verdict as changes requested", out)
        # fixed -> accepted; a new open nit does not block the approval (triage defers it)
        rc, out = self.r.review(b, "code-review.md", "APPROVE", [row("CR-1-1", "major", status="fixed"),
                                                               row("CR-1-2", "nit")])
        self.assertEqual(rc, 0, out)

    FIX1 = ("f1() -> i32 { 1 }", "f1() -> i32 { 11 }")

    def fix_line2(self, b):
        return self.edit(b, {"src/lib.rs": self.LIB.replace(*self.FIX1)})

    def approval_text(self, ref):
        return (self.r.dir / f".agents/reviews/F1/{ref}/approval.md").read_text()

    FIXNOW = "fix now (auto)"

    def merge_check(self, b):
        self.r.gate_pass(b)
        return self.r.kit("merge-check", "--branch", b, "--feature", "F1")

    def test_carry_forward_small_fix_now_minor_fix_passes_and_says_which_rule(self):
        b, ref = self.approved_branch([row("CR-1-1", "minor", "src/lib.rs:2", dec=self.FIXNOW)])
        self.fix_line2(b)
        self.r.kit("triage", "F1", "--ref", ref)  # marks the finding fixed: the change touched its cited line
        rc, out = self.merge_check(b)
        self.assertEqual(rc, 0, out)
        self.assertIn("carry-forward", out)
        self.assertIn("max 20", out)

    def test_carry_forward_off_when_the_setting_is_zero(self):
        self.r.set("unreviewed_fix_max_lines", "0")
        b, ref = self.approved_branch([row("CR-1-1", "minor", "src/lib.rs:2", dec=self.FIXNOW)])
        self.fix_line2(b)
        rc, out = self.merge_check(b)
        self.assertEqual(rc, 1, out)
        self.assertIn("reviews cover the current commit", out)
        self.assertNotIn("carry-forward:", out)

    def test_carry_forward_refuses_over_the_budget_by_text(self):
        self.r.set("unreviewed_fix_max_lines", "1")
        b, ref = self.approved_branch([row("CR-1-1", "minor", "src/lib.rs:2", dec=self.FIXNOW)])
        self.fix_line2(b)  # 1 added + 1 removed = 2 lines
        rc, out = self.merge_check(b)
        self.assertEqual(rc, 1, out)
        self.assertIn("2 changed lines since the review (max 1, team.md unreviewed_fix_max_lines)", out)

    def test_carry_forward_refuses_a_deferred_finding_and_other_lines(self):
        b, ref = self.approved_branch([row("CR-1-1", "minor", "src/lib.rs:2", dec="defer: next slice")])
        self.fix_line2(b)
        rc, out = self.merge_check(b)
        self.assertEqual(rc, 1, out)
        self.assertIn("change outside the fix-now minor/nit findings", out)

    def test_carry_forward_refuses_each_protected_path_by_text(self):
        rows = [row("CR-1-1", "minor", "src/lib.rs:2", dec=self.FIXNOW)]
        for i, (path, text) in enumerate([(".claude/kit/x.py", "x = 1\n"), (".agents/team.md", "| a | b |\n"),
                                          (".agents/owner-rules.md", "- a\n"), ("risk/limits.rs", "pub fn l() {}\n"),
                                          ("src/auth.rs", "pub fn a() {}\n")]):
            self.r.git("switch", "-q", "develop")
            self.r.write(path, text)
            self.r.commit(f"chore: seed {i}")
            b, ref = self.approved_branch(rows, f"feat/F1-p{i}")
            self.edit(b, {path: text.replace("1", "2").replace("a", "c").replace("l()", "m()")})
            rc, out = self.merge_check(b)
            self.assertEqual(rc, 1, f"{path}: {out}")
            self.assertIn("under .claude, a policy file, or a security-sensitive / risk / quant path", out, path)

    def test_carry_forward_refuses_a_file_with_a_security_finding(self):
        b = "feat/F1-sf2"
        self.r.branch(b, {"src/lib.rs": self.LIB})
        self.r.review(b, "code-review.md", "APPROVE", [row("CR-1-1", "minor", "src/lib.rs:2", dec=self.FIXNOW)])
        self.r.review(b, "security.md", "PASS", [row("SEC-1-1", "info", "src/lib.rs:25", status="fixed")])
        self.r.kit("triage", "F1", "--ref", "feat-F1-sf2")
        self.fix_line2(b)
        rc, out = self.merge_check(b)
        self.assertEqual(rc, 1, out)
        self.assertIn("src/lib.rs has a security or safety finding in this PR", out)

    def test_carry_forward_refused_next_to_a_blocking_finding(self):
        b, ref = self.approved_branch([row("CR-1-1", "minor", "src/lib.rs:2", dec=self.FIXNOW),
                                       row("CR-1-2", "major", "src/lib.rs:3", status="fixed")])
        self.fix_line2(b)
        rc, out = self.merge_check(b)
        self.assertEqual(rc, 1, out)
        self.assertIn("next to a blocking finding", out)

    def test_triage_confirm_needs_the_cited_line_in_the_hunk(self):
        self.r.branch("feat/F1-tv", {"src/lib.rs": self.LIB})
        self.r.review("feat/F1-tv", "code-review.md", "APPROVE", [row("CR-1-1", "minor", "src/lib.rs:2")])
        self.r.kit("triage", "F1", "--ref", "feat-F1-tv")
        # a change far from line 2: the finding stays open
        self.edit("feat/F1-tv", {"src/lib.rs": self.LIB.replace("f25() -> i32 { 25 }", "f25() -> i32 { 0 }")})
        rc, out = self.r.kit("triage", "F1", "--ref", "feat-F1-tv")
        self.assertNotIn("touched cited line", out)
        text = (self.r.dir / ".agents/reviews/F1/feat-F1-tv/code-review.md").read_text()
        self.assertNotIn("touched cited line", text)
        # a change on the cited line: marked, with the scope-check wording
        self.fix_line2("feat/F1-tv")
        rc, out = self.r.kit("triage", "F1", "--ref", "feat-F1-tv")
        self.assertIn("touched the cited line", out)
        text = (self.r.dir / ".agents/reviews/F1/feat-F1-tv/code-review.md").read_text()
        self.assertIn("touched cited line (scope check, reviewer confirms)", text)

    def test_triage_confirm_never_applies_to_safety_or_major(self):
        self.r.branch("feat/F1-sf", {"src/auth.rs": "pub fn login() {}\n", "src/lib.rs": self.LIB})
        self.r.review("feat/F1-sf", "code-review.md", "APPROVE", [row("CR-1-1", "major", "src/lib.rs:2", dec="fix now (auto)")])
        self.r.kit("triage", "F1", "--ref", "feat-F1-sf")
        self.fix_line2("feat/F1-sf")
        rc, out = self.r.kit("triage", "F1", "--ref", "feat-F1-sf")
        self.assertNotIn("touched cited line", out)

    def rereview(self, b, rows=None):
        return self.r.review(b, "code-review.md", "APPROVE",
                             rows or [row("CR-1-1", "nit", "src/lib.rs:2", status="fixed")])

    def test_triage_restamps_when_every_review_covers_the_new_head(self):
        b, ref = self.approved_branch([row("CR-1-1", "nit", "src/lib.rs:2", dec="defer: next slice")])
        head = self.fix_line2(b)
        self.rereview(b)
        rc, out = self.r.kit("triage", "F1", "--ref", ref)
        self.assertIn("re-stamped", out)
        text = self.approval_text(ref)
        self.assertIn(head, text)
        self.assertIn("re-stamped by triage after a fix round", text.replace("triage re-stamped", "re-stamped by triage"))

    def test_triage_does_not_restamp_without_a_re_review(self):
        b, ref = self.approved_branch([row("CR-1-1", "nit", "src/lib.rs:2", dec="defer: next slice")])
        old = self.approval_text(ref)
        self.fix_line2(b)
        rc, out = self.r.kit("triage", "F1", "--ref", ref)
        self.assertIn("approval stays", out)
        self.assertEqual(old, self.approval_text(ref))

    def test_triage_does_not_restamp_over_a_risk_path_change(self):
        b, ref = self.approved_branch([row("CR-1-1", "nit", "src/lib.rs:2", dec="defer: next slice")])
        old = self.approval_text(ref)
        self.edit(b, {"risk/limits.rs": "pub fn l() {}\n"})
        self.rereview(b)
        rc, out = self.r.kit("triage", "F1", "--ref", ref)
        self.assertIn("approval stays", out)
        self.assertEqual(old, self.approval_text(ref))

    def test_triage_does_not_restamp_a_garbled_or_non_approving_review(self):
        b, ref = self.approved_branch([row("CR-1-1", "nit", "src/lib.rs:2", dec="defer: next slice")])
        old = self.approval_text(ref)
        self.fix_line2(b)
        self.rereview(b)
        rf = self.r.dir / f".agents/reviews/F1/{ref}/code-review.md"
        good = rf.read_text()
        # verdict kept, findings table gone
        rf.write_text("\n".join(x for x in good.split("\n") if not x.startswith("|")))
        rc, out = self.r.kit("triage", "F1", "--ref", ref)
        self.assertIn("approval stays", out)
        self.assertEqual(old, self.approval_text(ref))
        # a verdict that is not an approval
        rf.write_text(good.replace("APPROVE", "REQUEST CHANGES"))
        rc, out = self.r.kit("triage", "F1", "--ref", ref)
        self.assertIn("approval stays", out)
        self.assertEqual(old, self.approval_text(ref))

    def test_triage_does_not_restamp_when_a_required_review_is_missing(self):
        b, ref = self.approved_branch([row("CR-1-1", "nit", "src/lib.rs:2", dec="defer: next slice")])
        old = self.approval_text(ref)
        self.fix_line2(b)
        self.rereview(b)
        (self.r.dir / f".agents/reviews/F1/{ref}/code-review.md").unlink()
        rc, out = self.r.kit("triage", "F1", "--ref", ref)
        self.assertIn("no findings files", out)
        self.assertEqual(old, self.approval_text(ref))



class BuildEconomyTests(Base):
    """Builds run only when something they read changed, never twice on the same inputs, and one at a time."""

    def use_steps(self, steps):
        st = json.loads((self.r.dir / ".agents/stack.json").read_text())
        st["gate"] = {"steps": steps}
        (self.r.dir / ".agents/stack.json").write_text(json.dumps(st))
        self.r.commit("chore: gate steps")

    def runs(self):
        f = self.r.dir / "ran.txt"
        return f.read_text().count("x") if f.exists() else 0

    STEP = {"name": "build", "cmd": "printf x >> ran.txt", "paths": ["src/**"], "heavy": True}

    def test_steps_skip_when_nothing_they_read_changed_and_reuse_a_pass(self):
        self.use_steps([self.STEP])
        (self.r.dir / ".gitignore").write_text((self.r.dir / ".gitignore").read_text() + "ran.txt\n")
        self.r.commit("chore: ignore")
        self.r.git("switch", "-q", "-c", "feat/F1-docs")
        self.r.write("docs/note.md", "only docs\n")
        rc, out = self.r.kit("gate")
        self.assertEqual(rc, 0, out)
        self.assertIn("skip build (no change to its inputs", out)
        self.assertEqual(self.runs(), 0)
        self.r.write("src/lib.rs", "pub fn a() -> i32 { 9 }\n")
        rc, out = self.r.kit("gate")
        self.assertEqual((rc, self.runs()), (0, 1), out)
        rc, out = self.r.kit("gate")
        self.assertIn("skip build (passed before on exactly these inputs", out)
        self.assertEqual(self.runs(), 1)
        rc, out = self.r.kit("gate", "--fresh")
        self.assertEqual(self.runs(), 2)
        self.r.write("src/lib.rs", "pub fn a() -> i32 { 10 }\n")  # new inputs: runs again
        self.r.kit("gate")
        self.assertEqual(self.runs(), 3)

    def test_a_failed_step_is_not_remembered(self):
        self.use_steps([{"name": "build", "cmd": "false", "paths": ["src/**"]}])
        self.r.git("switch", "-q", "-c", "feat/F1-fail")
        self.r.write("src/lib.rs", "broken\n")
        for _ in range(2):
            rc, out = self.r.kit("gate")
            self.assertEqual(rc, 1, out)
            self.assertIn("FAIL build", out)

    def test_heavy_step_waits_for_the_build_slot_and_gets_the_job_cap(self):
        import fcntl
        self.r.set("build_jobs", "2")
        self.use_steps([{"name": "build", "cmd": "echo jobs=$CARGO_BUILD_JOBS > jobs.txt", "paths": ["src/**"], "heavy": True}])
        self.r.git("switch", "-q", "-c", "feat/F1-lock")
        self.r.write("src/lib.rs", "pub fn a() -> i32 { 3 }\n")
        lock = open(self.r.dir / ".git" / "kit-build-0.lock", "a+")
        fcntl.flock(lock, fcntl.LOCK_EX)
        try:
            r = subprocess.run([sys.executable, str(self.r.dir / ".claude/kit/kit.py"), "gate"], cwd=self.r.dir,
                               capture_output=True, text=True, env=dict(os.environ, KIT_ROOT=str(self.r.dir), KIT_BUILD_WAIT="2"))
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("waiting for a build slot", r.stdout + r.stderr)
            self.assertIn("no build slot", r.stdout + r.stderr)
        finally:
            lock.close()
        rc, out = self.r.kit("gate")
        self.assertEqual(rc, 0, out)
        self.assertEqual((self.r.dir / "jobs.txt").read_text().strip(), "jobs=2")

    def test_cargo_steps_ignore_docs_but_not_config(self):
        st = {"name": "lint", "cmd": "cargo clippy"}
        paths = self.k_paths(st)
        reads = lambda x: self.k.reads(x, paths)
        self.assertFalse(reads("docs/guide.md"))
        self.assertFalse(reads(".agents/board.md"))
        self.assertTrue(reads("crates/core/src/lib.rs"))
        self.assertTrue(reads("config/limits.toml"))  # tests may read it
        self.assertTrue(self.k.is_heavy(st))
        self.assertFalse(self.k.is_heavy({"name": "fmt", "cmd": "cargo fmt --all -- --check"}))

    def k_paths(self, st):
        import importlib.util
        os.environ["KIT_ROOT"] = str(self.r.dir)
        spec = importlib.util.spec_from_file_location("kit_be", self.r.dir / ".claude" / "kit" / "kit.py")
        self.k = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.k)
        return self.k.step_paths(st)


class FunctionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.repo = KitRepo()
        os.environ["KIT_ROOT"] = str(cls.repo.dir)
        import importlib.util
        spec = importlib.util.spec_from_file_location("kit_under_test", cls.repo.dir / ".claude" / "kit" / "kit.py")
        cls.k = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.k)

    @classmethod
    def tearDownClass(cls):
        cls.repo.close()

    def test_touches_cited_line_fails_closed(self):
        r = self.repo
        lib = "".join(f"pub fn f{i}() -> i32 {{ {i} }}\n" for i in range(30))
        r.git("switch", "-q", "develop")
        r.write("src/cl.rs", lib)
        base = r.commit("chore: cl")
        r.write("src/cl.rs", lib.replace("f1() -> i32 { 1 }", "f1() -> i32 { 11 }"))
        head = r.commit("fix: cl")
        k = self.k
        self.assertTrue(k.touches_cited_line(base, head, "src/cl.rs:2"))
        self.assertTrue(k.touches_cited_line(base, head, "src/cl.rs:30 (and :2)"))
        self.assertFalse(k.touches_cited_line(base, head, "src/cl.rs:20"))
        self.assertFalse(k.touches_cited_line(base, head, "src/cl.rs"))
        self.assertFalse(k.touches_cited_line(base, "nonexistent-ref", "src/cl.rs:2"))
        self.assertIsNone(k.git_checked("rev-parse", "nonexistent-ref"))

    def test_ref_base_is_empty_when_the_pr_lookup_fails(self):
        k = self.k
        old = k.pr_info

        def boom(_n):
            raise SystemExit("no gh")
        k.pr_info = boom
        try:
            self.assertEqual(k.ref_base("pr-9"), "")
        finally:
            k.pr_info = old

    def test_carry_forward_fails_closed_on_git_errors_and_odd_changes(self):
        import tempfile as tf
        r, k = self.repo, self.k
        d = pathlib.Path(tf.mkdtemp()) / "pr-1"
        d.mkdir()
        r.git("switch", "-q", "develop")
        head0 = r.git("rev-parse", "HEAD")
        # same commit: nothing changed -> refused, not carried
        ok, why, _ = k.carry_forward_ids(d, head0, head0, "develop")
        self.assertFalse(ok)
        self.assertIn("no changed files", why)
        # binary change
        (r.dir / "bin.dat").write_bytes(b"\x00\x01\x02")
        c1 = r.commit("chore: bin")
        (r.dir / "bin.dat").write_bytes(b"\x00\x01\x03\x00")
        c2 = r.commit("fix: bin")
        ok, why, _ = k.carry_forward_ids(d, c1, c2, "develop")
        self.assertFalse(ok)
        self.assertIn("binary or unreadable change in bin.dat", why)
        # rename and delete
        r.git("mv", "bin.dat", "bin2.dat")
        c3 = r.commit("chore: rename")
        ok, why, _ = k.carry_forward_ids(d, c2, c3, "develop")
        self.assertFalse(ok)
        self.assertIn("added, deleted, renamed or type-changed file", why)
        # mode-only change
        r.write("m.sh", "echo 1\n")
        c4 = r.commit("chore: m")
        (r.dir / "m.sh").chmod(0o755)
        c5 = r.commit("chore: chmod")
        ok, why, _ = k.carry_forward_ids(d, c4, c5, "develop")
        self.assertFalse(ok)
        self.assertIn("mode only", why)
        # git itself failing (checked call returns None): refused
        old = k.git_checked
        k.git_checked = lambda *a: None
        try:
            ok, why, _ = k.carry_forward_ids(d, c4, c5, "develop")
        finally:
            k.git_checked = old
        self.assertFalse(ok)
        self.assertIn("git diff failed", why)

    def test_only_code_and_test_reviewers_can_carry_a_fix(self):
        k = self.k
        for name in ("code-review.md", "tests-core.md", "tests.md"):
            self.assertTrue(k.reviewer_may_carry(name), name)
        for name in ("security.md", "risk.md", "quant.md", "approval.md"):
            self.assertFalse(k.reviewer_may_carry(name), name)

    def test_unreviewed_fix_limit_reads_team_md_and_fails_closed(self):
        k = self.k
        old = k.team_setting
        try:
            for val, want in (("20", 20), ("0", 0), ("abc", 0), ("-", 0), ("", 0)):
                k.team_setting = lambda n, d, v=val: v
                self.assertEqual(k.unreviewed_fix_limit(), want, val)
        finally:
            k.team_setting = old
        self.assertEqual(k.unreviewed_fix_limit(), 20)

    def test_lockfile_changes_are_listed(self):
        r = self.repo
        r.write("Cargo.lock", '[[package]]\nname = "serde"\nversion = "1.0.1"\n')
        r.commit("chore: lock")
        r.git("switch", "-q", "-c", "feat/F1-lock")
        r.write("Cargo.lock", '[[package]]\nname = "serde"\nversion = "1.0.2"\n\n[[package]]\nname = "evil-crate"\nversion = "0.1.0"\n')
        r.commit("feat: dep")
        r.git("switch", "-q", "develop")
        out = self.k.lock_changes("develop", "feat/F1-lock", "Cargo.lock")
        self.assertIn("evil-crate 0.1.0", out)
        self.assertIn("serde 1.0.1 -> 1.0.2", out)

    def test_plan_warnings(self):
        text = "# F\n## Slices\n| slice | dev | est. lines |\n|---|---|---|\n| 1 | a | 900 |\n| 2 | b | 40 |\n"
        w = " ".join(self.k.plan_warnings(text))
        self.assertIn("slice 1: ~900 lines", w)
        self.assertIn("slice 2: ~40 lines", w)
        self.assertIn("live path", w)

    def test_defer_phrases(self):
        rx = self.k.DEFER_RX
        for yes in ("add a test in slice 7", "deferred to slice 4b, as agreed", "confirm it lands in a later slice",
                    "both can wait for slice 10/11"):
            self.assertTrue(rx.search(yes), yes)
        for no in ("The Deferred line ends in a dangling bracket", "Add both to the Deferred list",
                   "In step 2, document the precedence"):
            self.assertFalse(rx.search(no), no)

    def test_safety_follows_second_files_and_ids(self):
        self.assertTrue(self.k.is_safety("risk-2.md", {"id": "X-1"}))
        self.assertTrue(self.k.is_safety("code-review.md", {"id": "RISK-55-4"}))
        self.assertTrue(self.k.is_safety("code-review.md", {"id": "CR-1", "issue": "session token written to a log"}))
        self.assertFalse(self.k.is_safety("riskless.md", {"id": "CR-2", "issue": "rename a variable"}))

    def test_column_aliases(self):
        _, rows = self.k.findings_text("Verdict: APPROVE\n\n| id | sev | where | check | status |\n|---|---|---|---|---|\n| Q-1 | must-fix | a.rs:3 | x | open |\n")
        self.assertEqual(rows[0]["severity"], "major")
        self.assertEqual(rows[0]["file:line"], "a.rs:3")

    def test_split_row_keeps_escaped_pipes_in_one_cell(self):
        self.assertEqual(self.k.split_row(r"| a \| b | c |"), [r" a \| b ", " c "])
        self.assertEqual(self.k.split_row(r"| a \\| b |"), [r" a \\", " b "])  # an escaped backslash, then a real pipe
        self.assertEqual(self.k.split_row("| a | b"), [" a ", " b"])
        self.assertEqual(self.k.split_row("not a row | x"), [])
        _, rows = self.k.findings_text("Verdict: APPROVE\n\n| id | severity | issue | status |\n|---|---|---|---|\n"
                                       r"| X-1 | major | `a \| b` | open |" "\n| X-2 | major | a | b | fixed |\n")
        self.assertEqual(rows[0]["status"], "open")
        self.assertNotIn("_malformed", rows[0])
        self.assertIn("_malformed", rows[1])
        self.assertEqual(self.k.open_blockers(rows, {"major"}), ["X-1", "X-2 (malformed row)"])

    def test_status_states_fail_closed(self):
        st = self.k.status_state
        for s in ("open", "", "not fixed (accepted)", "partly fixed", "disputed", "Open (non-blocking)", "fix now"):
            self.assertEqual(st(s), "open", s)
        for s in ("fixed", "fixed (round 2)", "Fixed in abc123", "accepted risk", "deferred to s5", "verified", "closed"):
            self.assertEqual(st(s), "closed", s)
        for s in ("fxied", "done-ish", "rounding fixed"):
            self.assertEqual(st(s), "unknown", s)
            self.assertTrue(self.k.is_open({"status": s}))

    def test_every_table_parser_copy_matches_kit_py(self):
        import inspect
        want = inspect.getsource(self.k.split_row).split('"""')[2].strip()  # the body, after the docstring
        copies = [KIT / n for n in ("budget.py", "agents_sync.py", "new_agent.py")] + \
            [p for p in (REPO / "install.py",) if p.exists()]
        for f in copies:
            text = f.read_text()
            self.assertTrue("def split_row(" in text, f"{f.name}: no split_row copy")
            body = text.split("def split_row(", 1)[1].split('"""')[2].split("\n\n\n")[0].strip()
            self.assertTrue(body == want, f"{f.name}: split_row differs from kit.py")
            naive = [l.strip() for l in text.splitlines() if '.split("|")' in l and '["matcher"]' not in l]
            self.assertFalse(naive, f"{f.name}: naive table split: {naive[:2]}")

    def test_same_spot_escalates_only_when_fixes_differ(self):
        c = self.k.fixes_conflict
        self.assertFalse(c({"assert the error message", "Assert the error message."}))
        self.assertFalse(c({"assert the refusal reason", "also assert the refusal reason in the test"}))
        self.assertTrue(c({"return early when the list is empty", "make the field optional in the schema"}))


PLAN_L = ("# F1: thing\n\n## Goal\ng\n\n## Slices\n| slice | developer type | scope (modules/files owned) | est. lines | live path | reject if |\n"
          "|---|---|---|---|---|---|\n| 1 | dev-core | `crates/core`, config/x.toml | 200 | no | {r1} |\n| 2 | dev-api | web/ | 200 | no | {r2} |\n")


class DevBriefTests(Base):
    def setUp(self):
        super().setUp()
        import importlib.util
        os.environ["KIT_ROOT"] = str(self.r.dir)
        spec = importlib.util.spec_from_file_location("kit_devbrief", self.r.dir / ".claude" / "kit" / "kit.py")
        self.k = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.k)

    def plan(self, r1="audit row left on failure", r2="x"):
        self.r.write(".agents/plans/F1.md", PLAN_L.format(r1=r1, r2=r2))

    def findings_file(self, name, rows, d="F1/pr-1"):
        self.r.write(f".agents/reviews/{d}/{name}", "Verdict: REQUEST CHANGES\n\n" + FINDINGS_HEAD
                     + "".join("| " + " | ".join(x) + " |\n" for x in rows))

    def test_slice_brief_lists_findings_inside_scope_only(self):
        self.plan()
        self.findings_file("code-review.md", [
            row("CR-1-1", "major", "crates/core/src/a.rs:10", "unchecked add"),
            row("CR-1-2", "major", "crates/core2/src/a.rs:10", "nearby crate, must not match"),
            row("CR-1-3", "minor", "crates/core/src/b.rs:1", "minor, not listed"),
            row("CR-1-4", "high", "config/x.toml:3", "no verified_on")])
        rc, out = self.r.kit("plan", "F1", "--slice", "1")
        self.assertEqual(rc, 0, out)
        self.assertIn("Reviewers caught in these files before:", out)
        self.assertIn("CR-1-1 (crates/core/src/a.rs): unchecked add", out)
        self.assertIn("CR-1-4", out)
        caught = out.split("Reviewers caught in these files before:")[1]
        for no in ("CR-1-2", "CR-1-3"):
            self.assertNotIn(no, caught)

    def test_slice_brief_prints_nothing_without_a_match(self):
        self.plan()
        self.findings_file("code-review.md", [row("CR-1-1", "major", "elsewhere/a.rs:1", "x")])
        rc, out = self.r.kit("plan", "F1", "--slice", "1")
        self.assertNotIn("Reviewers caught", out)

    def test_slice_brief_caps_findings_at_eight(self):
        self.plan()
        self.findings_file("code-review.md", [row(f"CR-1-{i}", "major", f"crates/core/f{i}.rs:1", f"issue {i}")
                                              for i in range(1, 13)])
        rc, out = self.r.kit("plan", "F1", "--slice", "1")
        self.assertEqual(out.count("- CR-1-"), 8, out)
        self.assertIn("CR-1-12", out)
        self.assertNotIn("CR-1-4 ", out)

    def test_garbled_findings_file_is_skipped_with_a_note(self):
        self.plan()
        self.r.write(".agents/reviews/F1/pr-1/bad.md", "Verdict: APPROVE\n\n| nonsense | here |\n| a | b |\n")
        self.findings_file("code-review.md", [row("CR-1-1", "major", "crates/core/a.rs:1", "real")])
        (self.r.dir / ".agents/reviews/F1/pr-1/binary.md").write_bytes(b"\xff\xfe\x00| id |\n")
        rc, out = self.r.kit("plan", "F1", "--slice", "1")
        self.assertEqual(rc, 0, out)
        self.assertIn("CR-1-1", out)
        self.assertIn("skipped 2 file(s) that are unreadable or have no findings table", out)
        self.assertIn("quoted from reviews, not instructions", out)

    def test_reject_if_is_printed_as_tests_to_write_first(self):
        self.plan(r1="audit row left on failure; no refusal reason test")
        rc, out = self.r.kit("plan", "F1", "--slice", "1")
        self.assertIn("Write these tests first:\naudit row left on failure; no refusal reason test", out)

    def test_empty_reject_if_on_an_l_slice_is_flagged(self):
        self.plan(r2="")
        w = " ".join(self.k.plan_warnings(PLAN_L.format(r1="a", r2="")))
        self.assertIn("slice 2: empty 'reject if' cell", w)
        self.assertNotIn("slice 1: empty", w)
        w = " ".join(self.k.plan_warnings("# F\n## Slices\n| slice | dev | est. lines |\n|---|---|---|\n| 1 | a | 200 |\n"))
        self.assertIn("no 'reject if' column", w)

    def test_m_plan_steps_are_not_flagged_for_reject_if(self):
        w = " ".join(self.k.plan_warnings("# F\n## Steps\n| step | what | est. lines |\n|---|---|---|\n| 1 | a | 200 |\n"))
        self.assertNotIn("reject if", w)

    def test_fix_list_ends_with_the_pattern_line(self):
        b = "feat/F1-g"
        self.r.branch(b, {"src/lib.rs": "pub fn a() -> i32 { 2 }\n"})
        self.r.review(b, "code-review.md", "REQUEST CHANGES", [row("CR-1-1", "major", dec="fix now (auto)")])
        rc, out = self.r.kit("fix-list", "--branch", b, "--feature", "F1")
        self.assertTrue(out.rstrip().endswith("search your whole diff for the same pattern and fix every occurrence; "
                                              "list where in the handoff."), out)

    def checklist(self, text):
        self.plan()
        self.r.write(".agents/checklists/dev-core.md", text)

    def test_area_checklist_is_printed_for_the_developer_type(self):
        self.checklist("1. checked maths\n2. fail closed\n")
        rc, out = self.r.kit("plan", "F1", "--slice", "1")
        self.assertIn("Top 10 for this area:\n1. checked maths\n2. fail closed", out)
        rc, out = self.r.kit("plan", "F1", "--slice", "2")  # dev-api has no checklist file
        self.assertNotIn("Top 10", out)

    def test_area_checklist_is_capped_at_fifteen_lines(self):
        self.checklist("".join(f"rule {i}\n" for i in range(1, 31)))
        rc, out = self.r.kit("plan", "F1", "--slice", "1")
        self.assertIn("rule 15", out)
        self.assertNotIn("rule 16", out)

    def test_garbled_area_checklist_is_a_note_not_a_crash(self):
        self.plan()
        (self.r.dir / ".agents/checklists").mkdir(parents=True)
        (self.r.dir / ".agents/checklists/dev-core.md").write_bytes(b"\xff\xfe\x00bad")
        rc, out = self.r.kit("plan", "F1", "--slice", "1")
        self.assertEqual(rc, 0, out)
        self.assertIn("dev-core.md: unreadable", out)

    def test_long_reject_if_is_truncated(self):
        self.plan(r1="; ".join(f"reason {i} " + "x" * 100 for i in range(20)))
        rc, out = self.r.kit("plan", "F1", "--slice", "1")
        sec = out.split("Write these tests first:\n")[1].split("\n\n")[0]
        self.assertLessEqual(len(sec), 820)
        self.assertTrue(sec.endswith("(truncated)"), sec)

    def test_scope_without_paths_says_so(self):
        self.r.write(".agents/plans/F1.md", PLAN_L.format(r1="a", r2="b").replace("web/", "web ui"))
        rc, out = self.r.kit("plan", "F1", "--slice", "2")
        self.assertIn("scope names no paths: no findings lookup", out)

    def test_symlinked_and_oversized_sources_are_not_followed_or_read_whole(self):
        self.plan()
        secret = self.r.dir / "outside.md"
        secret.write_text("# x\n\n" + FINDINGS_HEAD + "| CR-9-9 | major | crates/core/a.rs:1 | leaked | w | f | open | |\n")
        d = self.r.dir / ".agents/reviews/F1/pr-1"
        d.mkdir(parents=True)
        (d / "link.md").symlink_to(secret)
        self.assertNotIn("CR-9-9", self.r.kit("plan", "F1", "--slice", "1")[1])
        self.r.write(".agents/checklists/dev-core.md", "y" * 5000 + "\n" + "z" * (300 * 1024))
        lines = self.k.area_checklist("dev-core")
        self.assertTrue(all(len(l) <= 300 for l in lines), [len(l) for l in lines])
        with self.assertRaises(OSError):
            self.k.read_capped(d / "link.md", d)

    def test_scope_prefixes_are_exact_segments(self):
        p = self.k.scope_prefixes("`crates/core/**`, config/, a.toml and words")
        self.assertEqual(p, ["crates/core", "config", "a.toml"])
        self.assertTrue(self.k.in_scope("crates/core/x.rs", p))
        self.assertFalse(self.k.in_scope("crates/core2/x.rs", p))
        self.assertFalse(self.k.in_scope("crates/cor", p))


if __name__ == "__main__":
    unittest.main(verbosity=1)
