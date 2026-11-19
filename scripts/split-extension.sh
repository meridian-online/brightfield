#!/usr/bin/env bash
# split-extension.sh EXT DEST_DIR — cut a DuckDB loadable extension in two at
# the end of its Mach-O: DEST_DIR/finetype.dylib is the shared library alone,
# and DEST_DIR/finetype.duckdb_extension.trailer is DuckDB's metadata trailer,
# byte for byte.
#
# WHY: notarisation requires every Mach-O in the app to pass codesign's strict
# validation, and codesign cannot sign a file with bytes after the end of its
# Mach-O, which is exactly where DuckDB puts its trailer. Cut there, the library
# is an ordinary dylib that codesign signs and verifies on its own, and the
# trailer is data. The engine joins the two back into one file before DuckDB's
# LOAD, which needs the trailer at the end
# (`brightfield_engine::semantic::FinetypeBundle::open`). scripts/package.sh
# runs this for the app only; the tarball keeps the joined file.
#
# The names are the engine's: `semantic::SPLIT_LIBRARY_FILE` and
# `semantic::SPLIT_TRAILER_FILE`.
#
# This does not sign the library; the caller does, with whichever identity it
# has. EXT is left in place. Exit 0 done, 1 refused (not a dylib, no trailer,
# the two parts do not rejoin to EXT).
set -euo pipefail

EXT="${1:?usage: scripts/split-extension.sh EXTENSION DEST_DIR}"
DEST="${2:?usage: scripts/split-extension.sh EXTENSION DEST_DIR}"
fail() { echo "split-extension: $*" >&2; exit 1; }

[ -f "$EXT" ] || fail "no file at ${EXT}"
[ -d "$DEST" ] || fail "no directory at ${DEST}"
command -v otool >/dev/null 2>&1 || fail "otool is not on PATH (it is a macOS developer tool)"

# `otool -D` prints the file name, then the install name; a file that is not a
# dylib prints the file name alone.
[ -n "$(otool -D "$EXT" | sed -n 2p)" ] || fail "${EXT} has no install name — it is not a Mach-O dylib"

# shellcheck source=scripts/lib/extension-macho-end.sh
. "$(dirname "$0")/lib/extension-macho-end.sh"
macho_end=$(extension_macho_end "$EXT")
[ -n "$macho_end" ] || fail "${EXT} has no __LINKEDIT segment"
size=$(wc -c < "$EXT" | tr -d ' ')
trailer_len=$((size - macho_end))
# 512 bytes is the fixed tail DuckDB reads its metadata fields from; a shorter
# remainder means the file was never stamped, or the cut is in the wrong place.
[ "$trailer_len" -ge 512 ] \
  || fail "${EXT} carries ${trailer_len} bytes after __LINKEDIT — shorter than DuckDB's metadata trailer"

LIB="$DEST/finetype.dylib"
TRAILER="$DEST/finetype.duckdb_extension.trailer"
head -c "$macho_end" "$EXT" > "$LIB"
tail -c "$trailer_len" "$EXT" > "$TRAILER"

# The engine loads `cat library trailer`, so that has to be the file packaging
# was handed, or the manifest would vouch for a pair that is not the extension.
cat "$LIB" "$TRAILER" | cmp -s - "$EXT" \
  || fail "${LIB} and ${TRAILER} do not rejoin to ${EXT}"

echo "   ${EXT##*/}: split into ${LIB##*/} (${macho_end} bytes) and ${TRAILER##*/} (${trailer_len} bytes)"
