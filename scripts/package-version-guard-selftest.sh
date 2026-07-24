#!/usr/bin/env bash
# Regression test for scripts/package.sh's version-mismatch guard.
#
# The guard refuses to package a release whose tag argument does not carry the
# crate's own version, so a mislabelled artifact can't be built. Both directions
# are checked, like the other gate self-tests: a mismatching tag MUST fail loud,
# and a matching tag must NOT be flagged (no crying wolf).
#
# Neither direction runs a real build. The failure path exits at the guard,
# before rustc or cargo is touched. The pass path stubs rustc so the script
# stops right after the guard with no filesystem side effects — the point is
# only whether the guard fired, not what packaging does next.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
PKG="$PWD/scripts/package.sh"
CRATE_VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' crates/brightfield-shell/Cargo.toml | head -1)

# --- mismatching tags MUST fail loudly, at the guard, before any build --------
must_fail=(
  "v9.9.9"        # a plainly wrong release
  "9.9.9"         # compared on its core even without the leading v
  "v0.0.0"        # below the crate
  "v${CRATE_VERSION}.1" # a near-miss on the same crate version
)
for tag in "${must_fail[@]}"; do
  if out=$("$PKG" "$tag" 2>&1); then
    echo "SELF-TEST FAILED: guard stayed silent on mismatched tag '$tag'; output:" >&2
    printf '%s\n' "$out" >&2
    exit 1
  fi
  if ! printf '%s\n' "$out" | grep -q "version mismatch"; then
    echo "SELF-TEST FAILED: mismatched tag '$tag' failed without the guard message:" >&2
    printf '%s\n' "$out" >&2
    exit 1
  fi
  if printf '%s\n' "$out" | grep -q "== build"; then
    echo "SELF-TEST FAILED: mismatched tag '$tag' reached the build stage" >&2
    exit 1
  fi
done

# --- a matching tag must NOT be flagged --------------------------------------
# Stub rustc/cargo so the script halts right after the guard (the guard runs
# before `rustc -vV`), leaving no dist/ artifacts and running no compiler. The
# only thing asserted is that the guard did not fire.
stub=$(mktemp -d)
trap 'rm -rf "$stub"' EXIT
printf '#!/bin/sh\nexit 1\n' > "$stub/rustc"
printf '#!/bin/sh\nexit 1\n' > "$stub/cargo"
chmod +x "$stub/rustc" "$stub/cargo"

assert_guard_passed() {
  local label="$1"
  shift
  local out
  out=$(PATH="$stub:$PATH" "$PKG" "$@" 2>&1) || true
  if printf '%s\n' "$out" | grep -q "version mismatch"; then
    echo "SELF-TEST FAILED: guard cried wolf on $label:" >&2
    printf '%s\n' "$out" >&2
    exit 1
  fi
}

# The exact crate version, v-prefixed as the release workflow supplies it.
assert_guard_passed "matching tag 'v${CRATE_VERSION}'" "v${CRATE_VERSION}"
# A pre-release suffix on the same core is still a match.
assert_guard_passed "pre-release tag 'v${CRATE_VERSION}-rc1'" "v${CRATE_VERSION}-rc1"
# No argument at all: the -local default is derived from the crate and untouched.
assert_guard_passed "the argument-less -local default"

echo "package-version-guard self-test: ok"
