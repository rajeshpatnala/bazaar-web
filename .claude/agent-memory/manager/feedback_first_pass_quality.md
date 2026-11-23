---
name: first-pass-quality
description: Owner wants fewer review rounds by building better first drafts; checklist-first, failure modes in plans, self-review gate, 400-line stop (2026-11-23)
metadata:
  type: feedback
---
Get code right the first time instead of relying on re-review rounds. Measured baseline (2026-11-22): 72% of PRs needed round 2, 1.9 rounds per PR, reviewers 27% of spend, 77% of findings minor/nit.

**Why:** owner pushed back when I only proposed review-setting tweaks; they want the root causes fixed (devs never saw reviewer checklists, happy-path plans, no feedback loop, oversized slices).

**How to apply:** do not dispatch a slice whose plan row lacks failure modes, checklists and answered policy questions; bounce handoffs missing the "checklists / self-check" line; feed each slice's major findings into the next brief; update .agents/known-pitfalls.md after each efficiency review; report rounds per PR and the blocker/major rate after each feature against the baseline. The envelope self-check does NOT work for developers on risk-guardian paths or diffs over 400 lines, so for those the self-check means going through the checklists by hand. Related: [[review-economy]].
