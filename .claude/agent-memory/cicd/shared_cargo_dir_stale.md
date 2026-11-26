---
name: shared-cargo-dir-stale
description: Sharing CARGO_TARGET_DIR or build.build-dir across git worktrees reused another checkout's code (tested on cargo 1.100.0, 2026-11-26)
metadata:
  type: project
---

Do not share a Cargo target dir or `build.build-dir` across worktrees of this repo.

**Why:** In a two-copy toy workspace with the same relative layout (2026-11-26, cargo 1.100.0), copy B's sources were written before copy A built. B then ran A's binary, and B's test passed although B's own source should have failed it. Both sharing methods did this, so a gate could pass falsely. The likely cause (inferred, not traced) is that path crates get the same unit hash and freshness is judged by mtime.

**How to apply:** Reject shared-dir proposals unless a newer Cargo is shown to fix this with the same two-copy test (build A, then B whose files are older).
