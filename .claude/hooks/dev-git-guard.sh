#!/usr/bin/env bash
# PreToolUse (Bash) for developers and testers: no merging, no pushing to main, no force push,
# no direct commits on main. Only the tech lead merges.
input=$(cat)
command -v jq >/dev/null 2>&1 || { echo "dev-git-guard: jq is required" >&2; exit 2; }
# Agent-attached hooks can also fire for tool calls of OTHER agents (e.g. the manager's hooks while it runs as the
# main session, or two sub-agents running at once). `--for <agent>` makes this hook apply only to that agent:
# when the call comes from a different agent (hook input `agent_type`), it is not ours to judge.
for_agent=""
if [ "${1:-}" = "--for" ]; then for_agent="${2:-}"; shift 2; fi
caller=$(jq -r '.agent_type // empty' <<<"$input" 2>/dev/null)
if [ -n "$for_agent" ] && [ -n "$caller" ] && [ "$caller" != "$for_agent" ]; then exit 0; fi
cmd=$(jq -r '.tool_input.command // ""' <<<"$input")
cwd=$(jq -r '.cwd // empty' <<<"$input")
[ -n "$cwd" ] && cd "$cwd" 2>/dev/null

deny() {
  echo "Blocked by git guard: $1. Push your own branch and open a PR; only the tech lead merges." >&2
  exit 2
}

grep -Eq 'gh[[:space:]]+pr[[:space:]]+merge' <<<"$cmd" && deny "merging PRs"
grep -Eq '(just|kit\.py)[[:space:]]+(merge|ship|promote|setup-branches|protect-branches)([[:space:]]|$)' <<<"$cmd" && deny "merging, promoting or changing branch setup (tech lead / user only)"
grep -Eq 'git[[:space:]]+push\b.*([[:space:]]|:)(main|master)([[:space:]]|$)' <<<"$cmd" && deny "pushing to main"
grep -Eq 'git[[:space:]]+push\b.*[[:space:]](--force|-f)([[:space:]]|$)' <<<"$cmd" && deny "force push (use --force-with-lease on your own branch)"

branch=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || true)
if [ "$branch" = "main" ] || [ "$branch" = "master" ]; then
  grep -Eq 'git[[:space:]]+(commit|merge|reset|rebase|cherry-pick)\b' <<<"$cmd" && deny "changing main directly (create your branch first)"
fi
exit 0
