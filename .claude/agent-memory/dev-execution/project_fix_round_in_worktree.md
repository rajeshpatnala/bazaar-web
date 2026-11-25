---
name: fix-round-in-worktree
description: In a fresh worktree, just fix-list can falsely say "nothing to fix", and another agent's worktree is off limits
metadata:
  type: project
---

Seen 2026-11-25 (PR 176): `just fix-list <pr>` run inside an agent worktree printed "nothing to fix" although findings were decided `fix now`.

**Why:** it looked for reviews under the worktree's own `.agents/reviews/`; the files were only in the main checkout at `.agents/reviews/BZ/pr-<n>/*.md`.

**How to apply:** tell the manager that the kit step failed (hard rule 7: no silent workaround). The review files are in the main checkout (the "tech-lead decision" column). If the brief points at another agent's worktree, git there is refused by isolation: branch from `origin/<pr-branch>` in your own worktree and `git push origin HEAD:<pr-branch>`. Long or compound git/heredoc commands are refused too; keep them plain.
