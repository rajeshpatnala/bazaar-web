---
name: keep-merged-branches
description: Owner does not want local or remote feature branches deleted after merge; just ship's hardcoded delete flag must be removed
metadata:
  type: feedback
---

Never delete feature branches (local or remote) after merging. Owner saw `just ship` print "Deleted local/remote branch feat/F1-core-config" and said "i dont want this" (2026-11-18).

**Why:** the owner wants the branches kept as history. The kit's ship command in `.claude/kit/kit.py` (around line 1715) hardcodes the delete-branch flag for non-protected branches; there is no team.md setting for it.

**How to apply:** recorded as an owner rule in `.agents/owner-rules.md`. A kit fix (setting `delete_merged_branches: no` in team.md, honored by kit.py) goes through a developer PR, since the manager cannot edit outside `.agents/`. Until it is merged, tell the owner that each `just ship` still deletes the branch; the commits stay in the merged PR and in develop. Related: [[new-branch-per-feature]].
