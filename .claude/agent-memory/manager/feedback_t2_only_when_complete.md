---
name: t2-only-when-complete
description: Never propose or launch T2 for a feature unless all its slices are merged and the record is not "building"
metadata:
  type: feedback
---

Only propose or launch the once-per-feature T2 pass when every planned slice is merged. Check the plan's slice table, the board row and the feature record status first, whatever `just status` says is "T2 pending".

**Why:** on 2026-11-25 I ran T2 for F10 although slice 9 (audit anchor, pending-intent reload, reset audit) was unbuilt. The owner called it wrong. The sign-off then covered only slices 0-8, which is misleading.

**How to apply:** before any T2 offer, read the plan's slices and `docs/features/F<n>-*.md` status. If anything is open, say so and offer to plan or build the remaining slices instead. Treat the status line's "T2 pending" as a hint only.
