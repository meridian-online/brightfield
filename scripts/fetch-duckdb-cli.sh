#!/usr/bin/env bash
# Fetch the DuckDB command-line shell that `arc run` executes a Protocol's SQL
# steps with, at the one version this workspace pins, for one Rust target.
#
#   scripts/fetch-duckdb-cli.sh RUST_TARGET DEST_DIR
#       downloads the official release archive, checks its sha256 against the
#       pin below BEFORE unpacking it, unpacks `duckdb` into DEST_DIR, checks the
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
# WHY THIS VERSION. v2.0.0 is the DuckDB that `libduckdb-sys` 1.20000.0 links
# into this binary (Cargo.lock), so the engine that writes a run and the
# engine that reads the result in the window are one version. Moving the
# embedded DuckDB moves this pin with it: the version, the two assets and all
# four hashes below. scripts/check-artifact-engine.sh refuses a release whose
# VERSION here and libduckdb-sys in Cargo.lock disagree.
#
# WHAT IS PINNED. Both hashes per target were measured on the official assets
# at https://github.com/duckdb/duckdb/releases/tag/v2.0.0, and the archive hashes
# agree with the digests GitHub publishes for those assets. From 2.0 DuckDB
# publishes the CLI as a `.tar.gz` named `duckdb-cli-<platform>` (1.5 published
# `duckdb_cli-<platform>.zip`). The per-architecture archives are used rather than
# `duckdb-cli-osx-universal.tar.gz`, which carries both slices: 40.6 MB to
# download against 19.2 MB (arm64) and 21.5 MB (x86_64).
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

VERSION="v2.0.0"
case "$TARGET" in
  aarch64-apple-darwin)
    ASSET="duckdb-cli-osx-arm64.tar.gz"
    ARCHIVE_SHA256="188a36fe8883ef52e69165a0cbb6031ef679c8e6c448a1ce22951964149b7f96"
    BIN_SHA256="432e9c8012d0259e7c4630fa2136bf59fb832769ca7e0f6e374b8af2d2cebed9"
    ;;
  x86_64-apple-darwin)
    ASSET="duckdb-cli-osx-amd64.tar.gz"
    ARCHIVE_SHA256="fbc7013ba09d117396dabf16783281c2add47fc01cdac4855464a03526dc96ee"
    BIN_SHA256="3abb2f746efe011979290d40448b8ac011b7fd7c55094ac00e7beeb8482c5ac0"
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
[ "$got" = "$ARCHIVE_SHA256" ] || fail "${ASSET} does not match its pin; refusing to unpack it
  sha256 wanted ${ARCHIVE_SHA256}
  sha256 found  ${got}"
tar -xzf "${WORK}/${ASSET}" -C "$DIR" duckdb || fail "${ASSET} holds no 'duckdb' entry"
chmod 755 "${DIR}/duckdb"
check_binary "$DIR"
echo "${DIR}/duckdb"
