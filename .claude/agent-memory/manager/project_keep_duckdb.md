---
name: project-keep-duckdb
description: Owner keeps DuckDB but wants the system library (not bundled); ticket BZ-106; Dockerfile install later
metadata:
  type: project
---

Owner keeps DuckDB (F12 DuckStore) but approved (2026-11-21) linking the system library instead of the bundled C++ compile: `brew install duckdb`, drop `bundled` in Cargo.toml, ticket BZ-106 in W2. Dockerfile install step comes later.

**Why:** bundled build recompiles DuckDB C++ in every worktree (RAM, 10-25 min) and on every new machine; owner may move to another machine (maybe Windows).

**How to apply:** don't propose removing DuckDB. Crate version must match the installed DuckDB (crate 1.20001.0 ~ DuckDB 2.0.1). Windows is not scoped; secrets use macOS Keychain.
