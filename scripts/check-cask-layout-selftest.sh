#!/usr/bin/env bash
# Prove scripts/check-cask-layout.sh reddens, and that the cask this repository
# generates is one it accepts.
#
# The cask is regenerated wholesale by .github/workflows/release.yml on every
# tag, from scripts/write-brightfield-cask.sh. That means a correct cask
# hand-written into the tap is REVERTED by the next release, silently, and the
# tap has no CI of its own to notice. So the shape has to be asserted here, on
# the generator, on every pull request.
#
# Both directions. The generated cask must pass, since a check that refused it
# would be reverted by the first person who needed a release, and each way to
# get the cask wrong must fail: every one of them produces a cask that installs
# and a command that cannot find its type source, or an upgrade that fails
# without naming why.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECK="$HERE/check-cask-layout.sh"
WRITE="$HERE/write-brightfield-cask.sh"

failures=0
TMP="$(mktemp -d)" || exit 1
trap 'rm -rf "$TMP"' EXIT
out="$TMP/out"

expect_pass() {
	local name="$1" file="$2"
	if "$CHECK" "$file" >"$out" 2>&1; then
		echo "  ok   ${name}"
	else
		echo "  FAIL ${name}: expected a pass, got a refusal"
		sed 's/^/       /' "$out"
		failures=$((failures + 1))
	fi
}

expect_fail() {
	local name="$1" needle="$2" file="$3"
	if "$CHECK" "$file" >"$out" 2>&1; then
		echo "  FAIL ${name}: expected a refusal, got a pass"
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
	echo "  ok   ${name}"
}

# A generator run that must exit 2, whatever it prints.
expect_usage_error() {
	local name="$1"
	shift
	"$WRITE" "$@" >"$out" 2>&1
	local rc=$?
	if [[ "$rc" -eq 2 ]]; then
		echo "  ok   ${name}"
	else
		echo "  FAIL ${name}: expected exit 2, got ${rc}"
		sed 's/^/       /' "$out"
		failures=$((failures + 1))
	fi
}

# A digest of the right shape, derived rather than pasted.
SHA="$(printf 'fixture' | { shasum -a 256 2>/dev/null || sha256sum; } | awk '{print $1}')"
BASE="https://example.invalid/download/v0.1.8"

echo "== the cask this repository generates"
"$WRITE" v0.1.8 "$BASE" "$SHA" "$SHA" >"$TMP/generated.rb" || {
	echo "  FAIL the generator did not run"
	failures=$((failures + 1))
}
expect_pass "the generated cask installs the app and execs it by absolute path" "$TMP/generated.rb"

# The values the generator is given reach the cask, and the parts that are not
# the shell's reach it untouched.
want() { # want NAME NEEDLE
	if grep -qF -- "$2" "$TMP/generated.rb"; then
		echo "  ok   $1"
	else
		echo "  FAIL $1: the cask does not contain: $2"
		failures=$((failures + 1))
	fi
}
want "the version is the tag without its v" 'version "0.1.8"'
want "the url drops the tag from the base and keeps Ruby's interpolation" \
	'url "https://example.invalid/download/v#{version}/brightfield-v#{version}-#{arch}-apple-darwin.dmg"'
want "the shell's \$@ is literal in the shim" 'Contents/MacOS/brightfield'"'"' "$@"'
want "the shell's \$link is literal in the preflight" 'link="${HOMEBREW_PREFIX:-/opt/homebrew}/bin/brightfield"'
want "{{appdir}} is literal in the shim" "exec '{{appdir}}/Brightfield.app"

# Ruby accepts it, when there is a Ruby to ask.
if command -v ruby >/dev/null 2>&1; then
	if ruby -c "$TMP/generated.rb" >"$out" 2>&1; then
		echo "  ok   ruby -c accepts the generated cask"
	else
		echo "  FAIL ruby -c refused the generated cask"
		sed 's/^/       /' "$out"
		failures=$((failures + 1))
	fi
else
	echo "  skip ruby -c: ruby is not installed"
fi

echo "== the generator refuses bad usage"
expect_usage_error "a checksum that is too short" v0.1.8 "$BASE" "$SHA" "abc123"
expect_usage_error "a checksum that is not a digest" v0.1.8 "$BASE" "$SHA" "not-a-digest"
expect_usage_error "a tag without its v" 0.1.8 "https://example.invalid/download/0.1.8" "$SHA" "$SHA"
expect_usage_error "a base url that does not end in the tag" v0.1.8 "https://example.invalid/download/v0.1.7" "$SHA" "$SHA"
expect_usage_error "too few arguments" v0.1.8 "$BASE" "$SHA"

# ---------------------------------------------------------------------------
# The wrong casks, each made by one edit to the real generated cask so nothing
# else about it differs and a case fails for the reason it names.
# ---------------------------------------------------------------------------
echo "== the ways to get the cask wrong"
sed '/^  app "Brightfield.app"$/d' "$TMP/generated.rb" >"$TMP/noapp.rb"
expect_fail "no app stanza" 'has no '"'"'app "Brightfield.app"'"'"' stanza' "$TMP/noapp.rb"

# The trap: every file lands in the right place and the command still cannot
# find its bundle, because current_exe() on macOS does not resolve the link.
sed 's|^  binary "brightfield.wrapper.sh".*|  binary "#{appdir}/Brightfield.app/Contents/MacOS/brightfield"|' \
	"$TMP/generated.rb" >"$TMP/symlink.rb"
expect_fail "the app's executable linked into bin" \
	"links the app's executable into bin/" "$TMP/symlink.rb"

sed "s|exec '{{appdir}}/Brightfield.app/Contents/MacOS/brightfield'|exec 'Brightfield.app/Contents/MacOS/brightfield'|" \
	"$TMP/generated.rb" >"$TMP/relative.rb"
expect_fail "a shim that execs a relative path" \
	"execs something other than the absolute path" "$TMP/relative.rb"

sed "s|exec '{{appdir}}/Brightfield.app/Contents/MacOS/brightfield'|exec brightfield|" \
	"$TMP/generated.rb" >"$TMP/bare.rb"
expect_fail "a shim that execs the bare name on PATH" \
	"execs something other than the absolute path" "$TMP/bare.rb"

# The exec named in a comment and not written: the shape that beat the
# release-path scan in scripts/check-finetype-pin.sh, checked here before it
# can happen again.
sed "s|^      exec '{{appdir}}.*|      # exec '{{appdir}}/Brightfield.app/Contents/MacOS/brightfield' \"\$@\"|" \
	"$TMP/generated.rb" >"$TMP/commented.rb"
expect_fail "the exec named in a comment and never run" \
	"writes no shim that execs" "$TMP/commented.rb"

sed '/brew uninstall --formula brightfield/d' "$TMP/generated.rb" >"$TMP/noformula.rb"
expect_fail "no refusal of an install over the old formula" \
	"does not refuse an install over the old formula" "$TMP/noformula.rb"

sed 's|^  depends_on :macos$|  depends_on :macos\n\n  preflight do\n  end|' "$TMP/generated.rb" >"$TMP/preflightdo.rb"
expect_fail "a preflight do block" "uses a 'preflight do' block" "$TMP/preflightdo.rb"

sed 's|#{arch}|aarch64|' "$TMP/generated.rb" >"$TMP/noarch.rb"
expect_fail "a url without #{arch}" "does not contain #{arch}" "$TMP/noarch.rb"

# Present in the cask but outside preflight_steps, which Homebrew does not run.
sed '/brew uninstall --formula brightfield/d' "$TMP/generated.rb" >"$TMP/outside.rb"
{ cat "$TMP/outside.rb"; echo '# brew uninstall --formula brightfield'; echo 'brew uninstall --formula brightfield'; } >"$TMP/outside2.rb"
expect_fail "the uninstall text outside the preflight_steps block" \
	"does not refuse an install over the old formula" "$TMP/outside2.rb"

sed '/\*\/Cellar\/brightfield\//d' "$TMP/generated.rb" >"$TMP/nocellar.rb"
expect_fail "a preflight that never tests the Cellar" "does not test whether" "$TMP/nocellar.rb"

sed 's|^    write_file "brightfield.wrapper.sh"|    write_file "other.sh"|' "$TMP/generated.rb" >"$TMP/nowrite.rb"
expect_fail "a preflight that does not write the shim" "does not write_file the shim" "$TMP/nowrite.rb"

sed '/set_permissions/d' "$TMP/generated.rb" >"$TMP/nochmod.rb"
expect_fail "a preflight that does not set the shim executable" "does not set_permissions" "$TMP/nochmod.rb"

sed 's|/v#{version}/brightfield-v#{version}|/latest/brightfield|' "$TMP/generated.rb" >"$TMP/noversion.rb"
expect_fail "a url without v#{version}" "does not contain v#{version}" "$TMP/noversion.rb"

echo "== a cask that is not there"
"$CHECK" "$TMP/absent.rb" >"$out" 2>&1
rc=$?
if [[ "$rc" -eq 2 ]] && grep -qF "no cask at" "$out"; then
	echo "  ok   a path that does not resolve exits 2"
else
	echo "  FAIL a path that does not resolve: expected exit 2 naming the cask, got ${rc}"
	sed 's/^/       /' "$out"
	failures=$((failures + 1))
fi

echo
if [[ "$failures" -ne 0 ]]; then
	echo "check-cask-layout-selftest: ${failures} case(s) did not behave as required." >&2
	exit 1
fi
echo "check-cask-layout-selftest: the generated cask installs the app and execs it by absolute path, and the wrong shapes are refused."
