#!/usr/bin/env bash
# Check that vendor/libduckdb-sys is crates.io's libduckdb-sys 1.20000.0 with
# Brightfield's one change and nothing else.
#
#   scripts/check-vendored-libduckdb-sys.sh
#
# Cargo.lock records no checksum for a path dependency, so once the crate is
# vendored nothing in Cargo checks its bytes. This does: it downloads the
# published crate, checks the archive against the sha256 crates.io's index
# records for it (the checksum Cargo.lock carried before the crate was
# vendored), unpacks it, applies Brightfield's change
# (vendor/libduckdb-sys/BRIGHTFIELD.patch) to the files it changes, and compares
# every file byte for byte. The files Brightfield added are listed below; any
# other difference fails, including an edit to a changed file that the patch
# does not record, and so does a listed file that is no longer there. A
# deliberate change to build.rs or Cargo.toml moves the patch with it, so the
# change is always a diff a reviewer reads.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
VENDORED="${HERE}/../vendor/libduckdb-sys"

VERSION="1.20000.0"
CRATE_SHA256="ac1dcdcfde48e11bbd46112ed3b0423acd8c644413b3891e7be8104edce7fdda"
URL="https://static.crates.io/crates/libduckdb-sys/libduckdb-sys-${VERSION}.crate"
# Changed, by BRIGHTFIELD.patch: build.rs, Cargo.toml. Added: managed_linked.rs,
# BRIGHTFIELD.md and the patch itself. Left out: .cargo-ok, Cargo's unpack marker.
CHANGED="build.rs Cargo.toml"
ADDED="managed_linked.rs BRIGHTFIELD.md BRIGHTFIELD.patch"
LEFT_OUT=".cargo-ok"

fail() { echo "check-vendored-libduckdb-sys: $*" >&2; exit 1; }

[ -d "$VENDORED" ] || fail "no vendored crate at ${VENDORED}"

SCRATCH=$(mktemp -d)
trap 'rm -rf "$SCRATCH"' EXIT

curl -fsSL --retry 3 -o "${SCRATCH}/crate.tar.gz" "$URL" || fail "could not download ${URL}"
got=$(shasum -a 256 "${SCRATCH}/crate.tar.gz" | awk '{print $1}')
[ "$got" = "$CRATE_SHA256" ] || fail "${URL} has sha256 ${got}, not the published ${CRATE_SHA256}"
tar -xzf "${SCRATCH}/crate.tar.gz" -C "$SCRATCH"
PUBLISHED="${SCRATCH}/libduckdb-sys-${VERSION}"
(cd "$PUBLISHED" && patch -s -p0 --no-backup-if-mismatch < "${VENDORED}/BRIGHTFIELD.patch") \
  || fail "BRIGHTFIELD.patch does not apply to the published crate"
patched=$(grep -E '^\+\+\+ ' "${VENDORED}/BRIGHTFIELD.patch" | awk '{print $2}' | sort | tr '\n' ' ')
[ "$patched" = "$(printf '%s\n' $CHANGED | sort | tr '\n' ' ')" ] \
  || fail "BRIGHTFIELD.patch changes '${patched}', not the files listed as changed: ${CHANGED}"

listed() { case " $2 " in *" $1 "*) return 0 ;; esac; return 1; }

problems=0
while IFS= read -r -d '' file; do
  rel=${file#"${PUBLISHED}/"}
  if listed "$rel" "$LEFT_OUT"; then
    continue
  fi
  if [ ! -f "${VENDORED}/${rel}" ]; then
    echo "missing from the vendored crate: ${rel}" >&2
    problems=$((problems + 1))
  elif ! cmp -s "$file" "${VENDORED}/${rel}"; then
    if listed "$rel" "$CHANGED"; then
      echo "differs from the published file with BRIGHTFIELD.patch applied: ${rel}" >&2
    else
      echo "differs from the published crate: ${rel}" >&2
    fi
    problems=$((problems + 1))
  fi
done < <(find "$PUBLISHED" -type f -print0)

while IFS= read -r -d '' file; do
  rel=${file#"${VENDORED}/"}
  if [ ! -e "${PUBLISHED}/${rel}" ] && ! listed "$rel" "$ADDED"; then
    echo "not in the published crate, and not listed as added: ${rel}" >&2
    problems=$((problems + 1))
  fi
done < <(find "$VENDORED" -type f -print0)

for rel in $ADDED; do
  [ -f "${VENDORED}/${rel}" ] || { echo "listed as added but missing: ${rel}" >&2; problems=$((problems + 1)); }
done

[ "$problems" -eq 0 ] || fail "${problems} difference(s) from crates.io's libduckdb-sys ${VERSION} beyond Brightfield's change"
echo "vendor/libduckdb-sys is crates.io's libduckdb-sys ${VERSION} (sha256 ${CRATE_SHA256}) with BRIGHTFIELD.patch applied to ${CHANGED}, and ${ADDED} added"
