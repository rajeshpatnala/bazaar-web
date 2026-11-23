---
name: review-economy
description: Owner wants reviewers called only where their risk applies; minor fix-round findings verified by tech lead, not re-reviewed
metadata:
  type: feedback
---

Call security-reviewer only for security-sensitive PRs (session/auth/secrets/deps/input handling); keep risk-guardian mandatory on order/approval/kill-switch/risk paths; verify minor/nit fixes with `just verify-fixes` (tech lead) instead of re-launching all reviewers.

**Why:** On BZ-511 S0 (scaffolding, no live routes) three reviewers plus three re-reviews found mostly minors/nits; owner questioned blind reviewer calls (2026-11-23) and approved the change.

**How to apply:** At every PR hand-off follow `just review-plan` and drop security when it is not flagged; never drop risk-guardian on its paths.
