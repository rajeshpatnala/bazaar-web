#!/usr/bin/env bash
# PreToolUse (Bash) for ALL agents: protected branches only change through checked PRs.
# - no push to the integration or release branch, no commits/merges/rebases/resets while on them
# - POLICY: never delete, rename, force-move or overwrite the integration or release branch (or the repo)
# - `gh pr merge` runs the merge-check first; PRs into the release branch must use `just promote`
#   (a command Claude Code asks the user about every time)
# Branch protection on GitHub is the authoritative safeguard; this hook stops mistakes early.
input=$(cat)
command -v jq >/dev/null 2>&1 || { echo "branch-guard: jq is required" >&2; exit 2; }
cmd=$(jq -r '.tool_input.command // ""' <<<"$input")
cwd=$(jq -r '.cwd // empty' <<<"$input")
[ -n "$cwd" ] && cd "$cwd" 2>/dev/null
grep -Eq '\bgit\b|\bgh\b' <<<"$cmd" || exit 0

root="${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel 2>/dev/null || pwd)}"
kit="$root/.claude/kit/kit.py"
read -r integ rel < <(KIT_ROOT="$root" python3 "$kit" branches 2>/dev/null | sed -E 's/integration=([^ ]+) release=([^ ]+).*/\1 \2/')
integ=${integ:-develop}; rel=${rel:-main}

deny() { echo "Blocked by branch guard: $1" >&2; exit 2; }
prot="($integ|$rel)"

grep -Eq "git[[:space:]]+push\b.*([[:space:]]|:)(refs/heads/)?$prot([[:space:]]|$)" <<<"$cmd" \
  && deny "pushing to $integ/$rel. Push your work branch and open a PR to $integ."
grep -Eq 'git[[:space:]]+push\b.*[[:space:]](--force|-f)([[:space:]]|$)' <<<"$cmd" \
  && deny "force push (use --force-with-lease on your own work branch)."

# Policy: never wipe the integration or release branch (delete, rename, force-move, overwrite, reset).
wipe="never delete, rename, force-move or reset $integ/$rel (policy). Ask the user if you believe this is needed."
grep -Eq "git[[:space:]]+branch\b.*[[:space:]](-[dDmMfcC]|--delete|--move|--force|--copy)\b.*[[:space:]]$prot([[:space:]]|$)" <<<"$cmd" && deny "$wipe"
grep -Eq "git[[:space:]]+push\b.*([[:space:]](--delete|-d)[[:space:]].*\b$prot([[:space:]]|$)|[[:space:]]\+(refs/heads/)?$prot([[:space:]]|$)|[[:space:]]\+?[^[:space:]]*:(refs/heads/)?$prot([[:space:]]|$))" <<<"$cmd" && deny "$wipe"
grep -Eq "git[[:space:]]+push\b.*[[:space:]]--(mirror|prune)\b" <<<"$cmd" && deny "mirror/prune pushes can delete remote branches."
grep -Eq "git[[:space:]]+update-ref\b.*[[:space:]](-d[[:space:]]+)?(refs/heads/)?$prot([[:space:]]|$)" <<<"$cmd" && deny "$wipe"
grep -Eq "git[[:space:]]+(checkout[[:space:]]+-B|switch[[:space:]]+(-C|--force-create))[[:space:]]+$prot([[:space:]]|$)" <<<"$cmd" && deny "$wipe"
if grep -Eq "gh[[:space:]]+api\b" <<<"$cmd" && grep -Eq "DELETE|PATCH|-f[[:space:]]+force=true" <<<"$cmd" && grep -Eq "(refs/)?heads/$prot\b|branches/$prot\b" <<<"$cmd"; then
  deny "$wipe"
fi
grep -Eq "gh[[:space:]]+repo[[:space:]]+(delete|archive|rename)\b" <<<"$cmd" && deny "deleting, archiving or renaming the repository."

cur=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || true)
if [ "$cur" = "$integ" ] || [ "$cur" = "$rel" ]; then
  grep -Eq 'git[[:space:]]+(commit|merge|rebase|cherry-pick|am|revert|reset)\b|git[[:space:]]+push([[:space:]]*$|[[:space:]]+(-u[[:space:]]+)?origin[[:space:]]*$|[[:space:]]+origin[[:space:]]+HEAD)' <<<"$cmd" \
    && deny "changing $cur directly. Create a work branch (just new-branch <type/id-slug>) and open a PR."
fi

pr=$(grep -Eo 'gh[[:space:]]+pr[[:space:]]+merge[[:space:]]+[#]?[0-9]+' <<<"$cmd" | grep -Eo '[0-9]+$')
if grep -Eq 'gh[[:space:]]+pr[[:space:]]+merge' <<<"$cmd"; then
  [ -z "$pr" ] && deny "gh pr merge needs an explicit PR number so it can be checked. Use: just merge <pr>"
  base=$(gh pr view "$pr" --json baseRefName -q .baseRefName 2>/dev/null)
  [ "$base" = "$rel" ] && deny "PR #$pr targets $rel. Promotions and releases go through 'just promote $pr', which asks the user."
  if ! out=$(KIT_ROOT="$root" python3 "$kit" merge-check "$pr" 2>&1); then
    echo "Blocked by branch guard: merge-check failed for PR #$pr:" >&2
    echo "$out" >&2
    exit 2
  fi
fi
exit 0
