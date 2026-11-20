#!/usr/bin/env bash
# Regression test for scripts/check-cask-asset.sh — the check's credibility.
#
# The check itself can only run on a tag: it needs a published release and a
# rewritten tap cask to address. That is exactly the shape that rots, because
# nothing exercises it between releases and a release is the worst moment to
# discover the check stopped reddening. So this file stands a throwaway HTTP
# server in front of throwaway assets, writes casks against it, and asserts
# on exit codes and on what the messages say — on a pull request, where a
# regression is cheap.
#
# Both directions are covered, like the other gate self-tests in this directory:
#
#   1. the real url/checksum pairs pass, and the output names each url it
#      fetched, so a check that silently examined nothing cannot look green;
#   2. a wrong checksum on the second platform block fails, and the message
#      names the url, the declared digest and the actual one;
#   3. a wrong checksum on the FIRST block fails too — the check does not stop
#      at the first pair, and it does not skip to the last;
#   4. a url that 404s fails, and the message says 404;
#   5. a checksum that is not a sha256 fails;
#   6. a cask with no pairs, a version or url that is missing, and an
#      architecture with a name but no checksum are each hard failures rather
#      than vacuous passes;
#   7. --min-pairs refuses a cask that lost an architecture;
#   8. a url template that lost `#{arch}` expands to one url for both
#      architectures and is refused, since it would fetch the arm image twice
#      and call the Intel one checked;
#   9. the url inside `livecheck do ... end` is not mistaken for the download.
#
# Usage:  ./scripts/check-cask-asset-selftest.sh
# Exit:   0 all assertions passed, 1 otherwise.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECK="$HERE/check-cask-asset.sh"

[[ -x "$CHECK" ]] || {
	echo "selftest: $CHECK is missing or not executable" >&2
	exit 1
}

# python3 serves the fixture assets. It is present on the hosted runner this
# workflow uses and on a developer macOS box; if it ever is not, this must be a
# loud failure rather than a skip. A self-test that skips is a self-test that
# reports green while proving nothing.
command -v python3 >/dev/null 2>&1 || {
	echo "selftest: python3 is required to serve the fixture assets" >&2
	exit 1
}

failures=0
ok() { printf '  ok    %s\n' "$1"; }
bad() {
	printf '  FAIL  %s\n' "$1"
	failures=$((failures + 1))
}

TMP="$(mktemp -d)" || exit 1
SERVER_PID=""
cleanup() {
	if [[ -n "$SERVER_PID" ]]; then
		# `wait` after the kill so the shell reaps the job here rather than
		# printing "Terminated" over the last line of the report.
		kill "$SERVER_PID" 2>/dev/null
		wait "$SERVER_PID" 2>/dev/null
	fi
	rm -rf "$TMP"
}
trap cleanup EXIT

# ---------------------------------------------------------------------------
# Fixture assets and a server in front of them.
#
# The bytes are arbitrary but fixed, so the digests below are computed from the
# files rather than pasted — a pasted digest would make case 1 pass for the
# wrong reason the moment the fixture changed.
# ---------------------------------------------------------------------------
# Laid out the way the release lays them out: a directory per tag, and the
# architecture in the file name. v0.0.2 holds the arm image only, which is the
# "intel never got published" case.
mkdir -p "$TMP/assets/v0.0.1" "$TMP/assets/v0.0.2"
ARM_FILE="$TMP/assets/v0.0.1/brightfield-v0.0.1-aarch64-apple-darwin.dmg"
INTEL_FILE="$TMP/assets/v0.0.1/brightfield-v0.0.1-x86_64-apple-darwin.dmg"
printf 'fixture asset, arm leg\n' >"$ARM_FILE"
printf 'fixture asset, intel leg, a different size on purpose\n' >"$INTEL_FILE"
cp "$ARM_FILE" "$TMP/assets/v0.0.2/brightfield-v0.0.2-aarch64-apple-darwin.dmg"

digest_of() {
	local out
	if command -v shasum >/dev/null 2>&1; then
		out="$(shasum -a 256 "$1")" || return 1
	else
		out="$(sha256sum "$1")" || return 1
	fi
	printf '%s' "${out%% *}"
}

ARM_SHA="$(digest_of "$ARM_FILE")" || {
	echo "selftest: could not hash the fixture asset" >&2
	exit 1
}
INTEL_SHA="$(digest_of "$INTEL_FILE")" || {
	echo "selftest: could not hash the fixture asset" >&2
	exit 1
}

# A digest of the right shape that belongs to no file here: the last hex digit
# rotated. This is the wrong-digest fixture, and it is derived rather than
# written so it can never accidentally become the right one.
last="${INTEL_SHA: -1}"
case "$last" in
0) rotated=1 ;;
*) rotated=0 ;;
esac
WRONG_INTEL_SHA="${INTEL_SHA:0:63}$rotated"

last="${ARM_SHA: -1}"
case "$last" in
0) rotated=1 ;;
*) rotated=0 ;;
esac
WRONG_ARM_SHA="${ARM_SHA:0:63}$rotated"

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

python3 "$TMP/serve.py" "$TMP/assets" >"$TMP/port" 2>"$TMP/server.err" &
SERVER_PID=$!

PORT=""
waited=0
while [[ $waited -lt 100 ]]; do
	PORT="$(tr -d '[:space:]' <"$TMP/port")"
	[[ -n "$PORT" ]] && break
	sleep 0.1
	waited=$((waited + 1))
done

if [[ -z "$PORT" ]]; then
	echo "selftest: the fixture server never reported a port" >&2
	cat "$TMP/server.err" >&2
	exit 1
fi

BASE="http://127.0.0.1:$PORT"
ARM_URL="$BASE/v0.0.1/brightfield-v0.0.1-aarch64-apple-darwin.dmg"
INTEL_URL="$BASE/v0.0.1/brightfield-v0.0.1-x86_64-apple-darwin.dmg"
MISSING_URL="$BASE/v0.0.2/brightfield-v0.0.2-x86_64-apple-darwin.dmg"

# Prove the server is really answering before any assertion depends on it —
# otherwise every "goes red" case below would pass for the wrong reason.
if ! curl -sSfL -o /dev/null "$ARM_URL"; then
	echo "selftest: the fixture server did not serve the arm asset" >&2
	cat "$TMP/server.err" >&2
	exit 1
fi

# ---------------------------------------------------------------------------
# Cask fixtures. The shape is the one scripts/write-brightfield-cask.sh writes:
# an arch pair, a two-line sha256, one templated url, and a livecheck block
# carrying a url of its own that must not be read as the download.
# ---------------------------------------------------------------------------
cask() { # cask <path> <version> <arm-sha> <intel-sha> [url-template]
	# The default sits in a variable because a `}` inside ${5:-...} would end
	# the expansion at the first `#{...}` the template contains.
	local default="$BASE/v#{version}/brightfield-v#{version}-#{arch}-apple-darwin.dmg"
	local tmpl="${5:-$default}"
	cat >"$1" <<RB
cask "brightfield" do
  arch arm: "aarch64", intel: "x86_64"

  version "$2"
  sha256 arm:   "$3",
         intel: "$4"

  url "$tmpl"
  name "Brightfield"
  homepage "https://example.invalid/"

  livecheck do
    url "https://example.invalid/releases.atom"
    strategy :page_match
    regex(%r{/releases/tag/v?(\d+(?:\.\d+)+)["<]}i)
  end

  depends_on :macos

  app "Brightfield.app"
end
RB
}

# run <expected-exit> <description> -- <args...>  ; leaves output in $OUT
OUT=""
run() {
	local want="$1" desc="$2"
	shift 3 # want, desc, and the literal --
	OUT="$("$CHECK" "$@" 2>&1)"
	rc=$?
	if [[ $rc -ne $want ]]; then
		bad "$desc: expected exit $want, got $rc"
		printf '%s\n' "$OUT" | sed 's/^/          /'
		return 1
	fi
	ok "$desc (exit $rc)"
	return 0
}

says() { # says <description> <needle>
	if printf '%s' "$OUT" | grep -Fq -- "$2"; then
		ok "$1"
	else
		bad "$1 — output did not contain: $2"
		printf '%s\n' "$OUT" | sed 's/^/          /'
	fi
}

echo "the real pairs pass, and say what they fetched:"
cask "$TMP/good.rb" 0.0.1 "$ARM_SHA" "$INTEL_SHA"
if run 0 "both architectures fetch and match" -- --min-pairs 2 "$TMP/good.rb"; then
	says "the output names the arm url" "$ARM_URL"
	says "the output names the intel url" "$INTEL_URL"
	says "the output names the arm digest" "$ARM_SHA"
	says "the output names the intel digest" "$INTEL_SHA"
	if printf '%s' "$OUT" | grep -Fq "releases.atom"; then
		bad "the livecheck url was fetched as if it were the download"
	else
		ok "the livecheck url is not the download"
	fi
fi

echo
echo "a wrong checksum goes red, and the message is usable:"
cask "$TMP/wrong-intel.rb" 0.0.1 "$ARM_SHA" "$WRONG_INTEL_SHA"
if run 1 "the intel checksum disagrees with the asset" -- --min-pairs 2 "$TMP/wrong-intel.rb"; then
	says "the message names the url" "$INTEL_URL"
	says "the message names the declared digest" "$WRONG_INTEL_SHA"
	says "the message names the actual digest" "$INTEL_SHA"
fi

# The arm digest, not the intel one: a check that stopped at the first pair
# would pass the case above and fail this one, and a check that only ever
# looked at the last pair would do the reverse.
cask "$TMP/wrong-arm.rb" 0.0.1 "$WRONG_ARM_SHA" "$INTEL_SHA"
if run 1 "the arm checksum disagrees with the asset" -- --min-pairs 2 "$TMP/wrong-arm.rb"; then
	says "the message names the arm url" "$ARM_URL"
	says "the message names the declared digest" "$WRONG_ARM_SHA"
	says "the message names the actual digest" "$ARM_SHA"
fi

echo
echo "a url nobody can download goes red:"
cask "$TMP/missing.rb" 0.0.2 "$ARM_SHA" "$INTEL_SHA"
if run 1 "the intel image was never published" -- --min-pairs 2 "$TMP/missing.rb"; then
	says "the message names the url" "$MISSING_URL"
	says "the message names the status" "HTTP 404"
fi

echo
echo "a checksum that is not a sha256 goes red:"
cask "$TMP/notasha.rb" 0.0.1 "$ARM_SHA" "deadbeef"
run 1 "a truncated checksum is refused" -- --min-pairs 2 "$TMP/notasha.rb"

echo
echo "a url that lost #{arch} is refused, not checked twice:"
cask "$TMP/noarch.rb" 0.0.1 "$ARM_SHA" "$INTEL_SHA" \
	"$BASE/v#{version}/brightfield-v#{version}-aarch64-apple-darwin.dmg"
if run 2 "both architectures expand to the arm image" -- --min-pairs 2 "$TMP/noarch.rb"; then
	says "the message says the url is the same" "expand to the same url"
fi
run 2 "the same refusal with no --min-pairs" -- "$TMP/noarch.rb"

echo
echo "nothing to check is a hard failure, never a pass:"
cat >"$TMP/empty.rb" <<'RB'
cask "brightfield" do
  version "0.0.1"
  url "https://example.invalid/brightfield.dmg"
end
RB
run 2 "a cask with no architectures and no checksums" -- "$TMP/empty.rb"

cat >"$TMP/noversion.rb" <<RB
cask "brightfield" do
  arch arm: "aarch64", intel: "x86_64"
  sha256 arm: "$ARM_SHA", intel: "$INTEL_SHA"
  url "$BASE/v#{version}/brightfield-v#{version}-#{arch}-apple-darwin.dmg"
end
RB
run 2 "a cask with no version to expand" -- --min-pairs 2 "$TMP/noversion.rb"

cat >"$TMP/nourl.rb" <<RB
cask "brightfield" do
  arch arm: "aarch64", intel: "x86_64"
  version "0.0.1"
  sha256 arm: "$ARM_SHA", intel: "$INTEL_SHA"
  livecheck do
    url "https://example.invalid/releases.atom"
  end
end
RB
run 2 "a cask whose only url is the livecheck one" -- --min-pairs 2 "$TMP/nourl.rb"

cat >"$TMP/nointelsha.rb" <<RB
cask "brightfield" do
  arch arm: "aarch64", intel: "x86_64"
  version "0.0.1"
  sha256 arm: "$ARM_SHA"
  url "$BASE/v#{version}/brightfield-v#{version}-#{arch}-apple-darwin.dmg"
end
RB
run 2 "an architecture named with no checksum" -- --min-pairs 2 "$TMP/nointelsha.rb"

cat >"$TMP/onepair.rb" <<RB
cask "brightfield" do
  arch arm: "aarch64"
  version "0.0.1"
  sha256 arm: "$ARM_SHA"
  url "$BASE/v#{version}/brightfield-v#{version}-#{arch}-apple-darwin.dmg"
end
RB
run 2 "one architecture where two were required" -- --min-pairs 2 "$TMP/onepair.rb"
run 0 "the same one architecture, when one is all that was required" -- "$TMP/onepair.rb"

run 2 "a cask path that does not exist" -- "$TMP/definitely-not-here.rb"

echo
if [[ $failures -ne 0 ]]; then
	echo "check-cask-asset self-test: $failures assertion(s) FAILED" >&2
	exit 1
fi
echo "check-cask-asset self-test: ok"
