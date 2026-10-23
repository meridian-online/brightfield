#!/usr/bin/env bash
# Prove the read jail in scripts/airgap-jail.sh refuses what it is meant to
# refuse, with no artefact and no bundle, so that it runs on every pull request
# rather than only on a release.
#
# The same shape as scripts/verify-airgapped.sh's network control, which runs
# curl inside the jail and requires it to fail: a jail nobody has watched bite
# proves nothing about what runs inside it. The type-source leg there passing
# under this profile says the artefact read nothing outside itself only if a
# read outside it would have been refused, and these cases are that "would".
#
# And the positive case beside each refusal, because a profile that refuses
# every read passes every negative case here and runs nothing at all.
#
# macOS only: the jail is `sandbox-exec`. On another host every case is
# reported as not applicable, not counted as a pass, and test.yml runs this
# file on macos-15 so none of them is a branch CI never takes.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/airgap-jail.sh
. "$HERE/airgap-jail.sh"

if [ "$(uname -s)" != "Darwin" ]; then
  echo "airgap-jail-selftest: not applicable on $(uname -s) — the read jail is macOS's sandbox-exec"
  exit 0
fi

failures=0
pass() { echo "   ok: $*"; }
fail() { echo "   FAILED: $*"; failures=$((failures + 1)); }

# Under ${TMPDIR:-/tmp} deliberately: both are reached through a symlink on
# macOS (/var -> /private/var, /tmp -> /private/tmp), so the granted directory
# is named here by a path the sandbox does not match, and the inside case only
# passes if read_jail_profile resolved it.
WORK=$(mktemp -d "${TMPDIR:-/tmp}/bf-readjail.XXXXXX")
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$WORK/granted" "$WORK/outside"
printf 'inside-%s\n' "$$" > "$WORK/granted/inside.txt"
printf 'outside-%s\n' "$$" > "$WORK/outside/secret.txt"
ln -s "$WORK/outside/secret.txt" "$WORK/granted/link-to-secret.txt"

# /bin is granted alongside, because the programs the cases run live there and
# a process has to read its own executable. The type-source leg grants the
# unpacked artefact for the same reason: the binary is inside it.
PROFILE=$(read_jail_profile "$WORK/granted" /bin) || { echo "airgap-jail-selftest: no profile"; exit 1; }
jail() { sandbox-exec -p "$PROFILE" "$@"; }

echo "== a file inside the grant is read"
out=$(jail /bin/cat "$WORK/granted/inside.txt" 2>&1)
if [ "$out" = "inside-$$" ]; then pass "read it, through a symlinked temp path"
else fail "a read inside the grant did not come back (${out}) — this jail refuses everything, so every refusal below is meaningless"; fi

echo "== a file outside the grant is refused"
status=0; out=$(jail /bin/cat "$WORK/outside/secret.txt" 2>&1) || status=$?
if [ "$status" -ne 0 ] && [ "${out#*outside-$$}" = "$out" ]; then pass "refused (exit ${status})"
else fail "read a file outside the grant (exit ${status}): ${out}"; fi

echo "== a symlink inside the grant to a file outside it is refused"
status=0; out=$(jail /bin/cat "$WORK/granted/link-to-secret.txt" 2>&1) || status=$?
if [ "$status" -ne 0 ] && [ "${out#*outside-$$}" = "$out" ]; then pass "refused (exit ${status}) — the jail matches where the bytes are, not the name"
else fail "followed a symlink out of the grant (exit ${status}): ${out}"; fi

# --check-type-source re-execs itself to seal its environment, so the verdict
# comes from a child of the process the jail was put around.
# The child reads inside the grant first, because a shell that cannot start
# under the profile would pass the refusal for the wrong reason.
echo "== a child of the jailed process is refused too"
out=$(jail /bin/sh -c "/bin/cat '$WORK/granted/inside.txt'" 2>&1)
if [ "$out" != "inside-$$" ]; then
  fail "a child process could not read inside the grant (${out}), so its refusal below would prove nothing"
else
  status=0; out=$(jail /bin/sh -c "/bin/cat '$WORK/outside/secret.txt'" 2>&1) || status=$?
  if [ "$status" -ne 0 ] && [ "${out#*outside-$$}" = "$out" ]; then pass "read inside the grant and refused outside it, in the child (exit ${status})"
  else fail "a child process read outside the grant (exit ${status}): ${out}"; fi
fi

echo "== the read jail still denies the network"
if command -v curl >/dev/null; then
  # curl needs more than /bin to start, and a curl that cannot start fails this
  # case for the wrong reason, so it is first required to run.
  NET_PROFILE=$(read_jail_profile "$WORK/granted" /usr/bin /private/etc /private/var/run /Library)
  if ! sandbox-exec -p "$NET_PROFILE" /usr/bin/curl --version >/dev/null 2>&1; then
    fail "curl does not start inside the read jail, so its network failure would prove nothing"
  elif sandbox-exec -p "$NET_PROFILE" /usr/bin/curl -sS --max-time 10 https://example.com -o /dev/null 2>/dev/null; then
    fail "curl reached the network inside the read jail"
  else
    pass "curl starts and cannot reach the network"
  fi
else
  fail "curl is required for the network case"
fi

echo "== a directory that does not exist is refused, not granted as written"
if read_jail_profile "$WORK/no-such-dir" >/dev/null 2>&1; then fail "granted a directory that does not exist"
else pass "refused"; fi

if [ "$failures" -ne 0 ]; then
  echo "airgap-jail-selftest: ${failures} case(s) failed"
  exit 1
fi
echo "airgap-jail-selftest: the read jail refuses reads outside its grant and lets them in inside it"
