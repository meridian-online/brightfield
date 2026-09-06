#!/usr/bin/env bash
# Prove scripts/fetch-finetype-bundle.sh refuses bytes that are not the ones
# FineType published.
#
# The script it exercises runs on a tag and nowhere else, which is the shape
# that rots: nothing touches it between releases and a release is the worst
# moment to discover the checksum comparison stopped comparing. So this stands
# a throwaway HTTP server in front of throwaway assets, points
# BRIGHTFIELD_FINETYPE_ASSET_BASE at it, and runs the real file — url
# construction, curl, sidecar parse, digest comparison, unpack, assembly. Only
# the host differs from a release.
#
# WHAT IT STILL DOES NOT REACH, said plainly because a green run here is easy
# to over-read: a real FineType release. At the time this was written a
# FineType tag published CLI tarballs and their checksums and nothing else, so
# the asset NAMES the fetch script builds have never resolved against
# github.com. Everything downstream of the name is exercised here; the name
# itself is confirmed by the first release that carries the assets.
#
# Both directions, like the other gate self-tests here. A good asset set must
# assemble a bundle that scripts/check-bundled-extension.sh accepts — a fetch
# that refused everything would be reverted by the first person who needed it —
# and each way an asset can be wrong must fail, leaving no bundle behind.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FETCH="$HERE/fetch-finetype-bundle.sh"
CHECK="$HERE/check-bundled-extension.sh"

TAG="$("$HERE/finetype-pin.sh")" || {
	echo "selftest: the pin does not read" >&2
	exit 1
}

# The target whose asset names these fixtures use. Any triple in
# scripts/duckdb-platform.sh would do; this one is the release matrix's native
# leg, so the platform stamp below is the one a packaging run would demand.
TARGET="aarch64-apple-darwin"
PLATFORM="$("$HERE/duckdb-platform.sh" "$TARGET")"

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

ASSETS="$TMP/assets"
DEST="$TMP/bundle"
out="$TMP/out"

EXT_ASSET="finetype-${TAG}-${TARGET}.duckdb_extension"
CATALOGUE_ASSET="finetype-${TAG}-taxonomy-schemas.json"
MODEL_ASSET="finetype-${TAG}-model.tar.gz"

digest_of() {
	if command -v shasum >/dev/null 2>&1; then
		shasum -a 256 "$1" | awk '{print $1}'
	else
		sha256sum "$1" | awk '{print $1}'
	fi
}

# sidecar ASSET — write the checksum file beside it, in the form the existing
# FineType CLI sidecars use (`shasum -a 256` output: digest, two spaces, name).
sidecar() {
	printf '%s  %s\n' "$(digest_of "$ASSETS/$1")" "$1" >"$ASSETS/$1.sha256"
}

# ---------------------------------------------------------------------------
# A complete, correct asset set. Rebuilt before every case so a mutation in one
# case cannot leak into the next and make it pass or fail for the wrong reason.
#
# MODEL_LAYOUT decides whether the model tarball carries a top directory. Both
# shapes are real — `tar -czf … model` produces one and `tar -czf … -C model .`
# does not — and the fetch script normalises them, so both are exercised.
# ---------------------------------------------------------------------------
make_assets() { # make_assets [nested|flat]
	local layout="${1:-flat}"
	# Emptied rather than replaced: the fixture server's working directory is
	# this path, and removing the directory leaves it serving a deleted inode
	# and answering every request with a connection reset — which reads in the
	# report below as "the fetch refused" for every case at once.
	mkdir -p "$ASSETS"
	find "$ASSETS" -mindepth 1 -delete
	rm -rf "$TMP/model"
	mkdir -p "$TMP/model/model2vec"

	"$HERE/fixture-extension.py" "$ASSETS/$EXT_ASSET" "$PLATFORM" v1.2.0 "${TAG#v}" C_STRUCT
	printf '[]\n' >"$ASSETS/$CATALOGUE_ASSET"

	printf 'fixture weights\n' >"$TMP/model/model.safetensors"
	printf '{"value_embed_model": "fixture-model-name"}\n' >"$TMP/model/config.json"
	printf '{}\n' >"$TMP/model/label_map.json"
	printf 'fixture m2v weights\n' >"$TMP/model/model2vec/model.safetensors"
	printf '{}\n' >"$TMP/model/model2vec/tokenizer.json"

	if [[ "$layout" == "nested" ]]; then
		tar -czf "$ASSETS/$MODEL_ASSET" -C "$TMP" model
	else
		tar -czf "$ASSETS/$MODEL_ASSET" -C "$TMP/model" .
	fi

	sidecar "$EXT_ASSET"
	sidecar "$CATALOGUE_ASSET"
	sidecar "$MODEL_ASSET"
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

make_assets flat
python3 "$TMP/serve.py" "$ASSETS" >"$TMP/port" 2>"$TMP/server.err" &
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
export BRIGHTFIELD_FINETYPE_ASSET_BASE="http://127.0.0.1:$PORT"

# Prove the server is really answering before any assertion depends on it —
# otherwise every "goes red" case below would pass for the wrong reason.
curl -sSfL -o /dev/null "$BRIGHTFIELD_FINETYPE_ASSET_BASE/$EXT_ASSET" || {
	echo "selftest: the fixture server did not serve the extension asset" >&2
	cat "$TMP/server.err" >&2
	exit 1
}

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

echo "== a complete asset set"
expect_pass "the three assets and their checksums assemble a bundle"
if "$CHECK" "$DEST" "$PLATFORM" "$TAG" >"$TMP/checked" 2>&1; then
	echo "  ok   the assembled bundle is one check-bundled-extension.sh accepts"
else
	echo "  FAIL the assembled bundle was refused by check-bundled-extension.sh:"
	sed 's/^/       /' "$TMP/checked"
	failures=$((failures + 1))
fi
if grep -q 'fixture-model-name' "$out"; then
	echo "  ok   the run names the model it found in the bundle's own config"
else
	echo "  FAIL the run did not name the model:"
	sed 's/^/       /' "$out"
	failures=$((failures + 1))
fi

echo "== a model archive with a top-level directory"
make_assets nested
expect_pass "a nested model tarball is normalised to model/"
[[ -f "$DEST/model/model2vec/tokenizer.json" ]] || {
	echo "  FAIL the nested layout did not place model2vec/tokenizer.json"
	failures=$((failures + 1))
}

echo "== bytes that are not the ones published"
make_assets flat
# Truncated: the sidecar is left alone, so the digest is the publisher's and
# the bytes are short. This is the partial-download case.
head -c 200 "$ASSETS/$EXT_ASSET" >"$TMP/short" && mv "$TMP/short" "$ASSETS/$EXT_ASSET"
expect_fail "a truncated extension" "does not match its published checksum"

make_assets flat
# Substituted: same length, different bytes, so nothing about the size gives it
# away and only the digest can.
python3 -c '
import sys
p = sys.argv[1]
b = bytearray(open(p, "rb").read())
b[0] ^= 0xFF
open(p, "wb").write(bytes(b))
' "$ASSETS/$MODEL_ASSET"
expect_fail "a substituted model archive" "does not match its published checksum"

make_assets flat
python3 -c '
import sys
p = sys.argv[1]
b = bytearray(open(p, "rb").read())
b[0] ^= 0xFF
open(p, "wb").write(bytes(b))
' "$ASSETS/$CATALOGUE_ASSET"
expect_fail "a substituted taxonomy catalogue" "does not match its published checksum"

echo "== an asset that published no usable checksum"
make_assets flat
rm "$ASSETS/$EXT_ASSET.sha256"
expect_fail "no checksum beside the extension" "published no checksum beside it"

make_assets flat
printf 'not a digest\n' >"$ASSETS/$MODEL_ASSET.sha256"
expect_fail "a sidecar that is not a sha256" "did not parse to a sha256"

echo "== an asset that is not published at all"
make_assets flat
rm "$ASSETS/$CATALOGUE_ASSET"
expect_fail "no taxonomy catalogue in the release" "could not fetch"

make_assets flat
rm "$ASSETS/$EXT_ASSET"
expect_fail "no extension for this target" "could not fetch"

echo "== an archive that is not the model"
make_assets flat
rm -rf "$TMP/wrong" && mkdir -p "$TMP/wrong"
printf 'a readme, not weights\n' >"$TMP/wrong/README.md"
tar -czf "$ASSETS/$MODEL_ASSET" -C "$TMP/wrong" .
sidecar "$MODEL_ASSET"
expect_fail "a model archive with no weights in it" "carries no model.safetensors"

echo "== a destination left over from an earlier run"
make_assets flat
rm -rf "$DEST" && mkdir -p "$DEST"
printf 'a bundle nobody downloaded\n' >"$DEST/finetype.duckdb_extension"
rm "$ASSETS/$EXT_ASSET"
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
