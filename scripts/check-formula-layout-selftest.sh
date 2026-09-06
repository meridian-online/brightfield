#!/usr/bin/env bash
# Prove scripts/check-formula-layout.sh reddens, and that the formula this
# repository generates is one it accepts.
#
# The formula is regenerated wholesale by .github/workflows/release.yml on
# every tag, from scripts/write-brightfield-formula.sh. That means a correct
# formula hand-written into the tap is REVERTED by the next release, silently,
# and the tap has no CI of its own to notice. So the shape has to be asserted
# here, on the generator, on every pull request.
#
# Both directions. The generated formula must pass — a check that refused it
# would be reverted by the first person who needed a release — and each of the
# two ways to get the install block wrong must fail, both of which produce a
# formula that installs successfully and an application that cannot find its
# type source.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECK="$HERE/check-formula-layout.sh"
WRITE="$HERE/write-brightfield-formula.sh"

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

# A digest of the right shape, derived rather than pasted.
SHA="$(printf 'fixture' | { shasum -a 256 2>/dev/null || sha256sum; } | awk '{print $1}')"

echo "== the formula this repository generates"
"$WRITE" v0.1.4 "https://example.invalid/download/v0.1.4" "$SHA" "$SHA" >"$TMP/generated.rb" || {
	echo "  FAIL the generator did not run"
	sed 's/^/       /' "$TMP/generated.rb"
	failures=$((failures + 1))
}
expect_pass "the generated formula installs the tree and execs it" "$TMP/generated.rb"

# The generator refusing a checksum that is not one. A truncated or empty
# digest otherwise reaches a stranger's `brew install` as a mismatch.
if "$WRITE" v0.1.4 "https://example.invalid/dl" "$SHA" "not-a-digest" >"$out" 2>&1; then
	echo "  FAIL the generator accepted a checksum that is not a sha256"
	failures=$((failures + 1))
else
	echo "  ok   the generator refuses a checksum that is not a sha256"
fi

# ---------------------------------------------------------------------------
# The two wrong install blocks, each substituted into the real generated
# formula so nothing else about it differs.
# ---------------------------------------------------------------------------
# Replaces the body of `def install` and leaves the rest of the generated
# formula exactly as it is, so a case fails for the reason it names rather than
# because the fixture differs somewhere else. python3 rather than awk: the
# bodies below are multi-line and `awk -v` cannot carry a newline in a variable.
swap_install() { # swap_install OUT BODY
	python3 - "$TMP/generated.rb" "$1" "$2" <<'PY'
import re, sys
src, out, body = sys.argv[1:4]
text = open(src).read()
new, n = re.subn(
    r"(?m)^(  def install\n)(?:.*\n)*?(  end\n)",
    lambda m: m.group(1) + body + "\n" + m.group(2),
    text,
    count=1,
)
if n != 1:
    sys.exit(f"swap_install: found {n} install blocks in {src}, expected exactly 1")
open(out, "w").write(new)
PY
}

echo "== the two ways to get the install block wrong"
swap_install "$TMP/binonly.rb" '    bin.install "brightfield"'
expect_fail "the binary alone, discarding the tree" \
	"installs the binary alone" "$TMP/binonly.rb"

# This one is the trap: every file lands in the right place and the
# application still cannot find its bundle, because current_exe() on macOS
# does not resolve the link.
swap_install "$TMP/symlink.rb" '    libexec.install Dir["*"]
    bin.install_symlink libexec/"brightfield"'
expect_fail "the tree in libexec reached by a symlink" \
	"symlinks the binary into bin/" "$TMP/symlink.rb"

echo "== an install block that is not there at all"
swap_install "$TMP/empty.rb" '    # nothing here but this comment'
expect_fail "an install block of comments" "holds nothing but comments" "$TMP/empty.rb"

# The exec script named in a comment and not called — the same shape that beat
# the release-path scan in scripts/check-finetype-pin.sh, checked here before
# it can happen again.
swap_install "$TMP/described.rb" '    # This used to call bin.write_exec_script libexec/"brightfield".
    libexec.install Dir["*"]'
expect_fail "the exec script named in a comment and never called" \
	"does not reach the binary through an exec script" "$TMP/described.rb"

echo "== a formula that is not there"
expect_fail "a path that does not resolve" "no formula at" "$TMP/absent.rb"

echo
if [[ "$failures" -ne 0 ]]; then
	echo "check-formula-layout-selftest: ${failures} case(s) did not behave as required." >&2
	exit 1
fi
echo "check-formula-layout-selftest: the generated formula installs the tree, and the two wrong shapes are refused."
