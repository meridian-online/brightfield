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
#      when its macros are refused would abort; FineType's two table macros
#      answer on a database opened read-write and on one opened read-only, and
#      the database holds neither afterwards;
#   3. no `.duckdb_extension` file is anywhere in the output, and nothing is
#      written to an empty extension directory and HOME while the shell runs;
#   4. with the network available, the shell installs nothing and loads no
#      file: INSTALL of an extension outside the built-ins, of one compiled in,
#      and of one FROM core, FORCE INSTALL and a LOAD of a file path each fail
#      with DuckDB's refusal for a build without its extension loader, and
#      nothing is written (UPDATE EXTENSIONS reinstalls only the files in the
#      extension directory, so with none installed it has nothing to fetch);
#      a LOAD of a built-in still succeeds, and httpfs still reads a file
#      over HTTP, from a server on this machine;
#   5. the shell links exactly one DuckDB, @rpath/libduckdb.dylib, binds
#      DuckDB's symbols from it rather than carrying its own, and at run time loads that library and no other, both
#      from the output layout (bin/ beside lib/) and from an app bundle's
#      (Contents/Helpers/ beside Contents/Frameworks/);
#   6. the library's install name is @rpath/libduckdb.dylib;
#   7. GEOS, which is LGPL-2.1, is a library of its own beside DuckDB's, which
#      links it by @rpath and carries none of GEOS's code itself, and from an app
#      bundle the shell loads it from Contents/Frameworks/;
#   8. licenses/ holds the notices the Store bundle carries: GEOS's LGPL-2.1
#      text and a SOURCE.txt naming each GEOS library in lib/ and the commit
#      its source is at, the C libraries' licences, DuckDB's, FineType's, and
#      FineType's Rust crates.
#
# It ends by printing the library's and the shell's sizes.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
DIR="${1:-$("${HERE}/build-duckdb.sh" --print-dir)}"
LIB="${DIR}/lib/libduckdb.dylib"
CLI="${DIR}/bin/duckdb"

EXPECTED_VERSION="v2.0.0 (Cyanoptera) d54cbbe3df"
BUILT_IN="autocomplete excel sqlite_scanner httpfs ducklake spatial finetype"

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
# FineType's two table macros answer however the database was opened, and are
# written into none. The entry point registers them in DuckDB's system catalog;
# a FineType left to CREATE them itself would write both into the read-write
# database here and have neither in a database it could not write. The
# read-only database is one the macros were never stored in, since the shell
# above created it. ft_validate is run. ft_profile is only planned, with
# EXPLAIN, which binds the macro and its SQL without running it: run, it would
# fetch FineType's model over the network into HOME.
MACRO_CALLS="SELECT count(*) FROM ft_validate('t', '{\"properties\": {\"answer\": {\"type\": \"integer\"}}}'); EXPLAIN FROM ft_profile('t');"
STORED_MACROS="SELECT count(*) FROM duckdb_functions() WHERE function_name IN ('ft_validate', 'ft_profile') AND function_type = 'table_macro' AND (NOT internal OR database_name = current_database());"
for mode in read-write read-only; do
  flag=(); [ "$mode" = read-only ] && flag=(-readonly)
  status=0
  answered=$(duck ${flag[@]+"${flag[@]}"} "${SCRATCH}/read-only.duckdb" -c "$MACRO_CALLS" 2>&1) || status=$?
  if [ "$status" = 0 ] && [ "$(printf '%s\n' "$answered" | head -1)" = "1" ]; then
    pass "ft_validate answers and ft_profile binds, as table macros, on a database opened ${mode}"
  else
    fail "FineType's table macros on a database opened ${mode}: exit ${status}, output: $(printf '%s' "$answered" | head -5)"
  fi
done
stored=$(duck -readonly "${SCRATCH}/read-only.duckdb" -c "$STORED_MACROS" 2>&1) || true
if [ "$stored" = "0" ]; then
  pass "the database holds neither macro after being opened read-write"
else
  fail "the database holds FineType's table macros, or the query failed: ${stored}"
fi

# 3. No extension files.
files=$(find "$DIR/lib" "$DIR/bin" -name '*.duckdb_extension' 2>/dev/null)
written=$(find "${SCRATCH}/home" "${SCRATCH}/extensions" -type f 2>/dev/null)
if [ -z "$files" ] && [ -z "$written" ]; then
  pass "no .duckdb_extension file is in the output, and none was written while it ran"
else
  fail "extension files: ${files} ${written}"
fi

# 4. Nothing installs and no file loads, with the network available. The
# refusal is the one DuckDB gives when the build leaves its extension loader
# out (ENABLE_EXTENSION_LOAD=OFF in scripts/build-duckdb.sh), which it raises
# before it looks at the network or the disk, so a refusal for want of the
# network, as DuckDB's release gives offline, does not pass here.
REFUSED="external extensions is not supported: this build does not link the loadable_extensions library"
printf 'not an extension' > "${SCRATCH}/h3.duckdb_extension"
for statement in "INSTALL h3 FROM community;" "INSTALL h3;" "INSTALL excel;" "INSTALL json FROM core;" \
    "FORCE INSTALL spatial;" "LOAD '${SCRATCH}/h3.duckdb_extension';"; do
  status=0
  out=$(duck -c "$statement" 2>&1) || status=$?
  if [ "$status" != 0 ] && printf '%s' "$out" | grep -qF "$REFUSED"; then
    pass "${statement} is refused, by a build with no extension loader"
  else
    fail "${statement}: exit ${status}, output: ${out}"
  fi
done
written=$(find "${SCRATCH}/home" "${SCRATCH}/extensions" -type f 2>/dev/null)
if [ -z "$written" ]; then
  pass "the refused statements wrote nothing to HOME or the extension directory"
else
  fail "the refused statements wrote: ${written}"
fi
loaded=$(duck -c "LOAD excel; LOAD spatial; SELECT 'loaded';" 2>&1) || true
if [ "$loaded" = "loaded" ]; then
  pass "a LOAD of a built-in extension succeeds"
else
  fail "a LOAD of a built-in extension: ${loaded}"
fi
# httpfs reads a URL with its own HTTP client, which the build keeps when it
# leaves DuckDB's built-in one out with the extension loader.
mkdir -p "${SCRATCH}/served"
printf 'answer\n42\n' > "${SCRATCH}/served/answer.csv"
port_file="${SCRATCH}/port"
python3 -I -c '
import http.server, socketserver, sys, functools
class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *args):
        pass
handler = functools.partial(Quiet, directory=sys.argv[1])
with socketserver.TCPServer(("127.0.0.1", 0), handler) as httpd:
    open(sys.argv[2], "w").write(str(httpd.server_address[1]))
    httpd.serve_forever()
' "${SCRATCH}/served" "$port_file" &
server=$!
for _ in $(seq 50); do [ -s "$port_file" ] && break; sleep 0.1; done
served=$(duck -c "SELECT answer FROM read_csv('http://127.0.0.1:$(cat "$port_file" 2>/dev/null)/answer.csv');" 2>&1) || true
kill "$server" 2>/dev/null || true
wait "$server" 2>/dev/null || true
if [ "$served" = "42" ]; then
  pass "httpfs reads a CSV over HTTP"
else
  fail "httpfs reading a CSV over HTTP: ${served}"
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

# 8. The licence notices.
NOTICES="${DIR}/licenses"
geos_named=1
for lib in "${DIR}"/lib/libgeos*.dylib; do
  grep -qF "Contents/Frameworks/$(basename "$lib")" "${NOTICES}/GEOS/SOURCE.txt" 2>/dev/null || geos_named=0
done
if [ "$geos_named" = 1 ] && grep -q 'Version 2.1' "${NOTICES}/GEOS/COPYING" 2>/dev/null \
   && grep -Eq 'libgeos/geos/tree/[0-9a-f]{40}' "${NOTICES}/GEOS/SOURCE.txt"; then
  pass "licenses/GEOS holds the LGPL-2.1 and a source notice naming each GEOS library and its commit"
else
  fail "licenses/GEOS does not hold the LGPL-2.1 and a source notice naming each GEOS library in lib/"
fi
missing_notices=""
for path in duckdb/LICENSE finetype/LICENSE c-libraries/gdal.txt rust-crates/finetype-in-libduckdb.txt rust-crates/finetype-command-line.txt; do
  [ -s "${NOTICES}/${path}" ] || missing_notices="${missing_notices} ${path}"
done
if [ -z "$missing_notices" ]; then
  pass "licenses/ holds DuckDB's, FineType's, the C libraries' and FineType's crates' notices"
else
  fail "licenses/ has no${missing_notices}"
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
