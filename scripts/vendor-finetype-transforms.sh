#!/usr/bin/env bash
# Write the transformation contract of every FineType label at the pinned tag into
# crates/brightfield-protocol/vendor/finetype/transforms.json.
#
#   scripts/vendor-finetype-transforms.sh            # fetch labels/definitions_*.yaml at the pin
#   scripts/vendor-finetype-transforms.sh --from DIR # read them from a checkout's labels/ folder
#
# The silver cast applies each label's `transform` (and falls back to its DuckDB type), and
# neither is reachable at run time: the bundle's catalogue carries no transforms and the
# extension's functions infer per value. Depending on `finetype-core` would pull a tokenizer,
# an HTTP client and a C crypto build into the crate for a table of strings, so the table is
# vendored instead and scripts/check-finetype-pin.sh refuses it when its tag is not the pin.
#
# `broad_type` is mapped to the DuckDB type name exactly as `DdlInfo::duckdb_type_from_broad_type`
# does at that tag. Needs `gh` (or --from) and python3 with PyYAML.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "${HERE}/.." && pwd)
OUT="${ROOT}/crates/brightfield-protocol/vendor/finetype/transforms.json"
TAG=$("${HERE}/finetype-pin.sh" --tag)
REPO=meridian-online/finetype

SRC=$(mktemp -d "${TMPDIR:-/tmp}/bf-finetype-labels.XXXXXX")
trap 'rm -rf "$SRC"' EXIT

if [ "${1:-}" = "--from" ]; then
  cp "$2"/definitions_*.yaml "$SRC/"
else
  names=$(gh api "repos/${REPO}/contents/labels?ref=${TAG}" --jq '.[].name' | grep -E '^definitions_.*\.yaml$')
  for name in $names; do
    gh api -H 'Accept: application/vnd.github.raw' "repos/${REPO}/contents/labels/${name}?ref=${TAG}" > "${SRC}/${name}"
  done
fi

python3 -I - "$SRC" "$TAG" "$OUT" <<'PY'
import glob, json, os, sys
import yaml

src, tag, out = sys.argv[1:4]
types = {
    "VARCHAR": "VARCHAR", "DOUBLE": "DOUBLE", "BIGINT": "BIGINT", "SMALLINT": "SMALLINT",
    "DECIMAL": "DECIMAL", "DATE": "DATE", "TIMESTAMP": "TIMESTAMP",
    "TIMESTAMPTZ": "TIMESTAMP WITH TIME ZONE", "TIME": "TIME", "INTERVAL": "INTERVAL",
    "POINT": "POINT", "BOOLEAN": "BOOLEAN", "UUID": "UUID", "JSON": "JSON",
    "STRUCT": "STRUCT", "LIST": "LIST", "ENUM": "VARCHAR",
}
labels = {}
files = sorted(glob.glob(os.path.join(src, "definitions_*.yaml")))
if not files:
    sys.exit("vendor-finetype-transforms: no definitions_*.yaml found")
for f in files:
    for label, d in yaml.safe_load(open(f)).items():
        transform = d.get("transform")
        labels[label] = {
            "duckdb_type": types.get(d.get("broad_type"), "VARCHAR"),
            "transform": transform if isinstance(transform, str) else None,
        }
doc = {
    "tag": tag,
    "source": "https://github.com/meridian-online/finetype/blob/<tag>/labels/definitions_*.yaml",
    "labels": dict(sorted(labels.items())),
}
with open(out, "w") as fh:
    json.dump(doc, fh, indent=1, sort_keys=True, ensure_ascii=False)
    fh.write("\n")
print(f"vendor-finetype-transforms: {len(labels)} labels at {tag} -> {out}")
PY
