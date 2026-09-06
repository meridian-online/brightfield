#!/usr/bin/env bash
# Prove scripts/fetch-finetype-bundle.sh refuses bytes that are not the ones
# published.
#
# The script it exercises runs on a tag and nowhere else, which is the shape
# that rots: nothing touches it between releases and a release is the worst
# moment to discover the checksum comparison stopped comparing. So this stands
# a throwaway HTTP server in front of throwaway assets, points both of the
# fetch script's sources at it, and runs the real file — url construction,
# curl, sidecar parse, digest comparison, the release metadata cross-check, the
# registry listing, the per-file digest comparison, and the assembly. Only the
# host differs from a release.
#
# TWO SOURCES AND TWO KINDS OF REFUSAL, which is the thing to keep straight.
# The release assets carry a `.sha256` written by their publisher, so a
# mismatch there means the bytes are not the ones FineType released. The model
# has no such sidecar; what it has is the registry's own listing at the pinned
# revision, carrying a sha256 for the files stored as LFS objects and a git
# blob id for the rest. Both refusals are exercised below and they do not mean
# the same thing — see the header of the script under test.
#
# WHAT THIS STILL DOES NOT REACH: the live release and the live registry. The
# asset names and the registry layout were read off the real ones; nothing here
# fetches from github.com or huggingface.co.
#
# Both directions, like the other gate self-tests here. A good asset set must
# assemble a bundle that scripts/check-bundled-extension.sh accepts — a fetch
# that refused everything would be reverted by the first person who needed it —
# and each way an input can be wrong must fail, leaving no bundle behind.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FETCH="$HERE/fetch-finetype-bundle.sh"
CHECK="$HERE/check-bundled-extension.sh"

TAG="$("$HERE/finetype-pin.sh" --tag)" || { echo "selftest: the pin does not read" >&2; exit 1; }
REVISION="$("$HERE/finetype-pin.sh" --revision)" || exit 1

# The target whose asset name these fixtures use. Any triple in
# scripts/duckdb-platform.sh would do; this one is the release matrix's native
# leg, so the platform stamp below is the one a packaging run would demand.
TARGET="aarch64-apple-darwin"
PLATFORM="$("$HERE/duckdb-platform.sh" "$TARGET")"

# The model name the fixture release metadata declares, and the directory its
# config names for the value embeddings. Deliberately NOT "model2vec": the
# published model calls it `value_model2vec`, and a fixture that used the
# convenient name would let a check that hardcodes one pass.
MODEL="m2v8m-s43"
EMBED="value_model2vec"
MODEL_REPO="meridian-online/finetype-model"

command -v python3 >/dev/null 2>&1 || {
	echo "selftest: python3 is required to serve the fixture assets" >&2
	exit 1
}

failures=0
TMP="$(mktemp -d)" || exit 1
SERVER_PID=""
cleanup() {
	if [[ -n "$SERVER_PID" ]]; then
		kill "$SERVER_PID" 2>/dev/null
		wait "$SERVER_PID" 2>/dev/null
	fi
	rm -rf "$TMP"
}
trap cleanup EXIT

SERVE="$TMP/serve"
RELEASE_DIR="$SERVE/release"
TREE_DIR="$SERVE/api/models/$MODEL_REPO/tree/$REVISION"
FILES_DIR="$SERVE/$MODEL_REPO/resolve/$REVISION/$MODEL"
DEST="$TMP/bundle"
out="$TMP/out"

EXT_ASSET="finetype-${TAG}-${TARGET}.duckdb_extension"
CATALOGUE_ASSET="taxonomy-schemas.json"
METADATA_ASSET="finetype-model.json"

digest_of() {
	if command -v shasum >/dev/null 2>&1; then
		shasum -a 256 "$1" | awk '{print $1}'
	else
		sha256sum "$1" | awk '{print $1}'
	fi
}

sidecar() { printf '%s  %s\n' "$(digest_of "$RELEASE_DIR/$1")" "$1" >"$RELEASE_DIR/$1.sha256"; }

# The registry's listing, derived from the fixture files rather than written
# out, so a fixture edit cannot leave the listing describing bytes that are no
# longer there — which would make a "goes red" case pass for the wrong reason.
#
# The two .safetensors are declared as LFS objects (sha256 of the content, the
# way the real registry declares them) and the JSON files by their git blob id,
# so both comparison paths in the script under test are exercised.
write_tree() {
	python3 - "$FILES_DIR" "$TREE_DIR/$MODEL" "$MODEL" <<'PY'
import hashlib, json, os, sys
root, out, model = sys.argv[1:4]
entries = []
for dirpath, _, names in os.walk(root):
    for name in sorted(names):
        full = os.path.join(dirpath, name)
        rel = os.path.relpath(full, root)
        data = open(full, "rb").read()
        entry = {"type": "file", "path": f"{model}/{rel}", "size": len(data)}
        if rel.endswith(".safetensors"):
            entry["oid"] = "a" * 40
            entry["lfs"] = {"oid": hashlib.sha256(data).hexdigest(), "size": len(data)}
        else:
            entry["oid"] = hashlib.sha1(b"blob %d\0" % len(data) + data).hexdigest()
        entries.append(entry)
os.makedirs(os.path.dirname(out), exist_ok=True)
json.dump(entries, open(out, "w"))
PY
}

# ---------------------------------------------------------------------------
# A complete, correct fixture. Rebuilt before every case so a mutation in one
# cannot leak into the next.
#
# The served directory is emptied rather than replaced: it is the fixture
# server's working directory, and removing it leaves the server answering every
# request with a connection reset, which reads in the report below as "the
# fetch refused" for every case at once.
# ---------------------------------------------------------------------------
make_fixture() {
	mkdir -p "$SERVE"
	find "$SERVE" -mindepth 1 -delete
	mkdir -p "$RELEASE_DIR" "$TREE_DIR" "$FILES_DIR/$EMBED"

	"$HERE/fixture-extension.py" "$RELEASE_DIR/$EXT_ASSET" "$PLATFORM" v1.2.0 "${TAG#v}" C_STRUCT
	printf '[{"x-finetype-label": "identity.person.email", "pattern": "^[^@]+@[^@]+$"}]\n' \
		>"$RELEASE_DIR/$CATALOGUE_ASSET"
	printf '{"tag": "%s", "model": "%s", "catalogue": "taxonomy-schemas.json", "label_map_entries": 1, "catalogue_entries": 1, "covered": 1, "coverage_fraction": 1.0, "threshold": 0.9}\n' \
		"$TAG" "$MODEL" >"$RELEASE_DIR/$METADATA_ASSET"
	sidecar "$EXT_ASSET"
	sidecar "$CATALOGUE_ASSET"
	sidecar "$METADATA_ASSET"

	printf '{"value_embed_model": "%s", "n_classes": 1}\n' "$EMBED" >"$FILES_DIR/config.json"
	printf '["identity.person.email"]\n' >"$FILES_DIR/label_map.json"
	printf 'fixture weights\n' >"$FILES_DIR/model.safetensors"
	printf 'fixture embed weights\n' >"$FILES_DIR/$EMBED/model.safetensors"
	printf '{"fixture": "tokenizer"}\n' >"$FILES_DIR/$EMBED/tokenizer.json"
	write_tree
}

cat >"$TMP/serve.py" <<'PY'
"""Serve a directory on an ephemeral loopback port and print the port."""
import http.server
import os
import socketserver
import sys

os.chdir(sys.argv[1])


class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *args):
        pass


socketserver.TCPServer.allow_reuse_address = True
with socketserver.TCPServer(("127.0.0.1", 0), Quiet) as httpd:
    print(httpd.server_address[1], flush=True)
    httpd.serve_forever()
PY

make_fixture
python3 "$TMP/serve.py" "$SERVE" >"$TMP/port" 2>"$TMP/server.err" &
SERVER_PID=$!

PORT=""
waited=0
while [[ $waited -lt 100 ]]; do
	PORT="$(tr -d '[:space:]' <"$TMP/port")"
	[[ -n "$PORT" ]] && break
	sleep 0.1
	waited=$((waited + 1))
done
[[ -n "$PORT" ]] || {
	echo "selftest: the fixture server never reported a port" >&2
	cat "$TMP/server.err" >&2
	exit 1
}
export BRIGHTFIELD_FINETYPE_ASSET_BASE="http://127.0.0.1:$PORT/release"
export BRIGHTFIELD_FINETYPE_MODEL_ORIGIN="http://127.0.0.1:$PORT"

# Prove the server is really answering before any assertion depends on it —
# otherwise every "goes red" case below would pass for the wrong reason.
for probe in "$BRIGHTFIELD_FINETYPE_ASSET_BASE/$EXT_ASSET" \
	"$BRIGHTFIELD_FINETYPE_MODEL_ORIGIN/api/models/$MODEL_REPO/tree/$REVISION/$MODEL?recursive=true" \
	"$BRIGHTFIELD_FINETYPE_MODEL_ORIGIN/$MODEL_REPO/resolve/$REVISION/$MODEL/config.json"; do
	curl -sSfL -o /dev/null "$probe" || {
		echo "selftest: the fixture server did not serve ${probe}" >&2
		cat "$TMP/server.err" >&2
		exit 1
	}
done

run_fetch() {
	rm -rf "$DEST"
	"$FETCH" "$TARGET" "$DEST" >"$out" 2>&1
}

expect_pass() {
	local name="$1"
	if run_fetch; then
		echo "  ok   ${name}"
	else
		echo "  FAIL ${name}: expected a bundle, got a refusal"
		sed 's/^/       /' "$out"
		failures=$((failures + 1))
	fi
}

expect_fail() {
	local name="$1" needle="$2"
	if run_fetch; then
		echo "  FAIL ${name}: expected a refusal, got a bundle"
		sed 's/^/       /' "$out"
		failures=$((failures + 1))
		return
	fi
	if ! grep -qF -- "$needle" "$out"; then
		echo "  FAIL ${name}: refused without saying ${needle}"
		sed 's/^/       /' "$out"
		failures=$((failures + 1))
		return
	fi
	# The half that matters as much as the exit code. A refusal that still left
	# a directory behind is a refusal scripts/package.sh would stage over.
	if [[ -e "$DEST" ]]; then
		echo "  FAIL ${name}: refused but left ${DEST} behind:"
		find "$DEST" | sed 's/^/       /'
		failures=$((failures + 1))
		return
	fi
	echo "  ok   ${name}"
}

corrupt() { # corrupt FILE — same length, different bytes, so only a digest sees it
	python3 -c '
import sys
p = sys.argv[1]
b = bytearray(open(p, "rb").read())
b[0] ^= 0xFF
open(p, "wb").write(bytes(b))
' "$1"
}

echo "== a complete release and registry"
expect_pass "the release assets and the registry files assemble a bundle"
if "$CHECK" "$DEST" "$PLATFORM" "$TAG" >"$TMP/checked" 2>&1; then
	echo "  ok   the assembled bundle is one check-bundled-extension.sh accepts"
else
	echo "  FAIL the assembled bundle was refused by check-bundled-extension.sh:"
	sed 's/^/       /' "$TMP/checked"
	failures=$((failures + 1))
fi
# The value-embedding directory keeps the name the model's own config gives it.
# Renaming it to something a check hardcodes is the defect this pins.
if [[ -f "$DEST/model/$EMBED/tokenizer.json" ]]; then
	echo "  ok   the value-embedding directory keeps the name config.json gives it"
else
	echo "  FAIL model/${EMBED}/tokenizer.json is not in the assembled bundle:"
	find "$DEST/model" | sed 's/^/       /'
	failures=$((failures + 1))
fi

echo "== release bytes that are not the ones published"
make_fixture
head -c 200 "$RELEASE_DIR/$EXT_ASSET" >"$TMP/short" && mv "$TMP/short" "$RELEASE_DIR/$EXT_ASSET"
expect_fail "a truncated extension" "does not match its published checksum"

make_fixture
corrupt "$RELEASE_DIR/$CATALOGUE_ASSET"
expect_fail "a substituted taxonomy catalogue" "does not match its published checksum"

make_fixture
corrupt "$RELEASE_DIR/$METADATA_ASSET"
expect_fail "substituted release metadata" "does not match its published checksum"

echo "== a release asset with no usable checksum beside it"
make_fixture
rm "$RELEASE_DIR/$EXT_ASSET.sha256"
expect_fail "no checksum beside the extension" "published no checksum beside it"

make_fixture
printf 'not a digest\n' >"$RELEASE_DIR/$CATALOGUE_ASSET.sha256"
expect_fail "a sidecar that is not a sha256" "did not parse to a sha256"

echo "== a release asset that is not published at all"
make_fixture
rm "$RELEASE_DIR/$CATALOGUE_ASSET" "$RELEASE_DIR/$CATALOGUE_ASSET.sha256"
expect_fail "no taxonomy catalogue in the release" "could not fetch"

make_fixture
rm "$RELEASE_DIR/$EXT_ASSET" "$RELEASE_DIR/$EXT_ASSET.sha256"
expect_fail "no extension for this target" "could not fetch"

echo "== release metadata that does not describe this release"
# The cross-check the extension's trailer cannot make: it says the RELEASE the
# assets came from is the one the pin names, however any single asset is
# stamped. Its sidecar is rewritten, so this is a well-formed asset saying the
# wrong thing rather than a corrupted one.
make_fixture
python3 - "$RELEASE_DIR/$METADATA_ASSET" <<'PY'
import json, sys
p = sys.argv[1]
meta = json.load(open(p))
meta["tag"] = "v9.9.9"
json.dump(meta, open(p, "w"))
PY
sidecar "$METADATA_ASSET"
expect_fail "metadata from another tag entirely" "says it was built for 'v9.9.9'"

make_fixture
printf '{"tag": "%s"}\n' "$TAG" >"$RELEASE_DIR/$METADATA_ASSET"
sidecar "$METADATA_ASSET"
expect_fail "metadata naming no model" "carries no 'tag' and 'model' pair"

make_fixture
printf '{"tag": "%s", "model": "../escape"}\n' "$TAG" >"$RELEASE_DIR/$METADATA_ASSET"
sidecar "$METADATA_ASSET"
expect_fail "a model name that is a path" "which is not a directory name"

make_fixture
printf 'not json at all\n' >"$RELEASE_DIR/$METADATA_ASSET"
sidecar "$METADATA_ASSET"
expect_fail "metadata that is not JSON" "did not parse"

echo "== model bytes that are not the ones the revision names"
# The registry listing is left describing the original bytes, so only the
# per-file comparison can see this. The two paths differ: the safetensors are
# declared by sha256 as LFS objects, the JSON by git blob id.
make_fixture
corrupt "$FILES_DIR/model.safetensors"
expect_fail "a substituted model weight (declared by sha256)" "does not match the registry's sha256"

make_fixture
corrupt "$FILES_DIR/label_map.json"
expect_fail "a substituted label map (declared by git blob id)" "does not match the registry's gitblob"

make_fixture
corrupt "$FILES_DIR/$EMBED/model.safetensors"
expect_fail "a substituted value-embedding weight" "does not match the registry's sha256"

echo "== a registry that does not carry this model at this revision"
make_fixture
rm -rf "$TREE_DIR"
expect_fail "no listing for the model at the pinned revision" "the registry listed no files"

make_fixture
rm "$FILES_DIR/$EMBED/tokenizer.json"
expect_fail "a file the listing names and the registry does not serve" "could not fetch"

make_fixture
printf '[]' >"$TREE_DIR/$MODEL"
expect_fail "an empty listing" "the registry listed no files"

echo "== a model whose config does not name its value embeddings"
make_fixture
printf '{"n_classes": 1}\n' >"$FILES_DIR/config.json"
write_tree
expect_fail "config.json with no value_embed_model" "declares no value_embed_model"

make_fixture
printf '{"value_embed_model": "../elsewhere"}\n' >"$FILES_DIR/config.json"
write_tree
expect_fail "value_embed_model pointing out of the model directory" \
	"which is a path rather than a directory name"

echo "== a destination left over from an earlier run"
make_fixture
rm -rf "$DEST" && mkdir -p "$DEST"
printf 'a bundle nobody downloaded\n' >"$DEST/finetype.duckdb_extension"
rm "$RELEASE_DIR/$EXT_ASSET"
if "$FETCH" "$TARGET" "$DEST" >"$out" 2>&1; then
	echo "  FAIL a failed fetch over a stale bundle reported success"
	failures=$((failures + 1))
elif [[ -e "$DEST" ]]; then
	echo "  FAIL a failed fetch left the stale bundle where packaging would stage it:"
	find "$DEST" | sed 's/^/       /'
	failures=$((failures + 1))
else
	echo "  ok   a failed fetch clears a stale bundle rather than leaving it to be staged"
fi

echo
if [[ "$failures" -ne 0 ]]; then
	echo "fetch-finetype-bundle-selftest: ${failures} case(s) did not behave as required." >&2
	exit 1
fi
echo "fetch-finetype-bundle-selftest: the fetch assembles what it should and refuses what it should."
