---
name: triage-pipe-bug
description: until the kit fix lands, an escaped pipe in a review table row can hide an open finding from triage; check statuses before just ship
metadata:
  type: project
---

On 2026-11-26 (PR 197), a review row containing an escaped pipe hid an open major: triage printed "approved to merge". merge-check blocked only because the verdict was REQUEST CHANGES.

**Why:** the kit splits table rows on every pipe, so the status column shifts. The fix needs the owner's approval (items 1-3 of `.agents/reports/kit-recommendations-2026-11-26.md`) and was not yet approved on 2026-11-26.

**How to apply:** until those items are fixed (check `kit.py` or the board's owner action), tell every reviewer not to use pipe characters inside table cells, and read each finding's status in the review files before `just ship`. Triage's "approved" line is not enough. Delete this memory once the fix is merged.
