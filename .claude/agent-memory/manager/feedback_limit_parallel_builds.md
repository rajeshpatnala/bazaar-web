---
name: limit-parallel-builds
description: Parallel Rust developers in separate worktrees ran the owner's Mac out of memory (2026-11-26); owner rule - only one building agent at a time
metadata:
  type: feedback
---

Don't run several developers that build Rust at the same time on this Mac. On 2026-11-26 three wave-1 developers each ran a full workspace gate in its own worktree (separate target dirs, Arrow/DuckDB-related crates), the owner reported running out of memory and had everything paused. Two had earlier stalled for 600s during those builds.

**Why:** local single machine; each worktree compiles from scratch.

**How to apply:** follow the owner rule in `.agents/owner-rules.md` (2026-11-26): only one building agent at once, across all worktrees, even if a plan recommends up to 4. Only the owner changes that rule. A shared target directory is not the way around it (cicd note `shared_cargo_dir_stale`). The SubagentStop dev-gate hook also starts a build whenever a developer ends its turn. See [[t2-only-when-complete]] for other owner pacing preferences.
