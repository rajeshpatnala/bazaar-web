---
name: findings-table-pipes
description: Never put a bare `||` or `|` inside a findings-file table cell (decision text or suggested fix); it shifts the columns and merge-check reads the finding as open
metadata:
  type: feedback
---

Findings files (`.agents/reviews/<F>/pr-N/*.md`) are pipe tables parsed by the kit. A bare `|` or `||` in a cell (e.g. `a || b` in a Rust condition) splits the cell, shifts the status column, and merge-check keeps reporting the finding as open (seen on PR 102 RISK-102-3, 2026-11-21).

**Why:** the parser counts columns from the right; extra cells misplace `status`.
**How to apply:** write decisions in words ("or"), or escape as `\|\|`. After editing, run `just merge-check <pr>`; if a closed finding still shows open, check that row's cell count first. Also copy the fixed file to `.saved/` if a stale copy exists there.
