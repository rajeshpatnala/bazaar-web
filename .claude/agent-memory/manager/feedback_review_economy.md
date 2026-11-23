---
name: review-economy
description: Owner wants reviewers called only where their risk applies; minor fix-round findings verified by tech lead, not re-reviewed
metadata:
  type: feedback
---

Call security-reviewer only for security-sensitive PRs (session/auth/secrets/deps/input handling); keep risk-guardian mandatory on order/approval/kill-switch/risk paths; verify minor/nit fixes with `just verify-fixes` (tech lead) instead of re-launching all reviewers.

**Why:** On BZ-511 S0 (scaffolding, no live routes) three reviewers plus three re-reviews found mostly minors/nits; owner questioned blind reviewer calls (2026-11-23) and approved the change.

**How to apply:** At every PR hand-off follow `just review-plan` and drop security when it is not flagged; never drop risk-guardian on its paths.

**Unreviewed-fix budget (owner, 2026-11-23, CR-143-4):** small fixes (<= 20 lines, setting unreviewed_fix_max_lines) for minor/nit findings may merge without re-review, but never on .claude/**, policy, risk-guardian, quant or security-sensitive paths. Owner first picked 'drop it' while thinking it was about nits; explain severity terms (blocker/major/minor/nit) plainly when asking review-policy questions.
