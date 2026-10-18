#!/usr/bin/env bash
# Prove scripts/drop-fetch-steps.sh finds each spelling of an http_fetch step,
# drops it only when its file is there, and refuses what it cannot read.
#
# The script deletes through arc (`edit-protocol delete steps[N]`). Here arc is
# a stand-in that logs each delete and removes that block item with awk, so the
# test needs no arc build and no network: what it checks is the script's
# reading of the manifest, the order it deletes in, its refusals, and the final
# check that no fetch step is left.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DROP="$HERE/drop-fetch-steps.sh"

failures=0
TMP="$(mktemp -d)" || exit 1
trap 'rm -rf "$TMP"' EXIT

# The stand-in arc: `edit-protocol delete steps[N]`, in the current directory.
ARC="$TMP/arc"
cat > "$ARC" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
[ "$1 $2" = "edit-protocol delete" ] || { echo "stand-in arc: unexpected $*" >&2; exit 2; }
n="${3#steps[}"; n="${n%]}"
echo "delete $n" >> "$DELETES"
awk -v n="$n" '
  /^steps:[ \t]*$/ { in_steps = 1; print; next }
  in_steps && /^[^ \t#-]/ { in_steps = 0 }
  in_steps && /^  - / { idx++ }
  !(in_steps && idx == n + 1) { print }
  BEGIN { idx = 0 }
' arcform.yaml > arcform.yaml.new
mv arcform.yaml.new arcform.yaml
STUB
chmod +x "$ARC"
export DELETES="$TMP/deletes"

# protocol NAME: a fresh folder with data/a.csv and data/b.csv, the manifest
# on stdin.
protocol() {
  local dir="$TMP/$1"
  rm -rf "$dir"
  mkdir -p "$dir/data"
  : > "$dir/data/a.csv"
  : > "$dir/data/b.csv"
  cat > "$dir/arcform.yaml"
  : > "$DELETES"
  echo "$dir"
}

ok() { echo "  ok   $1"; }
fail() {
  echo "  FAIL $1"
  [ -f "$TMP/out" ] && sed 's/^/       /' "$TMP/out"
  failures=$((failures + 1))
}

# expect_drop NAME DIR DELETES-WANTED: exits 0, deleted those indices in that
# order, and no http_fetch is left while the SQL step is.
expect_drop() {
  local name="$1" dir="$2" wanted="$3"
  local status=0
  "$DROP" "$dir" "$ARC" > "$TMP/out" 2>&1 || status=$?
  [ "$status" -eq 0 ] || { fail "$name: expected a drop, got exit $status"; return; }
  local got
  got=$(paste -sd ' ' - < "$DELETES")
  [ "$got" = "$wanted" ] || { fail "$name: deleted '$got', wanted '$wanted'"; return; }
  if grep -q http_fetch "$dir/arcform.yaml"; then fail "$name: an http_fetch is still there"; return; fi
  grep -q "name: load" "$dir/arcform.yaml" || { fail "$name: the SQL step went too"; return; }
  ok "$name"
}

# expect_refusal NAME DIR NEEDLE: exits non-zero, says NEEDLE, and leaves the
# manifest as it was.
expect_refusal() {
  local name="$1" dir="$2" needle="$3"
  cp "$dir/arcform.yaml" "$TMP/before"
  if "$DROP" "$dir" "$ARC" > "$TMP/out" 2>&1; then fail "$name: expected a refusal, got exit 0"; return; fi
  grep -qF -- "$needle" "$TMP/out" || { fail "$name: refused without saying '$needle'"; return; }
  cmp -s "$TMP/before" "$dir/arcform.yaml" || { fail "$name: the manifest changed"; return; }
  [ ! -s "$DELETES" ] || { fail "$name: it deleted something before refusing"; return; }
  ok "$name"
}

echo "== spellings that are dropped"
expect_drop "plain" "$(protocol plain <<'Y'
name: p
steps:
  - name: fetch
    op: http_fetch
    with:
      out: data/a.csv
      url: https://example.invalid/a.csv
  - name: load
    sql: models/load.sql
Y
)" "delete 0"

expect_drop "quoted op and out" "$(protocol quoted <<'Y'
name: p
steps:
  - name: fetch
    op: "http_fetch"
    with:
      out: 'data/a.csv'
      url: https://example.invalid/a.csv
  - name: load
    sql: models/load.sql
Y
)" "delete 0"

expect_drop "versioned op (http_fetch@1)" "$(protocol versioned <<'Y'
name: p
steps:
  - name: load
    sql: models/load.sql
  - name: fetch
    op: http_fetch@1
    with:
      out: data/a.csv
      url: https://example.invalid/a.csv
Y
)" "delete 1"

expect_drop "op on the first line, a flow-mapping with:, two fetches, highest first" "$(protocol two <<'Y'
name: p
steps:
  - op: http_fetch
    name: fetch_a
    with: { out: data/a.csv, url: https://example.invalid/a.csv }
  - name: fetch_b
    op: 'http_fetch@2'
    with:
      out: data/b.csv   # the second file
      url: https://example.invalid/b.csv
  - name: load
    sql: models/load.sql
assets: {}
Y
)" "delete 1 delete 0"

expect_drop "a with: flow mapping written across lines" "$(protocol multiline <<'Y'
name: p
steps:
  - name: fetch
    op: http_fetch
    with: {
      out: data/a.csv,
      url: https://example.invalid/a.csv
    }
  - name: load
    sql: models/load.sql
Y
)" "delete 0"

expect_drop "the last entry of a multi-line with: mapping" "$(protocol multiline_last <<'Y'
name: p
steps:
  - name: fetch
    op: http_fetch
    with: {
      url: https://example.invalid/a.csv,
      out: "data/a.csv" }
  - name: load
    sql: models/load.sql
Y
)" "delete 0"

echo "== nothing to drop"
dir=$(protocol none <<'Y'
name: p
steps:
  - name: load
    sql: models/load.sql
Y
)
if "$DROP" "$dir" "$ARC" > "$TMP/out" 2>&1 && [ ! -s "$DELETES" ]; then ok "no fetch step"; else fail "no fetch step"; fi

# expect_untouched NAME DIR: exits 0, deletes nothing, leaves the manifest as
# it was.
expect_untouched() {
  local name="$1" dir="$2"
  cp "$dir/arcform.yaml" "$TMP/before"
  local status=0
  "$DROP" "$dir" "$ARC" > "$TMP/out" 2>&1 || status=$?
  [ "$status" -eq 0 ] || { fail "$name: expected exit 0, got $status"; return; }
  [ ! -s "$DELETES" ] || { fail "$name: it deleted a step"; return; }
  cmp -s "$TMP/before" "$dir/arcform.yaml" || { fail "$name: the manifest changed"; return; }
  ok "$name"
}

expect_untouched "a commented-out fetch step" "$(protocol commented <<'Y'
name: p
steps:
  # - name: fetch
  #   op: http_fetch
  #   with: { out: data/never.csv }
  - name: load
    sql: models/load.sql
Y
)"

expect_untouched "a flow-style step that only mentions http_fetch" "$(protocol mention <<'Y'
name: p
steps:
  - {name: load_http_fetch_output, sql: models/http_fetch.sql}
Y
)"

echo "== what it refuses"
expect_refusal "flow-style item" "$(protocol flow <<'Y'
name: p
steps:
  - {name: fetch, op: http_fetch, with: {out: data/a.csv, url: https://example.invalid/a.csv}}
  - name: load
    sql: models/load.sql
Y
)" "flow style"

expect_refusal "flow-style versioned item" "$(protocol flow1 <<'Y'
name: p
steps:
  - { name: fetch, op: http_fetch@1, with: { out: data/a.csv } }
  - name: load
    sql: models/load.sql
Y
)" "flow style"

expect_refusal "a flow-style steps list" "$(protocol flowseq <<'Y'
name: p
steps: [{name: fetch, op: http_fetch, with: {out: data/a.csv}}, {name: load, sql: models/load.sql}]
Y
)" "in a form this script does not read"

expect_refusal "a fetch whose file is not there" "$(protocol missing <<'Y'
name: p
steps:
  - name: fetch
    op: http_fetch
    with:
      out: data/never.csv
      url: https://example.invalid/never.csv
  - name: load
    sql: models/load.sql
Y
)" "needs the network"

if [ "$failures" -ne 0 ]; then
  echo "drop-fetch-steps-selftest: ${failures} case(s) did not behave as required." >&2
  exit 1
fi
echo "drop-fetch-steps-selftest: every spelling of a fetch step is found, dropped only when its file is there, and what it cannot read is refused."
