#!/usr/bin/env bash
# Retire the tap's formula and point `brew` at the cask that replaces it.
#
# Usage:  scripts/move-tap-to-cask.sh TAPDIR
#
# Removes TAPDIR/Formula/brightfield.rb when it is there, and adds
# "brightfield": "meridian-online/tap/brightfield" to TAPDIR/tap_migrations.json,
# creating the file when the tap has none. Every other key is kept, and the file
# is written sorted with two-space indent so a rerun on a finished tap changes
# nothing and the release's no-op guard still sees "no changes".
#
# Why the migration entry: a formula and a cask of one name cannot both live in
# a tap, and a person with the formula installed has `brightfield` linked from
# the Cellar. The entry is what makes `brew upgrade` move them to the cask
# instead of leaving them on a formula the tap no longer carries.
#
# Exit codes: 0 done; 2 TAPDIR is not a directory or its tap_migrations.json is
# not a JSON object. Nothing is changed in either case.
set -euo pipefail

if [[ $# -ne 1 ]]; then
	echo "usage: $0 TAPDIR" >&2
	exit 2
fi
tap="$1"
if [[ ! -d "$tap" ]]; then
	echo "move-tap-to-cask: ${tap} is not a directory" >&2
	exit 2
fi

# Merge first, remove second: a malformed migrations file must leave the tap as
# it was, formula included.
python3 - "$tap/tap_migrations.json" <<'PY' || exit 2
import json, os, sys

path = sys.argv[1]
data = {}
if os.path.exists(path):
    try:
        with open(path, encoding="utf-8") as f:
            data = json.load(f)
    except ValueError as exc:
        print(f"move-tap-to-cask: {path} does not parse: {exc}", file=sys.stderr)
        sys.exit(2)
    if not isinstance(data, dict):
        print(f"move-tap-to-cask: {path} is not a JSON object", file=sys.stderr)
        sys.exit(2)
data["brightfield"] = "meridian-online/tap/brightfield"
with open(path, "w", encoding="utf-8") as f:
    json.dump(data, f, indent=2, sort_keys=True)
    f.write("\n")
PY

rm -f "$tap/Formula/brightfield.rb"
