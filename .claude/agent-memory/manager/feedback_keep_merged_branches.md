---
name: keep-merged-branches
description: Owner does not want local or remote feature branches deleted after merge; kit now honours team.md delete_merged_branches no
metadata:
  type: feedback
---

Never delete feature branches (local or remote) after merging. Owner saw `just ship` print "Deleted local/remote branch feat/F1-core-config" and said "i dont want this" (2026-11-18).

**Why:** the owner wants the branches kept as history.

**How to apply:** owner rule in `.agents/owner-rules.md`. The kit fix is in: `.claude/kit/kit.py` honours `delete_merged_branches` in team.md (currently `no`), checked 2026-11-20 and `just ship 60` kept its branch. Before shipping, `grep delete_merged_branches .agents/team.md` should still say `no`. Related: [[new-branch-per-feature]].
