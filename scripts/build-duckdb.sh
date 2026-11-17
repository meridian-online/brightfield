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
#   include/              duckdb.h and the other C API headers
#   bin/duckdb            DuckDB's shell, which loads lib/libduckdb.dylib through
#                         its rpaths: @executable_path/../lib here, and
#                         @executable_path/../Frameworks for an app bundle's
#                         Contents/Helpers/
#   sizes.txt             the sizes of the library and the shell
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
FINETYPE_COMMIT="3693c9f79c59530ac04f266b4dbd36762b79c328"

JOBS="${BRIGHTFIELD_DUCKDB_JOBS:-6}"
case "$JOBS" in ''|*[!0-9]*) fail "BRIGHTFIELD_DUCKDB_JOBS='${JOBS}' is not a number" ;; esac

[ "$(uname -s)" = "Darwin" ] || fail "the managed edition is built on macOS only"
for tool in git cmake ninja python3 cc c++ cargo install_name_tool otool codesign; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    case "$tool" in
      cmake|ninja) fail "needs ${tool} on the search path: brew install cmake ninja" ;;
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

say "building FineType ${FINETYPE_TAG} as a static library"
# crates/finetype-duckdb declares crate-type cdylib, the loadable extension;
# `cargo rustc --crate-type staticlib` builds the same crate as an archive
# instead, with nothing in FineType's own files changed. It builds with the
# toolchain FineType's own rust-toolchain.toml pins, the one FineType is tested
# on: run from `cargo xtask`, this script inherits rustup's RUSTUP_TOOLCHAIN for
# Brightfield's toolchain, which would take precedence over FineType's file, so
# it and the variables that name a cargo or rustc are cleared here.
(
  cd "$FINETYPE_SRC"
  env -u RUSTUP_TOOLCHAIN -u CARGO -u RUSTC -u RUSTDOC -u RUSTC_WRAPPER \
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
  export EXTRA_CMAKE_VARIABLES="-DBRIGHTFIELD_FINETYPE_STATICLIB=${FINETYPE_LIB} -DBRIGHTFIELD_FINETYPE_VERSION=${FINETYPE_TAG} -DBUILD_UNITTESTS=0 -DCMAKE_OSX_DEPLOYMENT_TARGET=11.0 -DVCPKG_HOST_TRIPLET=arm64-osx-brightfield -DVCPKG_OVERLAY_TRIPLETS=${CONFIG_DIR}/triplets"
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
# The shell's build-tree rpaths point into this checkout; replace them with the
# two layouts it runs from.
while read -r rpath; do
  install_name_tool -delete_rpath "$rpath" "${OUT}/bin/duckdb"
done < <(otool -l "${OUT}/bin/duckdb" | awk '/cmd LC_RPATH/ {getline; getline; print $2}')
current=$(otool -L "${OUT}/bin/duckdb" | awk '/libduckdb/ {print $1}')
[ -n "$current" ] || fail "the shell does not link libduckdb.dylib: $(otool -L "${OUT}/bin/duckdb")"
[ "$current" = "@rpath/libduckdb.dylib" ] \
  || install_name_tool -change "$current" "@rpath/libduckdb.dylib" "${OUT}/bin/duckdb"
install_name_tool -add_rpath "@executable_path/../lib" "${OUT}/bin/duckdb"
install_name_tool -add_rpath "@executable_path/../Frameworks" "${OUT}/bin/duckdb"
# install_name_tool invalidates the ad-hoc signature the linker gave each file,
# and Apple Silicon refuses to run unsigned code. Sign them again, ad hoc; the
# Store packaging signs them with the distribution identity.
codesign --force --sign - "${OUT}/lib/libduckdb.dylib" >&2
codesign --force --sign - "${OUT}/bin/duckdb" >&2

{
  echo "libduckdb.dylib $(stat -f %z "${OUT}/lib/libduckdb.dylib")"
  echo "duckdb $(stat -f %z "${OUT}/bin/duckdb")"
} > "${OUT}/sizes.txt"

say "built ${OUT}/lib/libduckdb.dylib and ${OUT}/bin/duckdb"
cat "${OUT}/sizes.txt" >&2
printf '%s\n' "$OUT"
