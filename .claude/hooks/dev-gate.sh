#!/usr/bin/env bash
# SubagentStop for developer agents: a developer can't finish while the quality gate fails
# or while production code changed without new unit tests (`just dev-gate` / .claude/kit/kit.py test-check).
# Runs only when the agent's checkout has changes relative to the integration branch; otherwise does nothing.
# The build-runner check before merge and CI remain the authoritative gates.
input=$(cat)
command -v jq >/dev/null 2>&1 || exit 0

# Avoid infinite loops: if we already blocked once and the agent stops again, let it go
# (the developer must then report Gate: FAIL in its handoff, and build-runner re-checks).
[ "$(jq -r '.stop_hook_active // false' <<<"$input")" = "true" ] && exit 0

cwd=$(jq -r '.cwd // empty' <<<"$input")
[ -n "$cwd" ] && cd "$cwd" 2>/dev/null

kitpy=./.claude/kit/kit.py; [ -f "$kitpy" ] || kitpy="$CLAUDE_PROJECT_DIR/.claude/kit/kit.py"
base=$(KIT_ROOT="$PWD" python3 "$kitpy" base-ref 2>/dev/null || echo main)
if git diff --quiet "$base"...HEAD 2>/dev/null && git diff --quiet 2>/dev/null; then
  exit 0
fi

[ -f .agents/stack.json ] || exit 0
kit=./.claude/kit/kit.py
[ -f "$kit" ] || kit="$CLAUDE_PROJECT_DIR/.claude/kit/kit.py"
# wait at most 8 minutes for a build slot, so the hook ends within its own time limit
if ! out=$( { KIT_ROOT="$PWD" KIT_BUILD_WAIT="${KIT_BUILD_WAIT:-480}" GATE_MAX_LINES=15 python3 "$kit" gate && KIT_ROOT="$PWD" python3 "$kit" test-check "$base"; } 2>&1 ); then
  {
    echo "Quality gate failed in $(pwd). Fix it before finishing (or report Gate: FAIL with the reason):"
    echo "$out" | tail -n 25
  } >&2
  exit 2
fi
exit 0
