#!/bin/sh
# Checks the system DuckDB library the build links (BZ-106). Prints one clear line per problem; exit 1 on failure.
# Same variables and defaults as .cargo/config.toml; setup steps: docs/duckdb-setup.md.
cd "$(dirname "$0")/.." || exit 1
lib_dir="${DUCKDB_LIB_DIR:-/opt/homebrew/lib}"
inc_dir="${DUCKDB_INCLUDE_DIR:-/opt/homebrew/include}"

# crate 1.<MMmmpp>.0 maps to DuckDB M.m.p (1.20001.0 -> 2.0.1)
crate=$(sed -n 's/^duckdb = { version = "~1\.\([0-9]*\)\..*/\1/p' Cargo.toml)
[ -n "$crate" ] || { echo "FAIL: duckdb version not found in Cargo.toml"; exit 1; }
want="$((crate / 10000)).$((crate / 100 % 100)).$((crate % 100))"

lib=""
for f in libduckdb.dylib libduckdb.so duckdb.dll; do
  if [ -f "$lib_dir/$f" ]; then lib="$lib_dir/$f"; break; fi
done
if [ -z "$lib" ]; then
  echo "FAIL: no DuckDB library in $lib_dir. macOS: brew install duckdb. Linux/Windows: extract the duckdb-shared-libs release v$want and set DUCKDB_LIB_DIR (docs/duckdb-setup.md)."
  exit 1
fi
if [ ! -f "$inc_dir/duckdb.h" ]; then
  echo "FAIL: duckdb.h not found in $inc_dir. Set DUCKDB_INCLUDE_DIR to the folder with duckdb.h (docs/duckdb-setup.md)."
  exit 1
fi
if ! LC_ALL=C grep -aq "v$want" "$lib"; then
  have=$(LC_ALL=C grep -aoE 'v[0-9]+\.[0-9]+\.[0-9]+' "$lib" | sort | uniq -c | sort -rn | head -1 | awk '{print $2}')
  echo "FAIL: $lib does not look like DuckDB v$want (the crate needs exactly that; found ${have:-no version string}). Install the matching version (docs/duckdb-setup.md)."
  exit 1
fi
echo "OK duckdb v$want: $lib, headers $inc_dir"
