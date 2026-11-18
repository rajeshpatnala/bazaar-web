---
name: new-branch-per-feature
description: Owner wants a new branch for every new feature; never work directly on develop or main
metadata:
  type: feedback
---

Create a new branch for each new feature (from `just base`, e.g. `feat/F<n>-<slug>`), never work directly on `develop` or `main`.

**Why:** GitHub branch protection is unavailable (private repo on free plan, 403 on 2026-11-18), and the owner chose to go without it (option 3). Local discipline is the only guard, so branching per feature is the safeguard.

**How to apply:** at the start of every Work-lane task, make sure the developer is dispatched on its own new branch (worktree) before any commit. Check the checkout is not `develop`/`main` before work starts. Revisit if the owner upgrades to GitHub Pro (`just protect-branches --apply`).
