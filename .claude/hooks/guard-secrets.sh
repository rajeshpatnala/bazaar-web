#!/usr/bin/env bash
# PreToolUse guard (all agents): blocks tool calls that reference secret material.
# Checks only paths/patterns for file tools and the command for Bash, not file contents.
# Exit 2 = block the call and show the reason to the agent.
input=$(cat)
command -v jq >/dev/null 2>&1 || { echo "guard-secrets: jq is required" >&2; exit 2; }

tool=$(jq -r '.tool_name // ""' <<<"$input")
if [ "$tool" = "Bash" ]; then
  s=$(jq -r '.tool_input.command // ""' <<<"$input")
else
  s=$(jq -r '[.tool_input.file_path, .tool_input.path, .tool_input.pattern, .tool_input.glob, .tool_input.notebook_path]
             | map(select(. != null)) | join(" ")' <<<"$input")
fi

# Allowed look-alikes
s=${s//.env.example/}
s=${s//.env.sample/}

block() {
  echo "Blocked by secret guard: $1. Agents never read secrets. Use 'just db-check' for connection status; key names are in .env.example and the user fills .env." >&2
  exit 2
}

grep -Eq '(^|[^A-Za-z0-9_])\.env(rc)?([.][A-Za-z0-9_-]+)?($|[^A-Za-z0-9_-])' <<<"$s" && block "reference to a .env file"
grep -Eq '\.(pem|p8|p12|pfx|key|jks|keystore)($|[^A-Za-z0-9_])' <<<"$s" && block "reference to a key file"
grep -Eq 'id_(rsa|ecdsa|ed25519)|(^|[/ ])secrets/|/proc/[^ ]*/environ' <<<"$s" && block "reference to secret storage"

if [ "$tool" = "Bash" ]; then
  grep -Eq '(^|[;&|(]\s*)(printenv|env|set|export -p|declare -x)\s*($|[|>;&)])' <<<"$s" && block "environment dump"
  grep -Eq '\$\{?[A-Za-z_]*(SECRET|PASSWORD|PASSWD|TOKEN|PRIVATE_KEY|API_KEY|CREDENTIAL)' <<<"$s" && block "expanding a secret variable"
fi
exit 0
