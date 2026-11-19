---
name: paper-and-live-both-supported
description: Owner requires both paper and real (live) trading to be supported at all times, chosen in the UI; paper stays the default
metadata:
  type: project
---

The app must always support both paper trading and real trading; the UI will be designed to let the owner switch between them (stated 2026-11-20 during F6 PR 60 review).

**Why:** the owner wants one app for practice and for real orders, not a paper-only build that gets replaced later.

**How to apply:** treat live mode as a real near-term feature, not a distant one. The paper/live choice must be made server-side (the executor/venue decides it, session-bound, with an explicit confirmation), never as a field the browser sends to the gate. Paper stays the default and `live-trading` stays a non-default cargo feature (CLAUDE.md never-break rules). Gate finding RISK-60-2/CR-60-4 (live flag is caller-supplied) is the F10/UI hand-off that enforces this. See [[feedback_new_branch_per_feature]] for branch rules.
