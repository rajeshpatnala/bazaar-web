#!/usr/bin/env python3
"""Decision tests for the kit engine with fake repos, fake review files and fake ledgers - no GitHub, no agents.

  python3 .claude/kit/tests/test_kit.py        (or: just kit-test)

Each test builds a throwaway git repo with the kit's scripts, a small stack.json and team.md, then runs kit.py the
way agents do (`--branch` mode, which needs no PR). Run after any change to .claude/kit/*.py."""
import json
import os
import pathlib
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
        shutil.copytree(KIT, self.dir / ".claude" / "kit", ignore=shutil.ignore_patterns("tests", "__pycache__"))
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
                return line.strip().strip("|").split("|")[-1].strip()
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


class NitCostTests(Base):
    LIB = "".join(f"pub fn f{i}() -> i32 {{ {i} }}\n" for i in range(30))

    def decision(self, ref, rid, fname="code-review.md"):
        text = (self.r.dir / f".agents/reviews/F1/{ref}/{fname}").read_text()
        for line in text.splitlines():
            if line.startswith(f"| {rid} "):
                return line.strip().strip("|").split("|")[-1].strip()
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

    def test_small_fix_after_review_needs_a_re_review_no_carry_forward(self):
        b, ref = self.approved_branch([row("CR-1-1", "minor", "src/lib.rs:2", dec="defer: next slice")])
        self.fix_line2(b)
        self.r.gate_pass(b)
        rc, out = self.r.kit("merge-check", "--branch", b, "--feature", "F1")
        self.assertEqual(rc, 1, out)
        self.assertIn("reviews cover the current commit", out)
        self.assertNotIn("carry-forward", out)

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


if __name__ == "__main__":
    unittest.main(verbosity=1)
