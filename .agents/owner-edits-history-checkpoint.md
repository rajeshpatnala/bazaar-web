# Owner edits after PR #182 (agreed 2026-11-25)

Adds to the human-only rules: `history-checkpoint accept-current`, `paper-session new`, `paper-session resume`.
`paper-session` is not built yet (S10 slice 9); adding it now means one edit only. Agents do not edit these files.

## 1. The list in 14 files (run in Terminal from the repo root)

```sh
# from the repo root
grep -rlF '`import-bhavcopy`, `backtest`)' CLAUDE.md .agents/stack.json .claude/kit/stack.kit.json .claude/agents \
 | while read -r f; do
     perl -pi -e 's/`import-bhavcopy`, `backtest`\)/`import-bhavcopy`, `backtest`, `bazaar-cli history-checkpoint accept-current`, `bazaar-cli paper-session new`, `bazaar-cli paper-session resume`)/g' "$f"
   done
grep -rlF 'paper-session resume`)' CLAUDE.md .agents/stack.json .claude/kit/stack.kit.json .claude/agents | wc -l   # expect 14
```

## 2. `.claude/settings.json`, add inside `permissions.deny`

```json
"Bash(*bazaar-cli*history-checkpoint accept-current*)",
"Bash(just cli-fixture history-checkpoint accept-current*)",
"Bash(*bazaar-cli*paper-session new*)",
"Bash(just cli-fixture paper-session new*)",
"Bash(*bazaar-cli*paper-session resume*)",
"Bash(just cli-fixture paper-session resume*)",
```

Deny rules are a tripwire, not a security boundary. After saving, tell the manager "edits done" so the files can be checked.

## 3. `.claude/skills/bz-repo-structure/references/bazaar-cli.md`

- Line 6, in the subcommand list, after `live-opt-in --mode` (human-only), add:
  `history-checkpoint verify`, `history-checkpoint accept-current` (human-only), `paper-session list`, `paper-session new` / `resume` (human-only; not built yet),
- Line 22, in the human-only list, after `bazaar-cli live-opt-in`, add:
  , `bazaar-cli history-checkpoint accept-current`, `bazaar-cli paper-session new` / `resume`
