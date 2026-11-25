---
name: fix-round-in-worktree
description: In a fresh worktree, just fix-list can falsely say "nothing to fix" (stop and report), and another agent's worktree is off limits
metadata:
  type: project
---

Seen 2026-11-25 (PR 176): `just fix-list <pr>` run inside an agent worktree printed "nothing to fix" although findings were decided `fix now`.

**Why:** it looked for reviews under the worktree's own `.agents/reviews/`; the files were only in the main checkout at `.agents/reviews/<feature-id>/pr-<n>/*.md`.

**How to apply:** stop and report to the manager (hard rule 7: never work around the kit); continue only when the manager sends the fix list or says how to proceed. For the manager: the review files are in the main checkout at `.agents/reviews/<feature-id>/pr-<n>/*.md` (the "tech-lead decision" column). Drop the `fix-list` part of this note once the kit is fixed.

Pushing: if the brief points at another agent's worktree, git there is refused by isolation: branch from `origin/<pr-branch>` in your own worktree and `git push origin HEAD:<pr-branch>`. Do this only for the PR branch your brief assigns; never force; if the push is rejected, stop and report. Long or compound git/heredoc commands are refused too; keep them plain.
