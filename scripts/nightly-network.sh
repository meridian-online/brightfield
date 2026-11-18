#!/usr/bin/env bash
# The tests the push suites deliberately do not run, because they need the
# network or a FineType bundle fetched over it. `cargo xtask ci --step
# nightly-network` runs this; the nightly run and nightly-network.yml both do.
#
# WHAT THIS COVERS: every `#[ignore]`d test in the workspace whose reason is
# the network, enumerated from the source (`grep -rn '#\[ignore' crates/`):
#
#   crates/brightfield-shell/tests/front_door.rs
#     the_published_crosswalk_is_still_the_table_its_fixture_was_cut_from
#       - the published crosswalk Parquet still has the size, sha256 and row
#         count recorded when its committed reduced copy was cut. Red means
#         re-cut it with scripts/cut-crosswalk-fixture.sh and re-shoot the
#         thumbnails.
#     statlib_still_serves_the_housing_archive_the_fixture_is
#       - StatLib still serves California Housing's archive with the size and
#         sha256 recorded beside the committed copy. Red means replace the
#         copy, update the provenance file and re-shoot the thumbnails.
#
#   crates/brightfield-shell/tests/crosswalk_chart.rs
#     the_published_crosswalk_is_still_past_the_drawn_primitive_ceiling
#     the_crosswalk_chart_start_opens_over_the_network_drawing_every_row
#       - the published table is still big enough that the chart's aggregating
#         marks are the right choice, and the start composes over the network
#         with both marks accounting for the same number of rows.
#
#   crates/brightfield-engine/src/lib.rs  (unit tests)
#     remote_https_parquet_executes_via_httpfs
#     ducklake_catalog_attaches_read_only_over_https
#       - the layer underneath: DuckDB httpfs reads a published parquet, and
#         the published DuckLake catalog attaches read-only. Engine red and
#         shell red together points at the host or the data; shell red alone
#         points at the start.
#
#   crates/brightfield-shell/tests/column_header_band.rs
#     the_full_band_names_what_a_varchar_column_means
#     one_process_opens_one_file_with_a_type_source_and_without_one
#       - the grid's header band stating a column's meaning rather than its
#         storage type, which needs a real FineType bundle. They run last, by
#         name, under their own floor, over the bundle this script fetches.
#
# DELIBERATELY NOT COVERED: `vello_bump_ceiling.rs`'s counter ladder. It is an
# instrument rather than a gate, and ignored for GPU reasons, not network ones.
#
# THE GUARD: `cargo test -- --ignored` with a filter that matches nothing exits
# 0 and prints "0 passed", and a run that goes green having executed no tests
# reads as coverage. So each target's executed count is parsed back out of the
# harness summary and held to a floor, per target rather than only in total: a
# single total would stay green if one target silently ran nothing while
# another still ran. The floors are floors, not equalities, so a new network
# test does not redden the run for being new; a covered test being renamed,
# deleted or filtered away to nothing does.
#
# THE FLOORS ARE LITERALS and nothing pins them to the source: deleting a test
# and its floor together would be green.
#
# Environment this reads:
#   BRIGHTFIELD_FINETYPE_BUNDLE  where the pinned FineType bundle is, or is
#                                fetched to when the directory is absent;
#                                default ${TMPDIR}/finetype-bundle. Only that
#                                default is deleted and refetched when it is
#                                refused; a bundle named here never is.
#   BRIGHTFIELD_BUNDLE_TARGET    the bundle's target triple; default
#                                aarch64-apple-darwin
set -u

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root" || exit 1

tmp="${TMPDIR:-/tmp}"
tmp="${tmp%/}"

# A failure line: a GitHub annotation in CI, a plain `error:` line elsewhere.
error() {
  local title="$1" message="$2"
  if [ -n "${GITHUB_ACTIONS:-}" ]; then
    echo "::error title=${title}::${message}"
  else
    echo "error: ${title}: ${message}"
  fi
}

# libtest's summary line, copied from a real run rather than remembered:
#   test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 9 filtered out; finished in 9.92s
# The number before the literal "passed;" is what executed, as opposed to what
# was compiled or filtered out. The match is anchored to the start of a line,
# so the phrase mid-line in a test's own output does not add to the count.
executed() {
  awk '/^test result:/ { for (i = 1; i <= NF; i++) if ($i == "passed;") n += $(i - 1) } END { print n + 0 }' "$1"
}

# label | cargo target selector | how many ignored tests must run
targets=(
  "front_door|-p brightfield-shell --test front_door|2"
  "crosswalk_chart|-p brightfield-shell --test crosswalk_chart|2"
  "engine_httpfs|-p brightfield-engine --lib|2"
)
# The sum of the floors above, stated once so a reader and the log both see
# what this run expects to have executed.
expected_total=6

failed=0
total=0

for entry in "${targets[@]}"; do
  IFS='|' read -r label selector expected <<<"$entry"
  log="${tmp}/ignored-${label}.log"

  # The exit code is captured, never masked: no pipe into head/tail, output
  # redirected to a file, the code kept in a variable and the file printed
  # afterwards.
  code=0
  # shellcheck disable=SC2086
  cargo test --locked $selector -- --ignored --test-threads=1 >"$log" 2>&1 || code=$?
  cat "$log"

  ran=$(executed "$log")
  echo "--- ${label}: cargo exited ${code}, ${ran} test(s) executed (floor ${expected})"

  if [ "$code" -ne 0 ]; then
    error "${label} failed" "cargo test exited ${code} for ${label} — read the log above"
    failed=1
  fi

  if [ "$ran" -lt "$expected" ]; then
    error "${label} executed too few tests" "expected at least ${expected} ignored test(s) to execute in ${label}, ${ran} executed — the filter matched nothing, or a covered test was renamed, deleted or un-ignored. A green run here would prove nothing."
    failed=1
  fi

  total=$((total + ran))
done

if [ "$total" -lt "$expected_total" ]; then
  error "too few network-gated tests executed" "expected at least ${expected_total} network-gated test(s) across all targets, ${total} executed"
  failed=1
fi

if [ "$failed" -ne 0 ]; then
  exit 1
fi

echo "${total} network-gated test(s) executed and passed (floor ${expected_total})"

# ------------------------------------------------------------------
# THE BUNDLE-PRESENT CONFIGURATION, which nothing else runs.
#
# brightfield's differentiating claim is that the grid states what a column
# means rather than how DuckDB stored it, and all of it depends on a FineType
# bundle being present. `LoadOptions::packaged` looks beside the running
# executable; a test binary has nothing there, so the labelled branch is
# reachable by a shipped binary and by no test a pull request's checks run.
#
# The fetch is two hosts that can be down while the commit is fine, so it
# lives here with the other network-dependent checks.
# ------------------------------------------------------------------
# A bundle the caller named is theirs: a contributor may point the variable at
# one they assembled, so it is never deleted. Only the script's own default
# directory is disposable.
default_bundle="${tmp}/finetype-bundle"
bundle="${BRIGHTFIELD_FINETYPE_BUNDLE:-$default_bundle}"
own_bundle=0
[ "$bundle" = "$default_bundle" ] && own_bundle=1
target="${BRIGHTFIELD_BUNDLE_TARGET:-aarch64-apple-darwin}"

pin_tag="$(./scripts/finetype-pin.sh --tag)"
platform="$(./scripts/duckdb-platform.sh "$target")"

# Read the bundle before trusting it, restored or freshly fetched alike. A
# cache entry is not an attestation: it is whatever a previous run left. This
# is the pre-flight scripts/package.sh runs over the bundle it stages, with the
# pinned tag, so a bundle from a different release is refused by name rather
# than by a puzzling assertion in a test.
bundle_is_pinned() {
  ./scripts/check-bundled-extension.sh "$bundle" "$platform" "$pin_tag"
}

fetch_bundle() {
  ./scripts/fetch-finetype-bundle.sh "$target" "$bundle" || {
    error "the FineType bundle did not fetch" "scripts/fetch-finetype-bundle.sh failed for ${target}"
    exit 1
  }
}

if [ ! -d "$bundle" ]; then
  fetch_bundle
elif ! bundle_is_pinned >/dev/null 2>&1; then
  if [ "$own_bundle" -eq 1 ]; then
    # What a pin bump leaves behind in the default directory: fetch it again.
    rm -rf "$bundle"
    fetch_bundle
  else
    error "the bundle is not the pinned one" "BRIGHTFIELD_FINETYPE_BUNDLE=${bundle} is refused by check-bundled-extension; fix or unset it"
    exit 1
  fi
fi

bundle_is_pinned || {
  error "the bundle is not the pinned one" "scripts/check-bundled-extension.sh refused ${bundle}"
  exit 1
}

# The same guard shape as above. The floor is 2 because column_header_band.rs
# carries two `#[ignore]`d tests today.
floor=2
log="${tmp}/column-header-band-typed.log"

code=0
BRIGHTFIELD_FINETYPE_BUNDLE="$bundle" cargo test --locked -p brightfield-shell --test column_header_band \
  -- --ignored --test-threads=1 --nocapture >"$log" 2>&1 || code=$?
cat "$log"

ran=$(executed "$log")
echo "--- column_header_band --ignored: cargo exited ${code}, ${ran} test(s) executed (floor ${floor})"

failed=0
if [ "$code" -ne 0 ]; then
  error "the grid does not name what a column means" "cargo test exited ${code} over a real FineType bundle. The failure above names the column and quotes what the band drew; a header reading the storage type in both halves of its leaf-and-storage row is the regression this covers."
  failed=1
fi

if [ "$ran" -lt "$floor" ]; then
  error "executed too few tests" "expected at least ${floor} ignored test(s) to execute in column_header_band, ${ran} executed — the filter matched nothing, or a covered test was renamed, deleted or un-ignored. A green run here would prove nothing."
  failed=1
fi

if [ "$failed" -ne 0 ]; then
  exit 1
fi

echo "${ran} bundle-present band test(s) executed and passed (floor ${floor})"
