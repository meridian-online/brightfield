#!/usr/bin/env bash
# Drop a Protocol's http_fetch steps whose file is already there, so that
# running it needs no network.
#
#   scripts/drop-fetch-steps.sh <protocol-dir> <arc> [<arc args>...]
#
# <arc> is the command that runs arc: an `arc` executable, or a Brightfield
# binary in its runner mode (`env BRIGHTFIELD_RUN_AS_ARC=1 <brightfield>`),
# which carries the arc Brightfield is pinned to.
#
# Run it on a COPY. It edits the folder's arcform.yaml, through arc's own
# `edit-protocol delete`, and arc records the edit in the folder's
# arcform-log.txt and in its history store.
#
# Why drop rather than run: arc's http_fetch does not take an existing file as
# done. Unless the step pins a sha256 the file matches, it GETs the URL again
# (conditionally when a `<out>.arcmeta` sidecar gives it a validator, which is
# still a request) and writes the answer over the file. A check that has to
# run offline, or that must not replace the file it was handed, drops the step
# and reads the file that is there. It is the handling the test suite's copies
# of the agent-written fixtures get.
#
# A fetch step whose file is NOT there cannot be dropped, since the steps after
# it would read nothing: that is reported on stderr and the exit is 1, with
# arcform.yaml unchanged.
#
# Prints one line per dropped step, on stdout.
set -euo pipefail

DIR="${1:?usage: scripts/drop-fetch-steps.sh <protocol-dir> <arc> [<arc args>...]}"
shift
[ "$#" -ge 1 ] || { echo "usage: scripts/drop-fetch-steps.sh <protocol-dir> <arc> [<arc args>...]" >&2; exit 2; }
MANIFEST="$DIR/arcform.yaml"
[ -f "$MANIFEST" ] || { echo "no arcform.yaml in $DIR" >&2; exit 2; }

# The http_fetch steps, as "<index><TAB><out>", read from the manifest's
# top-level `steps:` block sequence. An item starts at a `- ` line at the
# block's own indent; the block ends at the next top-level key.
fetch_steps() {
  awk '
    function flush() { if (idx >= 0 && fetch) print idx "\t" out; fetch = 0; out = "" }
    BEGIN { in_steps = 0; idx = -1; ind = -1 }
    /^steps:[ \t]*$/ { in_steps = 1; next }
    in_steps {
      if ($0 ~ /^[^ \t#-]/) { flush(); in_steps = 0; next }
      if ($0 ~ /^[ ]*- /) {
        match($0, /^[ ]*/)
        if (ind < 0) ind = RLENGTH
        if (RLENGTH == ind) { flush(); idx++ }
      }
      line = $0
      sub(/^[ ]*(- )?/, "", line)
      if (line ~ /^op:[ ]*["\047]?http_fetch["\047]?[ ]*$/) fetch = 1
      if (line ~ /^out:/) {
        v = line
        sub(/^out:[ ]*/, "", v)
        sub(/[ ]+#.*$/, "", v)
        gsub(/^["\047]|["\047][ ]*$/, "", v)
        out = v
      }
    }
    END { flush() }
  ' "$MANIFEST"
}

STEPS=$(fetch_steps)
[ -n "$STEPS" ] || exit 0

# Every file first, so a missing one leaves the manifest as it was.
while IFS=$'\t' read -r index out; do
  if [ -z "$out" ] || [ ! -f "$DIR/$out" ]; then
    echo "steps[$index] is an http_fetch whose file (${out:-no out}) is not in the folder; running it needs the network" >&2
    exit 1
  fi
done <<< "$STEPS"

# Highest index first, so each delete leaves the earlier indices where they were.
while IFS=$'\t' read -r index out; do
  ( cd "$DIR" && "$@" edit-protocol delete "steps[$index]" ) > /dev/null
  echo "dropped steps[$index], the fetch of $out (the file is there)"
done < <(printf '%s\n' "$STEPS" | sort -t$'\t' -k1,1rn)

# The block reading above is of the text, not arc's parse of it, so check the
# result: an http_fetch still in the file means the reading missed a step.
if grep -Eq "op:[ ]*[\"']?http_fetch" "$MANIFEST"; then
  echo "an http_fetch step is still in $MANIFEST after the drop" >&2
  exit 1
fi
