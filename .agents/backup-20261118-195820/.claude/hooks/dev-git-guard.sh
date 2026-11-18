#!/usr/bin/env bash
# PreToolUse (Bash) for developers and testers: no merging, no pushing to main, no force push,
# no direct commits on main. Only the tech lead merges.
input=$(cat)
command -v jq >/dev/null 2>&1 || { echo "dev-git-guard: jq is required" >&2; exit 2; }
cmd=$(jq -r '.tool_input.command // ""' <<<"$input")
cwd=$(jq -r '.cwd // empty' <<<"$input")
[ -n "$cwd" ] && cd "$cwd" 2>/dev/null

deny() {
  echo "Blocked by git guard: $1. Push your own branch and open a PR; only the tech lead merges." >&2
  exit 2
}

grep -Eq 'gh[[:space:]]+pr[[:space:]]+merge' <<<"$cmd" && deny "merging PRs"
grep -Eq '(just|kit\.py)[[:space:]]+(merge|promote|setup-branches|protect-branches)([[:space:]]|$)' <<<"$cmd" && deny "merging, promoting or changing branch setup (tech lead / user only)"
grep -Eq 'git[[:space:]]+push\b.*([[:space:]]|:)(main|master)([[:space:]]|$)' <<<"$cmd" && deny "pushing to main"
grep -Eq 'git[[:space:]]+push\b.*[[:space:]](--force|-f)([[:space:]]|$)' <<<"$cmd" && deny "force push (use --force-with-lease on your own branch)"

branch=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || true)
if [ "$branch" = "main" ] || [ "$branch" = "master" ]; then
  grep -Eq 'git[[:space:]]+(commit|merge|reset|rebase|cherry-pick)\b' <<<"$cmd" && deny "changing main directly (create your branch first)"
fi
exit 0
