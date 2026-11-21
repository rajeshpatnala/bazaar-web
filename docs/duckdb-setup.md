# DuckDB setup (system library)

The candle store links the system DuckDB library instead of compiling it (owner decision 2026-11-21, BZ-106).
The version must match the crate: `duckdb` 1.20001.x is DuckDB **2.0.1** (crate `1.<MMmmpp>.0` is DuckDB `M.m.p`).

- **macOS:** `brew install duckdb`. `.cargo/config.toml` defaults to `/opt/homebrew/lib` and `/opt/homebrew/include`
  (Apple Silicon). On Intel, export `DUCKDB_LIB_DIR=/usr/local/lib` and `DUCKDB_INCLUDE_DIR=/usr/local/include`.
  No rpath is needed: Homebrew's dylib has an absolute install name.
- **Linux / Windows:** download `libduckdb-<platform>.zip` for v2.0.1 from the DuckDB GitHub releases, unzip it, and set
  `DUCKDB_LIB_DIR` and `DUCKDB_INCLUDE_DIR` to that folder. Linux also needs `LD_LIBRARY_PATH=$DUCKDB_LIB_DIR` to run
  tests; Windows needs the folder on `PATH` (`duckdb.dll` + `duckdb.lib`).
- **Check:** `just duckdb-check` says what is missing or mismatched. If Homebrew upgrades DuckDB past the crate's
  version, the build or tests can fail; pin with `brew pin duckdb` or bump the crate (owner approval).
- CI installs the same release in `.github/workflows/ci.yml` (`DUCKDB_VERSION`); bump both together.
