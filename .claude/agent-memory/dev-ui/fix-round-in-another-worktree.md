---
name: fix-round-in-another-worktree
description: What to do when a brief says to work in another agent's worktree, or fix-list says nothing to fix
metadata:
  type: project
---

The isolation guard refuses git in another agent's worktree. In your own worktree, `git checkout --detach origin/<branch>`
(first check that it equals the local branch), commit, then `git push origin HEAD:refs/heads/<branch>`; report that the
other checkout is now behind. Push only the PR branch your brief assigns, and never force. If origin and the local branch
differ, or the push is rejected, stop and report to the manager.

`just fix-list <pr>` can print "nothing to fix" in a worktree because the findings are only in the main checkout's
`.agents/reviews/<feature-id>/pr-<n>/` (the id may be `BZ`, not only `F<n>`) (seen 2026-11-26). Stop and report to the
manager (hard rule 7: never work around a kit step). Continue only when the manager sends the fix list or says how to
proceed.

**Why:** the guard is deliberate, so don't route around it. **How to apply:** fix rounds whose prompt names an existing worktree.
