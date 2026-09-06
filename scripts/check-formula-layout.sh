#!/usr/bin/env bash
# The tap formula installs the tarball's tree, and reaches the binary by an
# exec script rather than a symlink.
#
#   scripts/check-formula-layout.sh FORMULA.rb
#
# scripts/check-formula-asset.sh proves the urls resolve and the checksums
# match — what `brew install` establishes before it unpacks anything. This
# proves what happens after it unpacks: that the tree the tarball carries
# survives the install, and that the installed binary can find it.
#
# TWO WAYS TO GET THIS WRONG, and both produce a formula that installs
# successfully.
#
#   `bin.install "brightfield"` takes the executable and discards everything
#   around it. The tarball's `finetype/` — the semantic type classifier — is
#   dropped on the way in, and the installed application reports the storage
#   type of every column. That is what the formula did, and it is why an
#   installed brightfield 0.1.4 has no finetype/ in its Cellar.
#
#   `bin.install_symlink libexec/"brightfield"` keeps the tree and still fails.
#   `std::env::current_exe` on macOS returns the path the process was invoked
#   with, unresolved — `_NSGetExecutablePath` does not follow the link — so the
#   binary reached through `bin/brightfield` reports `bin` as its own directory
#   and looks for `bin/finetype`. Measured against the installed 0.1.4: the
#   same binary with the same bundle beside it finds the bundle by its real
#   path and reports none through an identical symlink.
#
# THIS READS THE INSTALL BLOCK AND NOT A FILE LIST, because the second failure
# above installs every file in the right place. What is wrong with it is which
# path the process ends up running from, and no directory listing can see that.
set -euo pipefail

FORMULA="${1:?usage: scripts/check-formula-layout.sh FORMULA.rb}"

fail() { echo "check-formula-layout: $*" >&2; exit 1; }

[ -f "$FORMULA" ] || fail "no formula at ${FORMULA}"

# The install block alone. Reading the whole file would let a comment or the
# test block satisfy any of the requirements below, which is a mistake this
# repository has already made once in a workflow scan.
BLOCK=$(awk '
  /^[[:space:]]*def[[:space:]]+install[[:space:]]*$/ { inside = 1; next }
  inside && /^[[:space:]]*end[[:space:]]*$/          { inside = 0 }
  inside                                             { print }
' "$FORMULA" | { grep -vE '^[[:space:]]*#' || true; })
# `|| true` because grep exits 1 when every line was a comment, and under
# `set -o pipefail` that killed this script before it could say so — an install
# block of nothing but comments made the check exit 1 with no message, which
# reads as a refusal and explains nothing.

[ -n "$(printf '%s' "$BLOCK" | tr -d '[:space:]')" ] \
  || fail "${FORMULA} has no 'def install' block, or it holds nothing but comments"

# The two known-wrong shapes first, so the refusal names the specific mistake
# rather than the general requirement it happens to break.
if printf '%s\n' "$BLOCK" | grep -q 'bin.install_symlink'; then
  fail "${FORMULA}'s install block symlinks the binary into bin/. current_exe() on macOS
  returns the path the process was invoked with, so the binary reports bin/ as its own
  directory and looks for bin/finetype. Use bin.write_exec_script."
fi

if printf '%s\n' "$BLOCK" | grep -Eq 'bin\.install[[:space:]]+"brightfield"'; then
  fail "${FORMULA}'s install block installs the binary alone, which discards the
  finetype/ directory the tarball carries beside it."
fi

printf '%s\n' "$BLOCK" | grep -q 'libexec.install' \
  || fail "${FORMULA}'s install block does not put the tarball's tree in libexec, so
  finetype/ is discarded and the installed application has no type source beside it"

printf '%s\n' "$BLOCK" | grep -q 'bin.write_exec_script' \
  || fail "${FORMULA}'s install block does not reach the binary through an exec script.
  A symlink in bin/ is not equivalent: current_exe() on macOS does not resolve one,
  so the binary looks for its bundle in bin/ and finds nothing."

echo "check-formula-layout: ${FORMULA} installs the tree into libexec and execs it from bin."
