#!/bin/sh
# CI: install the pinned system DuckDB (BZ-106). Usage: install-duckdb.sh linux-amd64 | osx-universal
# The version comes from Cargo.toml (same rule as duckdb-check.sh); the archive is verified against the sha256
# below (digests of the GitHub release assets, checked 2026-11-21). A new version needs new digests here.
set -eu
cd "$(dirname "$0")/.."
platform="${1:?usage: install-duckdb.sh linux-amd64|osx-universal}"
crate=$(sed -n 's/^duckdb = { version = "~1\.\([0-9]*\)\..*/\1/p' Cargo.toml)
ver="$((crate / 10000)).$((crate / 100 % 100)).$((crate % 100))"

case "$ver-$platform" in
  2.0.1-linux-amd64) sum=e25502612fed533a243b61f23a2a574daad030e7e88906222a5f0ab30d06769e ;;
  2.0.1-osx-universal) sum=9a4be5a2a31ad5bb8c92bf37f18d59b9c10a79b392d7848cabd6eb2e05837449 ;;
  *) echo "no pinned sha256 for DuckDB v$ver on $platform: add it to scripts/install-duckdb.sh"; exit 1 ;;
esac

dir="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/libduckdb"
mkdir -p "$dir"
curl -fsSL --retry 3 --max-time 300 -o "$dir/libs.tar.gz" \
  "https://github.com/duckdb/duckdb/releases/download/v$ver/duckdb-shared-libs-$platform.tar.gz"
echo "$sum  $dir/libs.tar.gz" > "$dir/libs.sha256"
if command -v sha256sum >/dev/null 2>&1; then sha256sum -c "$dir/libs.sha256"; else shasum -a 256 -c "$dir/libs.sha256"; fi
tar xzf "$dir/libs.tar.gz" -C "$dir"

# visible to the check below (this process) and to later steps (GITHUB_ENV)
export DUCKDB_LIB_DIR="$dir" DUCKDB_INCLUDE_DIR="$dir"
if [ -n "${GITHUB_ENV:-}" ]; then
  {
    echo "DUCKDB_LIB_DIR=$dir"
    echo "DUCKDB_INCLUDE_DIR=$dir"
    echo "LD_LIBRARY_PATH=$dir"
    echo "DYLD_FALLBACK_LIBRARY_PATH=$dir"
  } >> "$GITHUB_ENV"
fi
sh scripts/duckdb-check.sh
