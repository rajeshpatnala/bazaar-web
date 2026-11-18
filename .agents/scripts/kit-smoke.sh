#!/usr/bin/env bash
# Smoke test for kit 0.2.0 local paths: light lane, ship --branch, review lookup, checkout warning.
# Run in your own terminal (outside Claude Code). Builds a throwaway repo under mktemp; touches nothing else.
# Usage: bash .agents/scripts/kit-smoke.sh [kit_dir]     Output is also saved to <scratch>/smoke.log
set -u
KIT="${1:-$HOME/agent-kits/bazaar-web}"
T="$(mktemp -d)"; W="$T/work"; LOG="$T/smoke.log"; : > "$LOG"
exec > >(tee -a "$LOG") 2>&1
pass=0; fail=0
expect() { # expect "<name>" <rc> "<grep pattern or empty>" <rc_wanted> <<< "$output"
  local name="$1" rc="$2" pat="$3" want="$4" out; out="$(cat)"
  local ok=1
  [ "$rc" -eq "$want" ] || ok=0
  [ -z "$pat" ] || grep -Eq -- "$pat" <<<"$out" || ok=0
  if [ $ok -eq 1 ]; then echo "PASS  $name"; pass=$((pass+1)); else echo "FAIL  $name (rc=$rc, wanted $want, pattern '$pat')"; echo "$out" | sed 's/^/      | /'; fail=$((fail+1)); fi
}
K() { python3 .claude/kit/kit.py "$@"; }
gate() { echo "$(git rev-parse "$1")" >> "$(git rev-parse --git-common-dir)/kit-gate-passed"; }  # simulate a passed gate

echo "== setup in $W"
mkdir -p "$W" && cp -R "$KIT/." "$W/" && cd "$W" || exit 1
rm -rf .git; git init -q -b develop . && git config user.email t@t && git config user.name t
mkdir -p src docs/features && echo 'fn main(){}' > src/main.rs
git add -A && git commit -qm "chore: init" && git branch main
sed -i.bak 's/^| work_mode | .*/| work_mode | worktrees |/' .agents/team.md 2>/dev/null; rm -f .agents/team.md.bak

echo "== 1 preflight on integration branch: no checkout warning"
out="$(K preflight 2>&1)"; rc=$?; echo "$out" | grep -q "main checkout is on" && r=1 || r=0
[ $r -eq 0 ] && { echo "PASS  no warning on develop"; pass=$((pass+1)); } || { echo "FAIL  unexpected warning"; fail=$((fail+1)); }

echo "== 2 light lane: docs-only branch"
git switch -q -c docs/light-x && echo "note" > docs/note.md && git add -A && git commit -qm "docs: note"
git worktree add -q -b agent/wt "$T/wt" develop   # warning applies only once agent worktrees exist
out="$(K preflight 2>&1)"; echo "$out" | grep -q "main checkout: .git switch develop" && { echo "PASS  preflight warns when main checkout is on a feature branch"; pass=$((pass+1)); } || { echo "FAIL  no checkout warning"; echo "$out" | sed 's/^/      | /'; fail=$((fail+1)); }
git switch -q develop
out="$(K review-plan --branch docs/light-x 2>&1)"; rc=$?
expect "review-plan light: no security/tester" $rc "" 0 <<<"$out"; echo "$out" | sed 's/^/      | /'
out="$(K merge-check --branch docs/light-x 2>&1)"; rc=$?
expect "merge-check light w/o gate fails only on gate" $rc "quality gate" 1 <<<"$out"
gate docs/light-x
out="$(K merge-check --branch docs/light-x 2>&1)"; rc=$?
expect "merge-check light passes after gate (no review files)" $rc "PASS" 0 <<<"$out"

echo "== 3 feature branch: review lookup and expected path"
git switch -q -c feat/F1-thing && echo '// change' >> src/main.rs && cat > docs/features/F1-thing.md <<'EOF'
# F1 thing
Status: building
EOF
git add -A && git commit -qm "feat: thing" && git switch -q develop
gate feat/F1-thing
out="$(K merge-check --branch feat/F1-thing 2>&1)"; rc=$?
expect "missing review prints exact path" $rc "missing \.agents/reviews/F1/code-review\.md" 1 <<<"$out"
mkdir -p .agents/reviews/F1 && printf '# Code review: F1 (feat/F1-thing)\n\nVerdict: APPROVE\n' > .agents/reviews/F1/code-review.md
out="$(K merge-check --branch feat/F1-thing 2>&1)"; rc=$?
expect "review found by feature id" $rc "PASS" 0 <<<"$out"
mkdir -p .agents/reviews/other && mv .agents/reviews/F1/code-review.md .agents/reviews/other/code-review.md
out="$(K merge-check --branch feat/F1-thing 2>&1)"; rc=$?
expect "review found by header naming the branch (fallback)" $rc "PASS" 0 <<<"$out"

echo "== 4 review-plan --files (pre-dispatch)"
out="$(K review-plan --files docs/a.md,README.md --feature F2 2>&1)"; rc=$?; expect "files: docs only = light" $rc "" 0 <<<"$out"; echo "$out" | sed 's/^/      | /'
out="$(K review-plan --files .github/workflows/ci.yml --feature F2 2>&1)"; rc=$?; expect "files: ci.yml needs security" $rc "security" 0 <<<"$out"; echo "$out" | sed 's/^/      | /'

echo "== 4b light lane must not cover kit/guard files, renames included"
git switch -q -c docs/kit-x develop && mkdir -p .claude && echo x > .claude/note.md && git add -A && git commit -qm "docs: kit note"; git switch -q develop
out="$(K review-plan --branch docs/kit-x 2>&1)"; echo "$out" | grep -q "light lane" && { echo "FAIL  .claude/** took the light lane"; fail=$((fail+1)); } || { echo "PASS  .claude/** not light lane"; pass=$((pass+1)); }
git switch -q -c docs/team-x develop && echo "# x" >> .agents/team.md && git add -A && git commit -qm "docs: team"; git switch -q develop
out="$(K review-plan --branch docs/team-x 2>&1)"; echo "$out" | grep -q "light lane" && { echo "FAIL  team.md took the light lane"; fail=$((fail+1)); } || { echo "PASS  team.md not light lane"; pass=$((pass+1)); }
echo body > docs/rn.md && git add -A && git commit -qm "docs: rn"   # on develop of the scratch repo, so the rename has a base
git switch -q -c docs/ren-in2 && mkdir -p .claude && git mv docs/rn.md .claude/rn.md && git commit -qm "docs: move in"; git switch -q develop
out="$(K review-plan --branch docs/ren-in2 2>&1)"; echo "$out" | grep -q "light lane" && { echo "FAIL  rename into .claude took the light lane"; fail=$((fail+1)); } || { echo "PASS  rename into .claude not light lane"; pass=$((pass+1)); }
git branch -D docs/ren-in2 >/dev/null 2>&1

echo "== 5 ship --branch (merge + switch)"
out="$(K ship --branch docs/light-x --title 'docs: note' 2>&1)"; rc=$?
expect "ship light branch merges" $rc "merged docs/light-x into develop" 0 <<<"$out"
git log --oneline -1 | grep -q "docs: note" && { echo "PASS  squash commit on develop"; pass=$((pass+1)); } || { echo "FAIL  commit missing"; fail=$((fail+1)); }
git rev-parse -q --verify docs/light-x >/dev/null && { echo "FAIL  branch not deleted"; fail=$((fail+1)); } || { echo "PASS  branch deleted"; pass=$((pass+1)); }
out="$(K ship --branch feat/F1-thing --title 'bad title' 2>&1)"; rc=$?
expect "ship rejects non-conventional title" $rc "conventional" 1 <<<"$out"
out="$(K ship --branch develop --title 'chore: x' 2>&1)"; rc=$?
expect "ship refuses protected branch" $rc "" 1 <<<"$out"

out="$(K ship --branch -x --title 'chore: x' 2>&1)"; rc=$?
expect "ship rejects leading-dash branch" $rc "invalid branch" 1 <<<"$out"
out="$(K ship abc 2>&1)"; rc=$?
expect "ship rejects non-numeric PR" $rc "usage" 1 <<<"$out"
out="$(K merge-check '١٢' 2>&1)"; rc=$?
expect "merge-check rejects non-ASCII digits" $rc "PR number" 1 <<<"$out"
out="$(K merge --branch 2>&1)"; rc=$?
expect "--branch without value is a clean error" $rc "needs a value" 1 <<<"$out"
out="$(K merge --branch nosuch --title 'chore: x' 2>&1)"; rc=$?
expect "missing branch is a clean error" $rc "no such branch" 1 <<<"$out"

echo "== 6 status without gh/board rows"
out="$(PATH=/usr/bin:/bin K status 2>&1)"; rc=$?; expect "status runs" $rc "preflight" 0 <<<"$out"; echo "$out" | sed 's/^/      | /'

echo; echo "RESULT: $pass passed, $fail failed. Log: $LOG  Scratch: $T (safe to delete)"
