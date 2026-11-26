---
name: findings-table-pipes
description: Never put any pipe character, escaped or not, inside a findings table cell; the kit splits rows on every pipe and can hide an open finding
metadata:
  type: feedback
---

Put no pipe character in any cell of a findings file (`.agents/reviews/<F>/pr-N/*.md`), and do not escape one: the kit splits rows on every pipe, escaped ones too. Write "or", and describe closures and shell pipes in words.

**Why:** the extra cells shift the later columns. On PR 102 (RISK-102-3, 2026-11-21) a closed finding read as open. On PR 197 (2026-11-26) an escaped pipe hid an open major and triage printed "approved to merge". This note's earlier advice to escape pipes was wrong.

**How to apply:** read each finding's status in the file yourself; do not rely on triage or merge-check alone. If a row looks broken, check its cell count. Copy a fixed file to `.saved/` if a stale copy exists there. Drop this note once the kit splits only on unescaped pipes (see the manager note `triage-pipe-bug`).
