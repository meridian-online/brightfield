#!/usr/bin/env bash
# Check that Meridian's build of DuckDB runs Protocols exactly as DuckDB's
# release command line does.
#
#   scripts/check-duckdb-parity.sh                  the whole check
#   scripts/check-duckdb-parity.sh --protocols-only the Protocol runs alone
#   scripts/check-duckdb-parity.sh --arc-tests-only arc's tests alone
#   scripts/check-duckdb-parity.sh --compare A B    compare two captures
#   scripts/check-duckdb-parity.sh --normalise RUNS OUT
#                                                   write a run's records,
#                                                   normalised, into OUT
#
# WHY. The Mac App Store edition cannot ship DuckDB's extension files, so it
# runs on Meridian's own build of DuckDB v2.0.0 with its extensions compiled in
# (scripts/build-duckdb.sh), where Brightfield Studio runs DuckDB's release. A
# Protocol is meant to give the same result whichever edition runs it, so the
# two builds are held to the same outputs, and any difference fails the check.
# It is not allowed for: a difference is fixed in the build, or raised.
#
# WHAT RUNS. Two parts.
#
# 1. Every Protocol Brightfield ships that can run offline, and the
#    agent-written Protocols the test suite checks, run three times through
#    `arc run`: twice with ARC_DUCKDB_BIN naming the release command line and
#    once naming Meridian's. Each run starts from the same copy of the
#    Protocol at the same path, with its own empty database and history, so
#    the paths in what it prints and records are the same on every run. Each
#    run is captured (see CAPTURE below), and the captures are compared: the
#    two release runs first, which shows the Protocol gives the same result
#    twice on one engine, so a difference from Meridian's build is the build's;
#    then the first release run with Meridian's.
#
#    A Protocol that fetches its data runs over the copy the test suite
#    commits (crates/brightfield-shell/tests/support/shipped_protocol.rs has
#    the same list): the copy is put where the fetch would write it and the
#    fetch is dropped (scripts/drop-fetch-steps.sh). Where the copy is of the
#    table a load step makes, as for Online Retail's zipped workbook, the steps
#    before that step are dropped and the step reads the copy. A Protocol
#    Brightfield ships that is not in the list below fails the check, so a new
#    one cannot go unchecked; a Protocol the managed edition opens restricted
#    is listed as such and not run.
#
# 2. arc's engine and tokenizer tests, from arcform at the revision Cargo.lock
#    pins, with Meridian's command line first on PATH and named by
#    ARC_DUCKDB_RECORD_BIN: the tests arc's own CI runs on the release command
#    line (the DuckDB lineage reader, the tokens its refusals read, the reader
#    switch and the reader parity test), and its engine contract tests. arcform
#    is cloned from cargo's own checkout of that revision, so no network is
#    needed once cargo has fetched it, and built against Meridian's library.
#    The same arc runs the Protocols in part 1, unless ARC_BIN names another.
#
# CAPTURE. For one run, a folder holding:
#   exit         arc's exit status
#   stdout       what arc printed on stdout, the asset graph and any rows a
#                SELECT printed (stderr is kept beside it, and not compared)
#   records/     each run record and its event log, with the run's id,
#                timestamps and durations taken out (--normalise)
#   files        the SHA-256 of every file the run left in the Protocol's
#                folder, other than the run records and a database
#   tables/      for each table in the run's database, its columns and types
#                (<schema>.<table>.schema) and its rows, sorted, as CSV
#                (<schema>.<table>.csv), with the ids and times of arc's
#                own state tables left out; each view's SQL in views.csv,
#                and each macro the database holds in macros.csv, since an
#                engine that leaves a macro of its own in a Protocol's
#                database has changed it
# The databases are read by the release command line, the same reader for
# both sides, so a difference is in what the engine under test wrote.
#
# Environment:
#   BRIGHTFIELD_RELEASE_DUCKDB  the release command line (default
#                               target/duckdb-cli/duckdb, which
#                               scripts/fetch-duckdb-cli.sh writes)
#   BRIGHTFIELD_MANAGED_DUCKDB  Meridian's command line (default bin/duckdb in
#                               the directory scripts/build-duckdb.sh
#                               --print-dir names)
#   ARC_BIN                     the arc to run the Protocols with (default: the
#                               one built from the pinned arcform)
#   BRIGHTFIELD_PARITY_DIR      where the copies, captures and the arcform
#                               build go (default target/duckdb-parity); the
#                               captures are kept there to be read after a
#                               failure
#
# NEEDS: python3, git and cargo, and both command lines. A missing one stops
# the check and says where it comes from. Most of the time is arc's build and
# tests: about 20 to 30 minutes from cold, and the Protocol runs a minute.
# Its regression test is scripts/check-duckdb-parity-selftest.sh.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "${HERE}/.." && pwd)

fail() { echo "check-duckdb-parity: $*" >&2; exit 1; }
say() { echo "check-duckdb-parity: $*" >&2; }

need_python() {
  command -v python3 > /dev/null 2>&1 || fail "needs python3 on the search path"
}

# --normalise RUNS OUT: the run records (*.json) and event logs (*.jsonl) in
# RUNS, written into OUT as 1.json, 1.jsonl, ... in the order of their run ids,
# which start with the run's time. What changes from one run to the next is
# taken out: every key named in VOLATILE, at any depth, and the run's id
# wherever a string carries it, as in a step's output path.
normalise() {
  need_python
  python3 -I - "$1" "$2" <<'PY'
import json, os, sys

runs, out = sys.argv[1], sys.argv[2]
VOLATILE = {"run_id", "attempt_id", "started_at", "finished_at", "duration_sec", "duration_ms", "ts"}

def clean(value, ids):
    if isinstance(value, dict):
        return {k: clean(v, ids) for k, v in value.items() if k not in VOLATILE}
    if isinstance(value, list):
        return [clean(v, ids) for v in value]
    if isinstance(value, str):
        for i in ids:
            value = value.replace(i, "<run>")
    return value

if not os.path.isdir(runs):
    sys.exit(f"no run state at {runs}")
records = sorted(f[:-5] for f in os.listdir(runs) if f.endswith(".json"))
if not records:
    sys.exit(f"the run left no run record in {runs}")
os.makedirs(out, exist_ok=True)
for n, run_id in enumerate(records, 1):
    with open(os.path.join(runs, run_id + ".json")) as f:
        record = json.load(f)
    ids = {run_id}
    run = record.get("run", {})
    ids.update(v for v in (run.get("run_id"), run.get("attempt_id")) if isinstance(v, str) and v)
    with open(os.path.join(out, f"{n}.json"), "w") as f:
        json.dump(clean(record, ids), f, indent=1, sort_keys=True)
        f.write("\n")
    events = os.path.join(runs, run_id + ".jsonl")
    if os.path.exists(events):
        with open(events) as f, open(os.path.join(out, f"{n}.jsonl"), "w") as w:
            for line in f:
                if line.strip():
                    w.write(json.dumps(clean(json.loads(line), ids), sort_keys=True) + "\n")
PY
}

# --compare A B: exit 0 when the two captures hold the same files with the same
# bytes. Each file that differs, or is on one side only, is named, with the
# start of the difference.
compare() {
  local a="$1" b="$2"
  [ -d "$a" ] || fail "no capture at $a"
  [ -d "$b" ] || fail "no capture at $b"
  local listed
  listed=$( (cd "$a" && find . -type f; cd "$b" && find . -type f) | LC_ALL=C sort -u)
  [ -n "$listed" ] || fail "both captures are empty, so there is nothing to compare: $a and $b"
  local differs=0 rel
  while IFS= read -r rel; do
    rel=${rel#./}
    if [ ! -f "$a/$rel" ]; then
      echo "  only in $b: $rel"; differs=1
    elif [ ! -f "$b/$rel" ]; then
      echo "  only in $a: $rel"; differs=1
    elif ! cmp -s "$a/$rel" "$b/$rel"; then
      echo "  differs: $rel"
      diff -u "$a/$rel" "$b/$rel" | sed -n '3,40p' | sed 's/^/      /' || true
      differs=1
    fi
  done <<< "$listed"
  [ "$differs" -eq 0 ]
}

case "${1:-}" in
  --normalise)
    [ $# -eq 3 ] || fail "usage: --normalise <runs-dir> <out-dir>"
    normalise "$2" "$3"; exit $? ;;
  --compare)
    [ $# -eq 3 ] || fail "usage: --compare <capture> <capture>"
    compare "$2" "$3"; exit $? ;;
esac

PROTOCOLS=1 ARC_TESTS=1
case "${1:-}" in
  "") ;;
  --protocols-only) ARC_TESTS=0 ;;
  --arc-tests-only) PROTOCOLS=0 ;;
  *) fail "usage: scripts/check-duckdb-parity.sh [--protocols-only | --arc-tests-only | --compare A B | --normalise RUNS OUT]" ;;
esac
[ $# -le 1 ] || fail "one option at a time"

# The Protocols, as name|folder|fetched copies|load copy. A fetched copy is
# out=file: the file is put at the fetch's out, and the fetch is dropped. A load
# copy is step:model=file: the steps before step are dropped, the file goes
# under data/, and step's model reads it. Paths are from the repository root.
DATA=crates/brightfield-shell/tests/data
CASES=(
  "palmer-penguins|examples/starts/palmer-penguins||"
  "usgs-earthquakes-2025|examples/starts/usgs-earthquakes-2025||"
  "nyc-yellow-taxi|examples/starts/nyc-yellow-taxi|data/yellow_tripdata_2026-01.parquet=${DATA}/nyc-yellow-taxi-reduced.parquet|"
  "epoch-ai-models|examples/starts/epoch-ai-models|data/notable_ai_models.csv=${DATA}/epoch-ai-models-synthetic.csv|"
  "epoch-ai-models-over-its-reduced-table|examples/starts/epoch-ai-models||raw_models:models/01_raw_models.sql=${DATA}/epoch-ai-models-reduced.parquet"
  "online-retail-ii|examples/starts/online-retail-ii||retail_lines:models/01_retail_lines.sql=${DATA}/online-retail-ii-reduced.parquet"
  "co2-per-capita|tests/fixtures/agent-protocols/co2-per-capita||"
  "shipped-protocol|${DATA}/shipped-protocol||"
)
# The Protocols the managed edition opens restricted, as folder|reason. They
# never run on Meridian's build, so there is nothing to compare.
RESTRICTED=(
  "examples/protocol/edgar_gleif|runs uv, which the managed edition can't run; crates/brightfield-shell/tests/beyond_edition.rs checks it opens restricted"
)

RELEASE="${BRIGHTFIELD_RELEASE_DUCKDB:-${ROOT}/target/duckdb-cli/duckdb}"
MANAGED_DIR=$("${HERE}/build-duckdb.sh" --print-dir)
MANAGED="${BRIGHTFIELD_MANAGED_DUCKDB:-${MANAGED_DIR}/bin/duckdb}"
PAR="${BRIGHTFIELD_PARITY_DIR:-${ROOT}/target/duckdb-parity}"
case "$PAR" in /*) ;; *) PAR="${ROOT}/${PAR}" ;; esac

# DuckDB's command line prints differently when it takes itself to be run by
# an agent, which it reads from variables such as AI_AGENT. Off for every run
# here, whoever starts the check.
export DUCKDB_AGENT_MODE=0 NO_COLOR=1

need_python
for tool in git cargo cmp diff; do
  command -v "$tool" > /dev/null 2>&1 || fail "needs ${tool} on the search path"
done
[ -x "$RELEASE" ] || fail "no release DuckDB command line at ${RELEASE}: run scripts/fetch-duckdb-cli.sh aarch64-apple-darwin target/duckdb-cli, or set BRIGHTFIELD_RELEASE_DUCKDB"
[ -x "$MANAGED" ] || fail "no managed DuckDB command line at ${MANAGED}: run cargo xtask ci --step duckdb-managed, or set BRIGHTFIELD_MANAGED_DUCKDB"
# Absolute, since each run starts in the Protocol's folder.
release_real=$(cd "$(dirname "$RELEASE")" && pwd -P)/$(basename "$RELEASE")
managed_real=$(cd "$(dirname "$MANAGED")" && pwd -P)/$(basename "$MANAGED")
RELEASE="$release_real" MANAGED="$managed_real"
[ "$release_real" != "$managed_real" ] || fail "both command lines are ${release_real}; the check would compare a build with itself"
release_version=$("$RELEASE" -init /dev/null --version)
managed_version=$("$MANAGED" -init /dev/null --version)
[ "$release_version" = "$managed_version" ] || fail "the command lines report different versions: release '${release_version}', managed '${managed_version}'"
say "both command lines report ${release_version}"
mkdir -p "$PAR"

# ---- arc, built from the pinned arcform ------------------------------------

ARC_REV=$(awk '
  /^name = "arcform"$/ { want = 1; next }
  want && /^source = / { sub(/.*#/, ""); sub(/"$/, ""); print; exit }
  /^\[\[package\]\]/ { want = 0 }
' "${ROOT}/Cargo.lock")
[ "${#ARC_REV}" -eq 40 ] || fail "cannot read the arcform revision Cargo.lock pins (read '${ARC_REV}')"
ARC_SRC="${PAR}/arcform-${ARC_REV:0:12}"
ARC_TARGET="${PAR}/arcform-target"
TOOLCHAIN=$(sed -n 's/^channel = "\(.*\)"$/\1/p' "${ROOT}/rust-toolchain.toml")
[ -n "$TOOLCHAIN" ] || fail "cannot read the toolchain rust-toolchain.toml pins"

# The arcform checkout: cloned from cargo's checkout of the pinned revision,
# which `cargo metadata` fetches when it is not already there.
arcform_source() {
  if [ -d "${ARC_SRC}/.git" ] && [ "$(git -C "$ARC_SRC" rev-parse HEAD)" = "$ARC_REV" ]; then
    return 0
  fi
  local manifest checkout
  manifest=$(cd "$ROOT" && cargo metadata --format-version 1 --locked | python3 -I -c '
import json, sys
print(next(p["manifest_path"] for p in json.load(sys.stdin)["packages"] if p["name"] == "arcform"))')
  checkout=$(dirname "$manifest")
  [ "$(git -C "$checkout" rev-parse HEAD)" = "$ARC_REV" ] \
    || fail "cargo's arcform checkout at ${checkout} is not at ${ARC_REV}, the revision Cargo.lock pins"
  rm -rf "$ARC_SRC"
  git clone --quiet "$checkout" "$ARC_SRC"
  git -C "$ARC_SRC" checkout --quiet --detach "$ARC_REV"
  [ "$(git -C "$ARC_SRC" rev-parse HEAD)" = "$ARC_REV" ] || fail "the arcform clone is not at ${ARC_REV}"
  say "arcform ${ARC_REV:0:12} cloned from cargo's checkout into ${ARC_SRC}"
}

# cargo in the arcform checkout, linked against Meridian's library rather than
# building DuckDB from source, with the library found by an rpath at run time.
arc_cargo() {
  ( cd "$ARC_SRC" && env \
      RUSTUP_TOOLCHAIN="$TOOLCHAIN" \
      CARGO_TARGET_DIR="$ARC_TARGET" \
      DUCKDB_LIB_DIR="${MANAGED_DIR}/lib" \
      DUCKDB_INCLUDE_DIR="${MANAGED_DIR}/include" \
      RUSTFLAGS="-C link-arg=-Wl,-rpath,${MANAGED_DIR}/lib" \
      cargo "$@" )
}

if [ -n "${ARC_BIN:-}" ]; then
  ARC="$ARC_BIN"
else
  [ -f "${MANAGED_DIR}/lib/libduckdb.dylib" ] || fail "no managed DuckDB library in ${MANAGED_DIR}/lib: run cargo xtask ci --step duckdb-managed"
  arcform_source
  say "building arc from arcform ${ARC_REV:0:12}"
  arc_cargo build --locked --bin arc
  ARC="${ARC_TARGET}/debug/arc"
fi
[ -x "$ARC" ] || fail "no arc at ${ARC}"
ARC=$(cd "$(dirname "$ARC")" && pwd -P)/$(basename "$ARC")

# ---- the Protocols ---------------------------------------------------------

# The names of a manifest's top-level steps, one per line, in order.
step_names() {
  awk '
    /^steps:[ \t]*$/ { in_steps = 1; next }
    in_steps && /^[^ \t#-]/ { in_steps = 0 }
    in_steps && /^[ ]*- / {
      match($0, /^[ ]*/); if (ind < 0) ind = RLENGTH
      if (RLENGTH == ind) { idx++; pending = 1 }
    }
    in_steps && pending && /name:/ {
      v = $0; sub(/^[ ]*(- )?name:[ ]*/, "", v); gsub(/^["\047]|["\047][ ]*$/, "", v)
      print v; pending = 0
    }
    BEGIN { ind = -1 }
  ' "$1"
}

# prepare NAME FOLDER FETCHED LOAD: the copy of the Protocol every run of this
# case starts from, at $PAR/prepared/NAME/<folder name>.
prepare() {
  local name="$1" src="${ROOT}/$2" fetched="$3" load="$4"
  local into="${PAR}/prepared/${name}"
  local copy
  copy="${into}/$(basename "$src")"
  rm -rf "$into"
  mkdir -p "$into"
  cp -R "$src" "$copy"
  rm -rf "${copy}/build"
  local arc_env=(env ARCFORM_DB_DIR="${into}/.db" ARCFORM_HISTORY_DIR="${into}/.history")
  if [ -n "$fetched" ]; then
    local out="${fetched%%=*}" file="${ROOT}/${fetched#*=}"
    [ -f "$file" ] || fail "${name}: no copy at ${file}"
    mkdir -p "$(dirname "${copy}/${out}")"
    cp "$file" "${copy}/${out}"
  fi
  if [ -n "$load" ]; then
    local step="${load%%:*}" rest="${load#*:}"
    local model="${rest%%=*}" file="${ROOT}/${rest#*=}"
    [ -f "$file" ] || fail "${name}: no copy at ${file}"
    local at
    at=$(step_names "${copy}/arcform.yaml" | awk -v s="$step" '$0 == s { print NR - 1; exit }')
    [ -n "$at" ] || fail "${name}: the manifest has no step ${step} for the copy to stand in for"
    [ -f "${copy}/${model}" ] || fail "${name}: no model ${model} for step ${step}"
    local i
    for (( i = at - 1; i >= 0; i-- )); do
      ( cd "$copy" && "${arc_env[@]}" "$ARC" edit-protocol delete "steps[$i]" ) > /dev/null < /dev/null \
        || fail "${name}: arc could not drop steps[$i]"
    done
    mkdir -p "${copy}/data"
    cp "$file" "${copy}/data/$(basename "$file")"
    printf 'CREATE OR REPLACE TABLE "%s" AS\nSELECT * FROM read_parquet('"'"'data/%s'"'"')\n' \
      "$step" "$(basename "$file")" > "${copy}/${model}"
  fi
  "${HERE}/drop-fetch-steps.sh" "$copy" env ARCFORM_DB_DIR="${into}/.db" ARCFORM_HISTORY_DIR="${into}/.history" "$ARC" > /dev/null \
    || fail "${name}: its fetch steps cannot be dropped, so it cannot run offline"
  rm -rf "${into}/.db" "${into}/.history"
  echo "$copy"
}

# The files a run left in the Protocol's folder, by SHA-256, other than its
# run records and any database.
hash_files() {
  python3 -I - "$1" <<'PY'
import hashlib, os, sys
root = sys.argv[1]
rows = []
for d, dirs, files in os.walk(root):
    rel_d = os.path.relpath(d, root)
    if rel_d == os.path.join("build", ".arcform", "runs"):
        continue
    for f in files:
        if f.endswith((".duckdb", ".duckdb.wal")):
            continue
        p = os.path.join(d, f)
        with open(p, "rb") as h:
            rows.append((os.path.relpath(p, root), hashlib.sha256(h.read()).hexdigest()))
for rel, digest in sorted(rows):
    print(f"{digest}  {rel}")
PY
}

# The tables and views of database DB into folder OUT, read by the release
# command line.
dump_tables() {
  local db="$1" out="$2"
  mkdir -p "$out"
  local read=("$RELEASE" -init /dev/null -readonly "$db")
  "${read[@]}" -csv -c "SELECT schema_name, view_name, sql FROM duckdb_views() WHERE database_name = current_database() AND NOT internal ORDER BY ALL" > "${out}/views.csv"
  "${read[@]}" -csv -c "SELECT schema_name, function_name, function_type, parameters, macro_definition FROM duckdb_functions() WHERE database_name = current_database() AND NOT internal ORDER BY ALL" > "${out}/macros.csv"
  local schema table qualified
  while IFS=$'\t' read -r schema table; do
    qualified="\"${schema//\"/\"\"}\".\"${table//\"/\"\"}\""
    "${read[@]}" -csv -c "SELECT column_name, column_type, \"null\" FROM (DESCRIBE ${qualified})" > "${out}/${schema}.${table}.schema"
    # arc keeps its own state in tables named _arcform_*: the runs it made and
    # when each step last ran. Their ids and times change on every run, so
    # those columns are left out of the comparison. No other column is.
    local columns='*'
    case "$table" in
      _arcform_*) columns="COLUMNS(lambda c: c NOT IN ('run_id', 'started_at', 'finished_at', 'last_run_at'))" ;;
    esac
    "${read[@]}" -c "COPY (SELECT ${columns} FROM ${qualified} ORDER BY ALL) TO '${out}/${schema}.${table}.csv' (FORMAT csv, HEADER)" > /dev/null
  done < <("${read[@]}" -noheader -separator $'\t' -list -c \
    "SELECT schema_name, table_name FROM duckdb_tables() WHERE database_name = current_database() AND NOT internal ORDER BY ALL")
}

# run_and_capture COPY CLI CAPTURE: one `arc run` of a fresh copy of COPY on the
# command line CLI, captured into CAPTURE. Every run uses the same paths.
run_and_capture() {
  local prepared="$1" cli="$2" capture="$3"
  local work="${PAR}/run"
  local copy
  copy="${work}/$(basename "$prepared")"
  rm -rf "$work" "$capture"
  mkdir -p "${work}/db" "${work}/history" "$capture"
  cp -R "$prepared" "$copy"
  local status=0
  ( cd "$copy" && env ARCFORM_DB_DIR="${work}/db" ARCFORM_HISTORY_DIR="${work}/history" \
      ARC_DUCKDB_BIN="$cli" "$ARC" run ) > "${capture}/stdout" 2> "${capture}/stderr.log" < /dev/null || status=$?
  echo "$status" > "${capture}/exit"
  # stderr is kept to be read, under a name the comparison skips.
  mv "${capture}/stderr.log" "${capture}.stderr"
  if [ -d "${copy}/build/.arcform/runs" ]; then
    normalise "${copy}/build/.arcform/runs" "${capture}/records"
  fi
  hash_files "$copy" > "${capture}/files"
  local dbs=()
  while IFS= read -r db; do dbs+=("$db"); done < <(find "${work}/db" "$copy" -name '*.duckdb' -type f | LC_ALL=C sort)
  if [ "${#dbs[@]}" -gt 1 ]; then
    fail "$(basename "$prepared"): the run left ${#dbs[@]} databases (${dbs[*]}); this check reads one"
  elif [ "${#dbs[@]}" -eq 1 ]; then
    dump_tables "${dbs[0]}" "${capture}/tables"
  fi
}

failed=0
if [ "$PROTOCOLS" -eq 1 ]; then
  # Every Protocol folder Brightfield ships, and the agent Protocols, must be
  # a case or restricted.
  listed=()
  for c in "${CASES[@]}"; do IFS='|' read -r _ dir _ _ <<< "$c"; listed+=("$dir"); done
  for r in "${RESTRICTED[@]}"; do listed+=("${r%%|*}"); done
  while IFS= read -r manifest; do
    dir=$(dirname "${manifest#"${ROOT}/"}")
    found=0
    for l in "${listed[@]}"; do [ "$l" = "$dir" ] && found=1; done
    [ "$found" -eq 1 ] || { echo "FAIL ${dir}: a shipped Protocol this check does not run; add it to CASES or RESTRICTED"; failed=1; }
  done < <(find "${ROOT}/examples" "${ROOT}/tests/fixtures/agent-protocols" -name arcform.yaml -type f -not -path '*/build/*' | LC_ALL=C sort)

  for r in "${RESTRICTED[@]}"; do
    echo "restricted in the managed edition  ${r%%|*}: ${r#*|}"
  done
  for c in "${CASES[@]}"; do
    IFS='|' read -r name dir fetched load <<< "$c"
    prepared=$(prepare "$name" "$dir" "$fetched" "$load")
    caps="${PAR}/captures/${name}"
    run_and_capture "$prepared" "$RELEASE" "${caps}/release"
    run_and_capture "$prepared" "$RELEASE" "${caps}/release-again"
    run_and_capture "$prepared" "$MANAGED" "${caps}/managed"
    if [ "$(cat "${caps}/release/exit")" != 0 ]; then
      echo "FAIL ${name}: arc run failed on the release command line (exit $(cat "${caps}/release/exit")); ${caps}/release.stderr says why"
      sed 's/^/    /' "${caps}/release.stderr" | tail -15
      failed=1
    elif ! compare "${caps}/release" "${caps}/release-again" > "${caps}/release-again.diff"; then
      echo "FAIL ${name}: two runs on the release command line differ, so the Protocol's result is not fixed and parity cannot be judged"
      sed 's/^/  /' "${caps}/release-again.diff"
      failed=1
    elif ! compare "${caps}/release" "${caps}/managed" > "${caps}/managed.diff"; then
      echo "FAIL ${name}: Meridian's build differs from the release command line"
      sed 's/^/  /' "${caps}/managed.diff"
      failed=1
    else
      echo "identical  ${name}"
    fi
  done
fi

# ---- arc's engine and tokenizer tests --------------------------------------

if [ "$ARC_TESTS" -eq 1 ]; then
  [ -f "${MANAGED_DIR}/lib/libduckdb.dylib" ] || fail "no managed DuckDB library in ${MANAGED_DIR}/lib: run cargo xtask ci --step duckdb-managed"
  arcform_source
  managed_bin_dir=$(dirname "$managed_real")
  arc_tests() {
    say "arc's tests: cargo test $*"
    PATH="${managed_bin_dir}:${PATH}" ARC_DUCKDB_RECORD_BIN="$managed_real" arc_cargo test --locked "$@" \
      || { echo "FAIL arc's tests on Meridian's command line: cargo test $*"; failed=1; }
  }
  arc_tests --test engine_contract --test engine_change_notice
  arc_tests --lib duckdb_lineage -- --include-ignored
  arc_tests --lib -- --include-ignored duckdb_tokens engine::extension_tests
  arc_tests --test duckdb_reader_switch -- --include-ignored
  arc_tests --test reader_parity -- --include-ignored
  [ "$failed" -eq 0 ] && echo "arc's engine and tokenizer tests pass on Meridian's command line"
fi

if [ "$failed" -ne 0 ]; then
  echo "parity check FAILED; the captures are in ${PAR}/captures"
  exit 1
fi
echo "parity check passed"
