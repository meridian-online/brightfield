#!/usr/bin/env bash
# Check what scripts/build-duckdb.sh built, the managed edition's DuckDB.
#
#   scripts/build-duckdb-selftest.sh [DIR]
#
# DIR is the build's output directory, target/duckdb-managed/ by default. Every
# check runs the built files; none reads the build's configuration. It checks:
#
#   1. the shell's `--version` is exactly what the pinned release prints, which
#      arc compares with its table of known DuckDB release commits;
#   2. the seven built-in extensions are loaded and statically linked, read
#      from `duckdb_extensions()`, in the shell and so in the library it loads,
#      and a database opened read-only is read, which a FineType that panics
#      when its macros are refused would abort;
#   3. no `.duckdb_extension` file is anywhere in the output, and nothing is
#      written to an empty extension directory and HOME while the shell runs;
#   4. an extension outside the seven cannot be installed with the network
#      denied. (The release shell refuses it too, for want of the network; the
#      managed edition's refusal of INSTALL and LOAD whatever the network is
#      the engine's, not the build's.)
#   5. the shell links exactly one DuckDB, @rpath/libduckdb.dylib, binds
#      DuckDB's symbols from it rather than carrying its own, and at run time loads that library and no other, both
#      from the output layout (bin/ beside lib/) and from an app bundle's
#      (Contents/Helpers/ beside Contents/Frameworks/);
#   6. the library's install name is @rpath/libduckdb.dylib;
#   7. GEOS, which is LGPL-2.1, is a library of its own beside DuckDB's, which
#      links it by @rpath and carries none of GEOS's code itself, and from an app
#      bundle the shell loads it from Contents/Frameworks/.
#
# It ends by printing the library's and the shell's sizes.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
DIR="${1:-$("${HERE}/build-duckdb.sh" --print-dir)}"
LIB="${DIR}/lib/libduckdb.dylib"
CLI="${DIR}/bin/duckdb"

EXPECTED_VERSION="v2.0.0 (Cyanoptera) d54cbbe3df"
BUILT_IN="autocomplete excel sqlite_scanner httpfs ducklake spatial finetype"
NETWORK_DENIED="(version 1)(allow default)(deny network*)"

failures=0
pass() { echo "ok   $*"; }
fail() { echo "FAIL $*"; failures=$((failures + 1)); }

[ -f "$LIB" ] || { echo "build-duckdb-selftest: no library at ${LIB}; run scripts/build-duckdb.sh" >&2; exit 1; }
[ -x "$CLI" ] || { echo "build-duckdb-selftest: no shell at ${CLI}; run scripts/build-duckdb.sh" >&2; exit 1; }

SCRATCH=$(mktemp -d)
trap 'rm -rf "$SCRATCH"' EXIT
mkdir -p "${SCRATCH}/home" "${SCRATCH}/extensions"

# Run the shell as arc does: agent mode off, no init file, HOME and the
# extension directory empty.
duck() {
  HOME="${SCRATCH}/home" DUCKDB_AGENT_MODE=0 "$CLI" -no-init -batch -noheader -list \
    -c "SET extension_directories = ['${SCRATCH}/extensions'];" "$@"
}

# 1. The version.
version=$(DUCKDB_AGENT_MODE=0 "$CLI" --version)
if [ "$version" = "$EXPECTED_VERSION" ]; then
  pass "--version prints '${version}'"
else
  fail "--version prints '${version}', not '${EXPECTED_VERSION}'"
fi

# 2. The extensions.
linked=$(duck -c "SELECT extension_name FROM duckdb_extensions() WHERE loaded AND install_mode = 'STATICALLY_LINKED' ORDER BY 1;")
for ext in $BUILT_IN; do
  if printf '%s\n' "$linked" | grep -qx "$ext"; then
    pass "${ext} is loaded and statically linked"
  else
    fail "${ext} is not loaded and statically linked; the shell reports: $(printf '%s' "$linked" | tr '\n' ' ')"
  fi
done
# Each one answers, not only lists: a function from each, in the shell.
probe=$(duck -c "SELECT
  (SELECT count(*) > 0 FROM sql_auto_complete('SELE')),
  ST_AsText(ST_Point(1, 2)),
  ft_version() IS NOT NULL;" 2>&1) || true
if [ "$probe" = "true|POINT (1 2)|true" ]; then
  pass "autocomplete, spatial and FineType answer in the shell"
else
  fail "a built-in extension's function did not answer: ${probe}"
fi
# A database opened read-only still opens. FineType is compiled in, so its
# entry point runs on every open, and it registers two table macros with a
# CREATE that a read-only database refuses. A FineType that panics on that
# refusal aborts the shell here (exit 134), as it would abort arc, which opens
# a run's database read-only to measure its tables.
duck "${SCRATCH}/read-only.duckdb" -c "CREATE TABLE t AS SELECT 42 AS answer;" > /dev/null 2>&1 || true
status=0
read_only=$(duck -readonly "${SCRATCH}/read-only.duckdb" -c "SELECT answer, ft_version() IS NOT NULL FROM t;" 2>&1) || status=$?
if [ "$status" = 0 ] && [ "$read_only" = "42|true" ]; then
  pass "a database opened read-only is read, and FineType answers there"
else
  fail "a database opened read-only: exit ${status}, output: ${read_only}"
fi

# 3. No extension files.
files=$(find "$DIR/lib" "$DIR/bin" -name '*.duckdb_extension' 2>/dev/null)
written=$(find "${SCRATCH}/home" "${SCRATCH}/extensions" -type f 2>/dev/null)
if [ -z "$files" ] && [ -z "$written" ]; then
  pass "no .duckdb_extension file is in the output, and none was written while it ran"
else
  fail "extension files: ${files} ${written}"
fi

# 4. Nothing else installs.
if out=$(HOME="${SCRATCH}/home" DUCKDB_AGENT_MODE=0 sandbox-exec -p "$NETWORK_DENIED" "$CLI" -no-init -batch \
    -c "SET extension_directories = ['${SCRATCH}/extensions'];" -c "INSTALL h3 FROM community;" 2>&1); then
  fail "INSTALL h3 succeeded with the network denied: ${out}"
else
  pass "INSTALL h3 is refused"
fi

# 5. One DuckDB, by rpath.
links=$(otool -L "$CLI" | tail -n +2 | awk '{print $1}' | grep -i duckdb || true)
if [ "$links" = "@rpath/libduckdb.dylib" ]; then
  pass "the shell links @rpath/libduckdb.dylib and no other DuckDB"
else
  fail "the shell links: ${links:-no DuckDB library}"
fi
# A shell with a DuckDB of its own binds none of DuckDB's symbols from outside
# itself; one that loads the library leaves them undefined, to be bound from it.
imported=$(nm -uj "$CLI" | grep -c 'duckdb' || true)
if [ "$imported" -gt 0 ]; then
  pass "the shell binds ${imported} of DuckDB's symbols from the library, so carries none of its own"
else
  fail "the shell binds none of DuckDB's symbols from a library, so it carries a DuckDB of its own"
fi
loaded_from() {
  # dyld names each image it loads on stderr; keep the DuckDB ones.
  DYLD_PRINT_LIBRARIES=1 HOME="${SCRATCH}/home" DUCKDB_AGENT_MODE=0 "$1" -no-init -batch -c "SELECT 1;" 2>&1 >/dev/null \
    | sed -n 's/^dyld\[[0-9]*\]: <[^>]*> //p' | grep -i '/[^/]*duckdb[^/]*$' \
    | grep -v "^$(cd "$(dirname "$1")" && pwd -P)/duckdb$" || true
}
expected_lib=$(cd "$(dirname "$LIB")" && pwd -P)/libduckdb.dylib
from_output=$(loaded_from "$CLI")
if [ "$from_output" = "$expected_lib" ]; then
  pass "run from bin/, the shell loads ${expected_lib} and no other DuckDB"
else
  fail "run from bin/, the shell loads: ${from_output:-no DuckDB library}"
fi
BUNDLE="${SCRATCH}/Brightfield.app/Contents"
mkdir -p "${BUNDLE}/Helpers" "${BUNDLE}/Frameworks"
cp "$CLI" "${BUNDLE}/Helpers/duckdb"
cp "${DIR}"/lib/*.dylib "${BUNDLE}/Frameworks/"
bundle_lib=$(cd "${BUNDLE}/Frameworks" && pwd -P)/libduckdb.dylib
from_bundle=$(loaded_from "${BUNDLE}/Helpers/duckdb")
if [ "$from_bundle" = "$bundle_lib" ]; then
  pass "in an app bundle's Helpers, the shell loads Contents/Frameworks/libduckdb.dylib and no other DuckDB"
else
  fail "in an app bundle's Helpers, the shell loads: ${from_bundle:-no DuckDB library}"
fi

# Every file's rpaths are the ones the build gives it, so none reaches into a
# build tree: @loader_path for each library, and bin/'s and an app's Helpers'
# two layouts for the shell.
rpaths() { otool -l "$1" | awk '/cmd LC_RPATH/ {getline; getline; print $2}' | tr '\n' ' '; }
bad_rpaths=""
for lib in "${DIR}"/lib/*.dylib; do
  [ "$(rpaths "$lib")" = "@loader_path " ] || bad_rpaths="${bad_rpaths} $(basename "$lib"): $(rpaths "$lib");"
done
[ "$(rpaths "$CLI")" = "@executable_path/../lib @executable_path/../Frameworks " ] \
  || bad_rpaths="${bad_rpaths} duckdb: $(rpaths "$CLI");"
if [ -z "$bad_rpaths" ]; then
  pass "every library's only rpath is @loader_path, and the shell's are its two layouts"
else
  fail "rpaths other than the build's own:${bad_rpaths}"
fi

# 6. The library's install name.
id=$(otool -D "$LIB" | tail -n +2)
if [ "$id" = "@rpath/libduckdb.dylib" ]; then
  pass "the library's install name is @rpath/libduckdb.dylib"
else
  fail "the library's install name is ${id}"
fi

# 7. GEOS, its own library.
geos_links=$(otool -L "$LIB" | tail -n +2 | awk '{print $1}' | grep -i geos || true)
if [ -n "$geos_links" ] && ! printf '%s\n' "$geos_links" | grep -qv '^@rpath/libgeos'; then
  pass "the library links GEOS by rpath: $(printf '%s' "$geos_links" | tr '\n' ' ')"
else
  fail "the library links GEOS as: ${geos_links:-nothing}"
fi
if nm -gUj "$LIB" | grep -qE '^_GEOS'; then
  fail "the library defines GEOS's functions itself, so GEOS is linked into it"
else
  pass "the library carries none of GEOS's code"
fi
geos_loaded=$(DYLD_PRINT_LIBRARIES=1 HOME="${SCRATCH}/home" DUCKDB_AGENT_MODE=0 "${BUNDLE}/Helpers/duckdb" -no-init -batch \
  -c "SELECT ST_AsText(ST_Buffer(ST_Point(0, 0), 1)) IS NOT NULL;" 2>&1 >/dev/null \
  | sed -n 's/^dyld\[[0-9]*\]: <[^>]*> //p' | grep -i '/libgeos[^/]*$' || true)
frameworks=$(cd "${BUNDLE}/Frameworks" && pwd -P)
if [ -n "$geos_loaded" ] && ! printf '%s\n' "$geos_loaded" | grep -qv "^${frameworks}/"; then
  pass "in an app bundle, GEOS loads from Contents/Frameworks"
else
  fail "in an app bundle, GEOS loads from: ${geos_loaded:-nowhere}"
fi

for lib in "${DIR}"/lib/*.dylib; do
  echo "size: $(basename "$lib") $(stat -f %z "$lib") bytes"
done
echo "size: duckdb $(stat -f %z "$CLI") bytes"
if [ "$failures" -gt 0 ]; then
  echo "build-duckdb-selftest: ${failures} check(s) failed" >&2
  exit 1
fi
echo "build-duckdb-selftest: all checks passed"
