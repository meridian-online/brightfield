#!/usr/bin/env bash
# Print the DuckDB platform name for a Rust target triple.
#
#   scripts/duckdb-platform.sh aarch64-apple-darwin   -> osx_arm64
#
# ONE table, because two callers need it and they must not drift:
# scripts/package.sh checks the staged extension's platform stamp against the
# target it is packaging for, and scripts/fetch-finetype-bundle.sh asks
# FineType's release for the extension built for that same target. A second
# copy of this mapping is how an arm64 extension ends up inside the x86_64
# artefact — a tarball that fails on every machine it was built FOR and works
# on the one machine it will never be tested on.
#
# An unknown target is a hard failure, not an unchecked pass. A mapping nobody
# added is a bundle nobody checked.
set -euo pipefail

TARGET="${1:?usage: scripts/duckdb-platform.sh RUST_TARGET_TRIPLE}"

case "$TARGET" in
  aarch64-apple-darwin)      echo osx_arm64 ;;
  x86_64-apple-darwin)       echo osx_amd64 ;;
  aarch64-unknown-linux-gnu) echo linux_arm64 ;;
  x86_64-unknown-linux-gnu)  echo linux_amd64 ;;
  *)
    echo "duckdb-platform: no DuckDB platform name known for target ${TARGET}." >&2
    echo "  Add the mapping here; every caller reads this one table." >&2
    exit 1 ;;
esac
