#!/usr/bin/env bash
# Build the managed edition's DuckDB: one library, libduckdb.dylib, with seven
# extensions compiled into it, and DuckDB's own command-line shell linked
# against that library rather than carrying a copy of its own.
#
#   scripts/build-duckdb.sh            build into target/duckdb-managed/
#   scripts/build-duckdb.sh --print-dir
#                                      print the output directory and exit
#
# Output, under target/duckdb-managed/ (BRIGHTFIELD_DUCKDB_MANAGED_DIR moves it):
#   lib/libduckdb.dylib   DuckDB v2.0.0 at d54cbbe3df, its install name
#                         @rpath/libduckdb.dylib
#   lib/libgeos*.dylib    GEOS, which spatial uses, as its own library beside
#                         it, since GEOS is LGPL-2.1
#   include/              duckdb.h and the other C API headers
#   bin/duckdb            DuckDB's shell, which loads lib/libduckdb.dylib through
#                         its rpaths: @executable_path/../lib here, and
#                         @executable_path/../Frameworks for an app bundle's
#                         Contents/Helpers/
#   licenses/             the licence notices of the third-party code in lib/
#                         and of FineType's command line, which
#                         scripts/package-store.sh puts in the app
#                         (scripts/duckdb-managed/notices/notices.py)
#   sizes.txt             the sizes of the libraries and the shell
#
# WHY ONE LIBRARY. The Mac App Store edition runs DuckDB twice: in process, in
# Brightfield's window, and as the shell arc runs a Protocol's SQL steps with.
# Built once as a library both load, the app carries one copy of DuckDB and its
# extensions instead of two, and the engine that runs a Protocol and the engine
# that reads its result in the window are the same build.
#
# WHY THE EXTENSIONS ARE COMPILED IN. A Mac App Store app may not download code,
# and every piece of code in its bundle is signed with the developer's identity.
# A `.duckdb_extension` file cannot be signed: DuckDB appends its metadata and
# signature after the Mach-O image, and codesign refuses a file with bytes after
# its signature, as measured on macOS 27.2. So the extensions the edition needs are
# compiled into the library, and none ships as a file. Which extensions, and at
# which commits, is scripts/duckdb-managed/extensions.cmake.
#
# WHAT IS PINNED, AND HOW IT IS CHECKED. DuckDB's source is fetched by commit,
# and the checkout's HEAD is compared with the pin before anything builds; a git
# commit hash is a checksum of its whole tree. The extensions' GIT_TAGs in the
# configuration are commits too, checked out by hash by DuckDB's build. vcpkg,
# which builds the C libraries spatial, httpfs and excel need, is pinned to the
# commit DuckDB v2.0.0's own release builds use. FineType is fetched at the tag
# packaging/finetype-pin.env names, and its commit is compared with the one
# below, so a moved tag fails the build.
#
# NEEDS: git, cmake, ninja, python3 and a C/C++ toolchain (Xcode's), plus
# FineType's Rust toolchain, which rustup installs from its rust-toolchain.toml.
# A cold build takes 30 to 90 minutes, most of it spatial's GDAL, PROJ and GEOS
# under vcpkg; vcpkg keeps what it built in its binary cache (~/.cache/vcpkg), so
# a second checkout's build skips them. An unchanged rerun is minutes.
#
# BUILD SLOTS. On a shared machine at most three Rust or C++ builds should run
# at once. This build runs ninja and cargo with BRIGHTFIELD_DUCKDB_JOBS jobs
# (default 6), the cap each Claude Code session's cargo builds use.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "${HERE}/.." && pwd)
OUT="${BRIGHTFIELD_DUCKDB_MANAGED_DIR:-${ROOT}/target/duckdb-managed}"
CONFIG_DIR="${HERE}/duckdb-managed"

fail() { echo "build-duckdb: $*" >&2; exit 1; }
say() { echo "build-duckdb: $*" >&2; }

if [ "${1:-}" = "--print-dir" ]; then
  printf '%s\n' "$OUT"
  exit 0
fi
[ $# -eq 0 ] || fail "usage: scripts/build-duckdb.sh [--print-dir]"
# DuckDB's make passes its CMake variables through one space-separated string,
# and otool's output is read by field, so a path with a space in it cannot
# build. Say so before anything is fetched.
for path in "$OUT" "$ROOT" "${BRIGHTFIELD_VCPKG_ROOT:-}"; do
  case "$path" in
    *" "*) fail "a build path cannot have a space in it, and this one does: '${path}'" ;;
  esac
done

# Compiler flags from the environment would go into every C and C++ compile
# below: CMake takes CFLAGS and CXXFLAGS as its starting flags, and FineType's
# C dependencies read them too. Run through `cargo xtask`, this script inherits
# the CXXFLAGS that .cargo/config.toml sets for duckdb-rs's bundled DuckDB, a
# second DUCKDB_VERSION define without DuckDB's own quoting, which breaks
# DuckDB's version file. This build sets every flag it needs itself.
unset CFLAGS CXXFLAGS CPPFLAGS LDFLAGS

# The DuckDB release this edition runs: the version libduckdb-sys 1.20000.0
# links into the full edition, and the shell scripts/fetch-duckdb-cli.sh fetches.
DUCKDB_VERSION_PIN="v2.0.0"
DUCKDB_COMMIT="d54cbbe3df17e8e5ce68312f483b4155836112aa"
DUCKDB_URL="https://github.com/duckdb/duckdb"
# vcpkg at the commit DuckDB v2.0.0's release workflows pin ("Release 2026.06.24"
# in .github/workflows/Main.yml at the DuckDB commit above).
VCPKG_COMMIT="cd61e1e26a038e82d6550a3ebbe0fbbfe7da78e3"
VCPKG_URL="https://github.com/microsoft/vcpkg"
# FineType: the tag is packaging/finetype-pin.env's; this is the commit it names.
FINETYPE_URL="https://github.com/meridian-online/finetype"
FINETYPE_COMMIT="47f59e2bf6f1b6d05ce42d3ec8019832e368bb17"

JOBS="${BRIGHTFIELD_DUCKDB_JOBS:-6}"
case "$JOBS" in ''|*[!0-9]*) fail "BRIGHTFIELD_DUCKDB_JOBS='${JOBS}' is not a number" ;; esac

[ "$(uname -s)" = "Darwin" ] || fail "the managed edition is built on macOS only"
for tool in git cmake ninja python3 cc c++ cargo cargo-deny install_name_tool otool codesign; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    case "$tool" in
      cmake|ninja) fail "needs ${tool} on the search path: brew install cmake ninja" ;;
      cargo-deny) fail "needs cargo-deny, which checks the licences of FineType's crates: cargo install --locked cargo-deny" ;;
      *) fail "needs ${tool} on the search path" ;;
    esac
  fi
done

FINETYPE_TAG=$("${HERE}/finetype-pin.sh" --tag)

mkdir -p "$OUT"
SRC="${OUT}/src/duckdb"
# vcpkg's clone is about a gigabyte; BRIGHTFIELD_VCPKG_ROOT shares one between
# checkouts.
VCPKG="${BRIGHTFIELD_VCPKG_ROOT:-${OUT}/vcpkg}"
FINETYPE_SRC="${OUT}/src/finetype"

# Fetch REPO at COMMIT into DIR with a depth-one fetch, and refuse if the
# checkout's HEAD is anything else.
fetch_commit() {
  local url="$1" commit="$2" dir="$3"
  if [ ! -d "${dir}/.git" ]; then
    mkdir -p "$dir"
    git -C "$dir" init -q
    git -C "$dir" remote add origin "$url"
  fi
  if [ "$(git -C "$dir" rev-parse -q --verify HEAD 2>/dev/null || true)" != "$commit" ]; then
    say "fetching ${url} at ${commit}"
    git -C "$dir" fetch -q --depth 1 origin "$commit"
    git -C "$dir" checkout -q --force FETCH_HEAD
  fi
  local head
  head=$(git -C "$dir" rev-parse HEAD)
  [ "$head" = "$commit" ] || fail "${dir} is at ${head}, not the pinned ${commit}"
}

# --- DuckDB's source -------------------------------------------------------
fetch_commit "$DUCKDB_URL" "$DUCKDB_COMMIT" "$SRC"

# The shell links the library, not a static copy of DuckDB. DuckDB's own
# tools/shell/CMakeLists.txt links duckdb_static and every statically linked
# extension into the shell; these two edits point it at the shared `duckdb`
# target instead, which already carries the extensions. Each edit is checked, so
# a DuckDB bump that moves either line fails here instead of building a shell
# with its own copy.
SHELL_CMAKE="${SRC}/tools/shell/CMakeLists.txt"
git -C "$SRC" checkout -q -- tools/shell/CMakeLists.txt
python3 - "$SHELL_CMAKE" <<'PY'
import sys
path = sys.argv[1]
text = open(path, encoding="utf-8").read()
edits = [
    ("target_link_libraries(shell duckdb_shell duckdb_static",
     "target_link_libraries(shell duckdb_shell duckdb"),
    ('link_extension_libraries(shell "")\n', ""),
]
for old, new in edits:
    if text.count(old) != 1:
        sys.exit(f"build-duckdb: expected one '{old.strip()}' in {path}, found {text.count(old)}")
    text = text.replace(old, new)
open(path, "w", encoding="utf-8").write(text)
PY

# Brightfield's patches to extensions, where DuckDB's build reads extension
# patches from: .github/patches/extensions/<name>/ in its source. An extension
# whose configuration says APPLY_PATCHES gets them applied, as commits, when its
# source is cloned. The names start brightfield- so they sort after, and never
# replace, any patch DuckDB ships for the same extension.
# Copies from an earlier build are removed first, so the directory holds exactly
# the patches scripts/duckdb-managed/patches/ holds now; DuckDB's own are kept.
rm -f "${SRC}"/.github/patches/extensions/*/brightfield-*.patch
for patch_dir in "${CONFIG_DIR}"/patches/*/; do
  ext=$(basename "$patch_dir")
  mkdir -p "${SRC}/.github/patches/extensions/${ext}"
  cp "$patch_dir"/*.patch "${SRC}/.github/patches/extensions/${ext}/"
done

# --- vcpkg -----------------------------------------------------------------
# A full clone, not a depth-one fetch: vcpkg resolves each port's version from
# the repository's history (its versions/ database names git trees), and a
# shallow clone fails that with "failed to unpack tree object".
if [ ! -d "${VCPKG}/.git" ]; then
  say "cloning ${VCPKG_URL}"
  git clone -q "$VCPKG_URL" "$VCPKG"
fi
if [ "$(git -C "$VCPKG" rev-parse HEAD)" != "$VCPKG_COMMIT" ]; then
  git -C "$VCPKG" cat-file -e "${VCPKG_COMMIT}^{commit}" 2>/dev/null || git -C "$VCPKG" fetch -q origin
  git -C "$VCPKG" checkout -q --force "$VCPKG_COMMIT"
fi
[ "$(git -C "$VCPKG" rev-parse HEAD)" = "$VCPKG_COMMIT" ] || fail "${VCPKG} is not at the pinned ${VCPKG_COMMIT}"
if [ ! -x "${VCPKG}/vcpkg" ]; then
  say "bootstrapping vcpkg"
  "${VCPKG}/bootstrap-vcpkg.sh" -disableMetrics >&2
fi

# --- FineType, as a static library -----------------------------------------
fetch_commit "$FINETYPE_URL" "$FINETYPE_COMMIT" "$FINETYPE_SRC"
# The tag must still name the commit pinned above.
tag_commit=$(git ls-remote "$FINETYPE_URL" "refs/tags/${FINETYPE_TAG}^{}" "refs/tags/${FINETYPE_TAG}" \
  | awk 'NR==1 || /\^\{\}$/ {c=$1} END {print c}')
[ "$tag_commit" = "$FINETYPE_COMMIT" ] \
  || fail "FineType ${FINETYPE_TAG} names ${tag_commit:-nothing}, not the pinned ${FINETYPE_COMMIT}; move FINETYPE_COMMIT with the pin"

# FineType's two table macros move from its Rust, which can only CREATE them in
# the database being opened, to the C++ entry point, which registers them in
# DuckDB's system catalog (scripts/duckdb-managed/finetype/move-table-macros.py
# says why). The script reads their SQL out of FineType's source into a header
# the entry point is compiled with, and makes FineType's own registration do
# nothing. That one function is the only change to FineType's files; it is
# reapplied to a clean copy each build and refuses a source it does not match.
FINETYPE_GENERATED="${OUT}/src/finetype-generated"
mkdir -p "$FINETYPE_GENERATED"
git -C "$FINETYPE_SRC" checkout -q -- crates/finetype-duckdb/src/lib.rs
python3 "${CONFIG_DIR}/finetype/move-table-macros.py" \
  "${FINETYPE_SRC}/crates/finetype-duckdb/src/lib.rs" "${FINETYPE_GENERATED}/finetype_table_macros.hpp"

say "building FineType ${FINETYPE_TAG} as a static library"
# crates/finetype-duckdb declares crate-type cdylib, the loadable extension;
# `cargo rustc --crate-type staticlib` builds the same crate as an archive
# instead, with FineType's manifest unchanged. It builds with the
# toolchain FineType's own rust-toolchain.toml pins, the one FineType is tested
# on: run from `cargo xtask`, this script inherits rustup's RUSTUP_TOOLCHAIN for
# Brightfield's toolchain, which would take precedence over FineType's file, so
# it and the variables that name a cargo or rustc are cleared here. The gate
# also sets CARGO_TARGET_DIR to its shared target directory, which would put
# the archive somewhere other than FineType's own target/, where it is read
# below, so that is cleared as well.
(
  cd "$FINETYPE_SRC"
  env -u RUSTUP_TOOLCHAIN -u CARGO -u RUSTC -u RUSTDOC -u RUSTC_WRAPPER -u CARGO_TARGET_DIR \
    MACOSX_DEPLOYMENT_TARGET=11.0 cargo rustc -p finetype_duckdb --release --locked \
    -j "$JOBS" --crate-type staticlib >&2
)
FINETYPE_LIB="${FINETYPE_SRC}/target/release/libfinetype.a"
[ -f "$FINETYPE_LIB" ] || fail "FineType's build left no ${FINETYPE_LIB}"

# --- DuckDB, with the extensions ------------------------------------------
say "building DuckDB ${DUCKDB_VERSION_PIN} with its extensions (${JOBS} jobs)"
(
  cd "$SRC"
  # DUCKDB_VERSION stands in for `git describe`, which a depth-one fetch has no
  # tags for; the commit is read from the checkout. Together they make
  # `duckdb --version` print what the release prints.
  export DUCKDB_VERSION="$DUCKDB_VERSION_PIN"
  export GEN=ninja
  export EXTENSION_CONFIGS="${CONFIG_DIR}/extensions.cmake"
  export VCPKG_TOOLCHAIN_PATH="${VCPKG}/scripts/buildsystems/vcpkg.cmake"
  # Release-only, for macOS 11: scripts/duckdb-managed/triplets/.
  export VCPKG_TARGET_TRIPLET=arm64-osx-brightfield
  export CMAKE_BUILD_PARALLEL_LEVEL="$JOBS"
  export VCPKG_MAX_CONCURRENCY="$JOBS"
  export MACOSX_DEPLOYMENT_TARGET=11.0
  # Some of vcpkg's ports at this commit still declare a CMake minimum below 3.5,
  # which CMake 4 refuses without this.
  export CMAKE_POLICY_VERSION_MINIMUM=3.5
  # ENABLE_EXTENSION_LOAD=OFF leaves DuckDB's extension installer and file
  # loader out of the library. Every INSTALL, FORCE INSTALL and UPDATE
  # EXTENSIONS, and every LOAD of a file, then fails before it reaches the
  # network or the disk, whatever the name, the FROM repository or the
  # settings; a LOAD of an extension compiled in still succeeds. No DuckDB
  # setting refuses `INSTALL json FROM core` and keeps file reads open, and a
  # Mac App Store app may not download code, so the refusal is compiled in, for
  # the shell arc runs a Protocol's steps with as well as for the engine.
  # httpfs carries its own HTTP clients, so DuckDB's built-in one, which this
  # also leaves out, is not needed to read a URL.
  export EXTRA_CMAKE_VARIABLES="-DENABLE_EXTENSION_LOAD=OFF -DBRIGHTFIELD_FINETYPE_STATICLIB=${FINETYPE_LIB} -DBRIGHTFIELD_FINETYPE_VERSION=${FINETYPE_TAG} -DBRIGHTFIELD_FINETYPE_GENERATED_DIR=${FINETYPE_GENERATED} -DBUILD_UNITTESTS=0 -DCMAKE_OSX_DEPLOYMENT_TARGET=11.0 -DVCPKG_HOST_TRIPLET=arm64-osx-brightfield -DVCPKG_OVERLAY_TRIPLETS=${CONFIG_DIR}/triplets"
  make release >&2
)

BUILD="${SRC}/build/release"
[ -f "${BUILD}/src/libduckdb.dylib" ] || fail "DuckDB's build left no src/libduckdb.dylib"
[ -x "${BUILD}/duckdb" ] || fail "DuckDB's build left no shell at ${BUILD}/duckdb"

# --- Stage the output ------------------------------------------------------
rm -rf "${OUT}/lib" "${OUT}/bin" "${OUT}/include"
mkdir -p "${OUT}/lib" "${OUT}/bin" "${OUT}/include"
cp "${BUILD}/src/libduckdb.dylib" "${OUT}/lib/libduckdb.dylib"
cp "${BUILD}/duckdb" "${OUT}/bin/duckdb"
cp "${SRC}/src/include/duckdb.h" "${SRC}/src/include/duckdb_extension.h" \
  "${SRC}/src/include/duckdb_static_extension.h" "${OUT}/include/"

install_name_tool -id "@rpath/libduckdb.dylib" "${OUT}/lib/libduckdb.dylib"

# The vcpkg libraries the library links dynamically: GEOS's two, which the
# triplet builds shared because GEOS is LGPL-2.1 (scripts/duckdb-managed/
# triplets/). Each is copied beside the library, named by @rpath, and found
# through the library's @loader_path rpath, which is lib/ here and
# Contents/Frameworks/ in the app. Any other library outside the system's
# fails the build, so nothing lands in the app without being chosen.
VCPKG_LIB="${BUILD}/vcpkg_installed/arm64-osx-brightfield/lib"
# A staged file keeps only the rpaths given here: the link leaves ones into this
# build's tree, which would load a library from the build instead of from
# beside it.
clear_rpaths() {
  while read -r rpath; do
    install_name_tool -delete_rpath "$rpath" "$1"
  done < <(otool -l "$1" | awk '/cmd LC_RPATH/ {getline; getline; print $2}')
}
clear_rpaths "${OUT}/lib/libduckdb.dylib"
install_name_tool -add_rpath "@loader_path" "${OUT}/lib/libduckdb.dylib"
non_system() { otool -L "$1" | tail -n +2 | awk '{print $1}' | grep -vE '^(/usr/lib/|/System/)' || true; }
staged=()
pending=("${OUT}/lib/libduckdb.dylib")
while [ "${#pending[@]}" -gt 0 ]; do
  next=()
  for file in "${pending[@]}"; do
    while IFS= read -r dep; do
      [ -n "$dep" ] || continue
      name=$(basename "$dep")
      case "$dep" in
        "@rpath/libduckdb.dylib") continue ;;
        "@rpath/${name}"|"${VCPKG_LIB}/${name}"|"@loader_path/${name}") ;;
        *) fail "$(basename "$file") links ${dep}, which is neither DuckDB nor a vcpkg library this build chose to ship" ;;
      esac
      case "$name" in
        libgeos*.dylib) ;;
        *) fail "$(basename "$file") links ${name} dynamically; only GEOS is meant to ship as its own library" ;;
      esac
      [ "$dep" = "@rpath/${name}" ] || install_name_tool -change "$dep" "@rpath/${name}" "$file"
      if [ ! -f "${OUT}/lib/${name}" ]; then
        real=$(cd "$VCPKG_LIB" && pwd -P)/${name}
        [ -f "$real" ] || fail "no ${name} in ${VCPKG_LIB}"
        cp -L "$real" "${OUT}/lib/${name}"
        chmod u+w "${OUT}/lib/${name}"
        install_name_tool -id "@rpath/${name}" "${OUT}/lib/${name}"
        clear_rpaths "${OUT}/lib/${name}"
        install_name_tool -add_rpath "@loader_path" "${OUT}/lib/${name}"
        staged+=("$name")
        next+=("${OUT}/lib/${name}")
      fi
    done < <(non_system "$file")
  done
  pending=("${next[@]+"${next[@]}"}")
done
[ "${#staged[@]}" -gt 0 ] || fail "the library links no GEOS library of its own; the triplet should build GEOS shared"
# The shell's build-tree rpaths point into this checkout; replace them with the
# two layouts it runs from.
clear_rpaths "${OUT}/bin/duckdb"
current=$(otool -L "${OUT}/bin/duckdb" | awk '/libduckdb/ {print $1}')
[ -n "$current" ] || fail "the shell does not link libduckdb.dylib: $(otool -L "${OUT}/bin/duckdb")"
[ "$current" = "@rpath/libduckdb.dylib" ] \
  || install_name_tool -change "$current" "@rpath/libduckdb.dylib" "${OUT}/bin/duckdb"
install_name_tool -add_rpath "@executable_path/../lib" "${OUT}/bin/duckdb"
install_name_tool -add_rpath "@executable_path/../Frameworks" "${OUT}/bin/duckdb"
# install_name_tool invalidates the ad-hoc signature the linker gave each file,
# and Apple Silicon refuses to run unsigned code. Sign them again, ad hoc; the
# Store packaging signs them with the distribution identity.
for lib in "${OUT}"/lib/*.dylib; do
  codesign --force --sign - "$lib" >&2
done
codesign --force --sign - "${OUT}/bin/duckdb" >&2

# The licence notices of what this build put in lib/, and of FineType's
# command line, which the app carries beside it: the C libraries vcpkg built,
# GEOS's licence with where its source is, DuckDB's and its extensions', and
# FineType's crates, which cargo-deny first holds to Brightfield's licence
# policy. A vcpkg port the script does not know stops the build.
rm -rf "${OUT}/licenses"
python3 "${CONFIG_DIR}/notices/notices.py" managed "${OUT}/licenses" \
  --duckdb-src "$SRC" --duckdb-build "$BUILD" --lib-dir "${OUT}/lib" \
  --extensions-cmake "${CONFIG_DIR}/extensions.cmake" --vcpkg-commit "$VCPKG_COMMIT" \
  --finetype-src "$FINETYPE_SRC" --deny-config "${ROOT}/deny.toml" >&2

{
  for lib in "${OUT}"/lib/*.dylib; do
    echo "$(basename "$lib") $(stat -f %z "$lib")"
  done
  echo "duckdb $(stat -f %z "${OUT}/bin/duckdb")"
} > "${OUT}/sizes.txt"

say "built ${OUT}/lib/libduckdb.dylib and ${OUT}/bin/duckdb"
cat "${OUT}/sizes.txt" >&2
printf '%s\n' "$OUT"
