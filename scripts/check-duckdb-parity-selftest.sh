#!/usr/bin/env bash
# Prove scripts/check-duckdb-parity.sh's comparison fails when one output of a
# run differs by a byte, and passes when two runs differ only in what changes
# from one run to the next.
#
# The parity check captures each run of a Protocol into a folder: its exit
# status, what arc printed, its run records with the run's id, timestamps and
# durations taken out (--normalise), a hash of every file the run left in the
# Protocol's folder, and each table in its database as sorted CSV. Two captures
# are then compared (--compare). Both halves are tested here on captures made
# by hand, so the test needs no DuckDB, no arc and no build.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PARITY="$HERE/check-duckdb-parity.sh"

failures=0
TMP="$(mktemp -d)" || exit 1
trap 'rm -rf "$TMP"' EXIT

ok() { echo "  ok   $1"; }
fail() {
  echo "  FAIL $1"
  [ -f "$TMP/out" ] && sed 's/^/       /' "$TMP/out"
  failures=$((failures + 1))
}

[ -x "$PARITY" ] || { echo "no executable $PARITY" >&2; exit 1; }

# A run's state folder as arc leaves it: one run record and its event log.
# record RUN_ID STARTED DURATION ROWS: the record, on stdout.
record() {
  cat <<JSON
{"contract_version":"b5/1",
 "run":{"run_id":"$1","attempt_id":"$1","protocol":{"name":"p","dir":"/w/p"},
        "engine":{"duckdb_cli":"v2.0.0"},"started_at":"$2","finished_at":"$2","outcome":"success"},
 "assets":[{"id":"table:t","row_count":$4,"content_hash":"abc"}],
 "steps":[{"name":"t","duration_sec":$3,"status":{"state":"success"},
           "io":{"stdout_path":"build/.arcform/runs/$1/t.out"}}]}
JSON
}
events() {
  printf '{"step":"t","state":"success","ts":"%s"}\n{"event":"run_complete","outcome":"success","ts":"%s"}\n' "$1" "$1"
}

# capture NAME RUN_ID STARTED DURATION ROWS: a capture folder as the check
# writes one, its run records put through --normalise.
capture() {
  local dir="$TMP/$1" runs="$TMP/$1.runs"
  rm -rf "$dir" "$runs"
  mkdir -p "$dir/tables" "$runs"
  record "$2" "$3" "$4" "$5" > "$runs/$2.json"
  events "$3" > "$runs/$2.jsonl"
  echo 0 > "$dir/exit"
  printf '[1/1] t ...\n1/1 steps succeeded.\n' > "$dir/stdout"
  printf 'abc123  data/a.csv\n' > "$dir/files"
  printf 'column_name,column_type\nx,INTEGER\n' > "$dir/tables/main.t.schema"
  printf 'x\n1\n2\n' > "$dir/tables/main.t.csv"
  "$PARITY" --normalise "$runs" "$dir/records" > "$TMP/out" 2>&1 || return 1
  echo "$dir"
}

# expect_same NAME A B / expect_differs NAME A B FILE: --compare's verdict, and
# for a difference, that the file that differs is named.
expect_same() {
  if "$PARITY" --compare "$2" "$3" > "$TMP/out" 2>&1; then ok "$1"; else fail "$1"; fi
}
expect_differs() {
  if "$PARITY" --compare "$2" "$3" > "$TMP/out" 2>&1; then
    fail "$1 (compared equal)"
  elif ! grep -qF -- "$4" "$TMP/out"; then
    fail "$1 (the difference does not name $4)"
  else
    ok "$1"
  fi
}

A=$(capture a 20261118-081602-347ae7f0 2026-11-18T08:16:03Z 0.105 2) || { fail "a capture is made"; exit 1; }
B=$(capture b 20261118-091744-0c1d2e3f 2026-11-18T09:17:45Z 0.311 2) || { fail "a second capture is made"; exit 1; }

echo "--normalise and --compare"
expect_same "a capture compares equal with itself" "$A" "$A"
expect_same "two runs differing only in run id, timestamps and durations compare equal" "$A" "$B"

# Each mutation below is one byte, in one kind of output, in a copy of B.
mutate() { rm -rf "$TMP/m"; cp -R "$B" "$TMP/m"; echo "$TMP/m"; }

M=$(mutate); printf 'x\n1\n3\n' > "$M/tables/main.t.csv"
expect_differs "a table value one byte apart fails" "$A" "$M" "tables/main.t.csv"

M=$(mutate); printf 'column_name,column_type\nx,BIGINT\n' > "$M/tables/main.t.schema"
expect_differs "a column's type differing fails" "$A" "$M" "tables/main.t.schema"

M=$(mutate); echo 1 > "$M/exit"
expect_differs "an exit status differing fails" "$A" "$M" "exit"

M=$(mutate); printf '[1/1] t ...\n0/1 steps succeeded.\n' > "$M/stdout"
expect_differs "what arc printed differing by a byte fails" "$A" "$M" "stdout"

M=$(mutate); printf 'abc124  data/a.csv\n' > "$M/files"
expect_differs "a file the run left differing fails" "$A" "$M" "files"

C=$(capture c 20261118-091744-0c1d2e3f 2026-11-18T09:17:45Z 0.311 3) || fail "a third capture is made"
expect_differs "a run record's row count differing fails" "$A" "$C" "records/"

M=$(mutate); rm "$M/tables/main.t.csv"
expect_differs "a table missing on one side fails" "$A" "$M" "main.t.csv"

M=$(mutate); printf 'y\n' > "$M/tables/main.u.csv"
expect_differs "a table only one side made fails" "$A" "$M" "main.u.csv"

M=$(mutate); sed '$ s/"success"}$/"failure"}/' "$M/records/1.jsonl" > "$TMP/e" && mv "$TMP/e" "$M/records/1.jsonl"
grep -q '"failure"' "$M/records/1.jsonl" || fail "the event-log mutation took effect"
expect_differs "an event log's outcome differing fails" "$A" "$M" "records/"

M=$(mutate); rm -rf "$M/records"; echo "no run record" > "$M/no-run-record"; echo 134 > "$M/exit"
expect_differs "a run that died before writing its record fails" "$A" "$M" "no-run-record"

M=$(mutate); printf 'y\n' > "$M/tables/main.u.csv"
if (cd "$TMP" && "$PARITY" --compare "$(basename "$A")" "$(basename "$M")") > "$TMP/out" 2>&1; then
  fail "given relative paths, a file only the second capture holds fails (compared equal)"
elif ! grep -qF "main.u.csv" "$TMP/out"; then
  fail "given relative paths, a file only the second capture holds fails (main.u.csv not named)"
else
  ok "given relative paths, a file only the second capture holds fails"
fi

echo "what the comparison refuses"
mkdir -p "$TMP/empty1" "$TMP/empty2"
if "$PARITY" --compare "$TMP/empty1" "$TMP/empty2" > "$TMP/out" 2>&1; then
  fail "two empty captures are refused, not taken as equal"
else
  ok "two empty captures are refused, not taken as equal"
fi
if "$PARITY" --compare "$A" "$TMP/nonexistent" > "$TMP/out" 2>&1; then
  fail "a capture that is not there is refused"
else
  ok "a capture that is not there is refused"
fi
rm -rf "$TMP/runs-none"; mkdir -p "$TMP/runs-none"
if "$PARITY" --normalise "$TMP/runs-none" "$TMP/records-none" > "$TMP/out" 2>&1; then
  fail "a run that left no run record is refused by --normalise"
else
  ok "a run that left no run record is refused by --normalise"
fi

echo
if [ "$failures" -eq 0 ]; then
  echo "self-test passed: a one-byte difference in each kind of output fails the comparison"
else
  echo "self-test FAILED: $failures case(s)"
  exit 1
fi
