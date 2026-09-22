#!/usr/bin/env bash
# Fetch the DuckDB command-line shell that `arc run` executes a Protocol's SQL
# steps with, at the one version this workspace pins, for one Rust target.
#
#   scripts/fetch-duckdb-cli.sh RUST_TARGET DEST_DIR
#       downloads the official release zip, checks its sha256 against the pin
#       below BEFORE unpacking it, unpacks `duckdb` into DEST_DIR, checks the
#       unpacked executable's sha256 too, and prints its path.
#
#   scripts/fetch-duckdb-cli.sh --check RUST_TARGET DIR
#       no network: exits 0 only when DIR/duckdb is an executable file whose
#       sha256 is the pinned one for RUST_TARGET. scripts/package.sh runs this
#       over BRIGHTFIELD_DUCKDB_CLI before it stages anything, so a directory
#       holding some other duckdb — a Homebrew one, the other architecture's —
#       is refused rather than shipped.
#
# WHY A CLI AT ALL. `arc run` does not embed DuckDB: it spawns the `duckdb`
# executable once per SQL step. It resolves that executable from ARC_DUCKDB_BIN
# when the variable is set and from the search path when it is not, and neither
# a macOS app started from Finder nor a CI runner has a duckdb on its search
# path. So a packaged Brightfield carries its own — scripts/package.sh stages
# it — and the shell's Run control names it to the child in ARC_DUCKDB_BIN
# (crates/brightfield-shell/src/run.rs, `staged_engine_beside`).
#
# WHY THIS VERSION. v1.5.2 is the DuckDB that `libduckdb-sys` 1.10502.0 links
# into this binary (Cargo.lock), so the engine that writes a run and the
# engine that reads the result in the window are one version. Moving the
# embedded DuckDB moves this pin with it: the version, the two assets and all
# four hashes below.
#
# WHAT IS PINNED. Both hashes per target were measured on the official assets
# at https://github.com/duckdb/duckdb/releases/tag/v1.5.2, and the zip hashes
# agree with the digests GitHub publishes for those assets. The per-architecture
# zips are used rather than `duckdb_cli-osx-universal.zip`, which carries both
# slices: 34.8 MB to download against 16.3 MB (arm64) and 18.5 MB (x86_64).
# The executables are signed by the DuckDB Foundation with the hardened runtime
# and a timestamp; nothing here re-signs them, so the signature a user can
# check is the upstream one.
set -euo pipefail

usage="usage: scripts/fetch-duckdb-cli.sh [--check] RUST_TARGET DIR"
fail() { echo "fetch-duckdb-cli: $*" >&2; exit 1; }

MODE=fetch
if [ "${1:-}" = "--check" ]; then
  MODE=check
  shift
fi
TARGET="${1:?$usage}"
DIR="${2:?$usage}"

VERSION="v1.5.2"
case "$TARGET" in
  aarch64-apple-darwin)
    ASSET="duckdb_cli-osx-arm64.zip"
    ZIP_SHA256="d5289966c3284b432afc7bf064b8a134ba38db0597e1f3559f0aba4ce23c5ea8"
    BIN_SHA256="5f5fafb02b609cdb20d199c06835d095023616e7366033775ba99a6a0b6969f3"
    ;;
  x86_64-apple-darwin)
    ASSET="duckdb_cli-osx-amd64.zip"
    ZIP_SHA256="67c79301e25bf2289aec81a33131b4d3bfecef0fb4074cf38771e63de6da9c38"
    BIN_SHA256="091b85ffe0b8db434723238d64fad363509d3ce3f65783c050f71f441827b90e"
    ;;
  *)
    fail "no pinned DuckDB CLI for target ${TARGET}. The two darwin targets
  release.yml packages are pinned here; add a target with both of its hashes,
  measured on the ${VERSION} release assets."
    ;;
esac
URL="https://github.com/duckdb/duckdb/releases/download/${VERSION}/${ASSET}"

sha256_of() { shasum -a 256 "$1" | awk '{print $1}'; }

# The executable check both modes end on: present, executable, and the pinned
# bytes. A hash that differs is named with both values, so a bump that moved
# one hash and not the other says which.
check_binary() {
  local bin="$1/duckdb" got
  [ -f "$bin" ] || fail "no duckdb executable at ${bin}"
  [ -x "$bin" ] || fail "${bin} is not executable"
  got=$(sha256_of "$bin")
  [ "$got" = "$BIN_SHA256" ] || fail "${bin} is not the pinned DuckDB ${VERSION} CLI for ${TARGET}
  sha256 wanted ${BIN_SHA256}
  sha256 found  ${got}"
}

if [ "$MODE" = check ]; then
  check_binary "$DIR"
  echo "fetch-duckdb-cli: ${DIR}/duckdb is DuckDB ${VERSION} for ${TARGET} (sha256 ${BIN_SHA256})"
  exit 0
fi

mkdir -p "$DIR"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
curl -sSfL --retry 3 --retry-delay 2 -o "${WORK}/${ASSET}" "$URL" \
  || fail "could not download ${URL}"
got=$(sha256_of "${WORK}/${ASSET}")
[ "$got" = "$ZIP_SHA256" ] || fail "${ASSET} does not match its pin; refusing to unpack it
  sha256 wanted ${ZIP_SHA256}
  sha256 found  ${got}"
unzip -o -q "${WORK}/${ASSET}" duckdb -d "$DIR" || fail "${ASSET} holds no 'duckdb' entry"
chmod 755 "${DIR}/duckdb"
check_binary "$DIR"
echo "${DIR}/duckdb"
