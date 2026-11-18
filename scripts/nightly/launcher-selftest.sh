#!/usr/bin/env bash
# Prove the nightly launcher does what its design says, without launchd and
# without the real home directory: install.sh renders a plist and an installed
# copy of run.sh under a temporary HOME, and run.sh hands a fetched ref to that
# ref's own copy of itself inside a throwaway worktree, which is gone again
# afterwards whatever the build exits with.
#
# cargo is a stub that records where it ran and with what, so this never builds
# anything. launchctl is a stub that fails the test if it is ever called, since
# BRIGHTFIELD_NIGHTLY_DRY_RUN=1 promises it is not. kata is a stub that logs
# its arguments and keeps one issue's state in files, so a launcher failure's
# report is checked without touching the real issue tracker.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
REAL_HOME="$HOME"

failures=0
pass() { echo "   ok: $*"; }
fail() { echo "   FAILED: $*"; failures=$((failures + 1)); }

TMP=$(mktemp -d "${TMPDIR:-/tmp}/bf-nightly-selftest.XXXXXX")
# Physical path, so a macOS /var -> /private/var symlink cannot make the paths
# the stub reports differ from the ones this script computes.
TMP=$(cd "$TMP" && pwd -P)
trap 'rm -rf "$TMP"' EXIT

export HOME="$TMP/home"
mkdir -p "$HOME"
# Git has no identity under the temporary HOME, and signing config of the real
# one must not apply to the throwaway repositories below.
export GIT_AUTHOR_NAME=selftest GIT_AUTHOR_EMAIL=selftest@example.invalid
export GIT_COMMITTER_NAME=selftest GIT_COMMITTER_EMAIL=selftest@example.invalid
export GIT_CONFIG_NOSYSTEM=1

STUBS="$TMP/stubs"
mkdir -p "$STUBS"
cat > "$STUBS/cargo" <<'STUB'
#!/usr/bin/env bash
{
  echo "pwd=$(pwd -P)"
  echo "head=$(git rev-parse HEAD)"
  echo "dirty=$(git status --porcelain | wc -l | tr -d ' ')"
  echo "args=$*"
} >> "$STUB_LOG"
# run.sh starts the toolchain and builds xtask before running it: those calls
# succeed unless STUB_NO_TOOLCHAIN asks for a missing toolchain, and
# STUB_EXIT is the exit code of the xtask run alone.
case "${2:-}" in
  --version) [ -z "${STUB_NO_TOOLCHAIN:-}" ] || { echo "error: toolchain '${1#+}' is not installed" >&2; exit 1; }; exit 0 ;;
  build) exit 0 ;;
esac
exit "${STUB_EXIT:-0}"
STUB
cat > "$STUBS/kata" <<'STUB'
#!/usr/bin/env bash
echo "kata $*" >> "$KATA_LOG"
case "$1" in
  list) if [ -e "$KATA_STATE/created" ]; then echo '{"issues":[{"short_id":"bf-1"}]}'; else echo '{"issues":[]}'; fi ;;
  create) touch "$KATA_STATE/created" ;;
  show)
    if [ -s "$KATA_STATE/comment" ]; then printf '{"comments":[{"body":"%s"}]}\n' "$(cat "$KATA_STATE/comment")"
    else echo '{"comments":[]}'; fi ;;
  comment) printf '%s' "$4" > "$KATA_STATE/comment" ;;
esac
exit 0
STUB
cat > "$STUBS/launchctl" <<'STUB'
#!/usr/bin/env bash
echo "launchctl $*" >> "$LAUNCHCTL_LOG"
exit 0
STUB
chmod +x "$STUBS/cargo" "$STUBS/launchctl" "$STUBS/kata"
export STUB_LOG="$TMP/stub.log" LAUNCHCTL_LOG="$TMP/launchctl.log"
export KATA_LOG="$TMP/kata.log" KATA_STATE="$TMP/kata"
mkdir -p "$KATA_STATE"
# A fresh tracker with no issue and an empty log.
reset_kata() { : > "$KATA_LOG"; rm -f "$KATA_STATE"/*; }
export PATH="$STUBS:$PATH"

AGENTS="$HOME/Library/LaunchAgents"
PLIST="$AGENTS/online.meridian.brightfield-nightly.plist"
INSTALLED="$HOME/.cache/meridian/gate/nightly/run.sh"

echo "== install renders a plist without calling launchctl"
BRIGHTFIELD_NIGHTLY_DRY_RUN=1 bash "$ROOT/scripts/nightly/install.sh" install >"$TMP/install.out" 2>&1
status=$?
if [ "$status" -eq 0 ] && [ -f "$PLIST" ]; then pass "wrote $PLIST under the temporary HOME"
else fail "install exited $status or wrote no plist: $(cat "$TMP/install.out")"; fi
if [ ! -e "$LAUNCHCTL_LOG" ]; then pass "launchctl was never called"
else fail "launchctl was called: $(cat "$LAUNCHCTL_LOG")"; fi

if command -v plutil >/dev/null 2>&1; then
  if plutil -lint "$PLIST" >/dev/null 2>&1; then pass "plutil accepts the plist"
  else fail "plutil rejects the plist: $(plutil -lint "$PLIST" 2>&1)"; fi
else
  echo "   skipped: plutil is not installed here"
fi
if ! grep -q '@' "$PLIST" 2>/dev/null; then pass "no @ placeholder is left"
else fail "a placeholder is left: $(grep -n '@' "$PLIST")"; fi
if grep -qF "$INSTALLED" "$PLIST" 2>/dev/null && ! grep -qF "$ROOT/scripts/nightly/run.sh" "$PLIST" 2>/dev/null; then
  pass "ProgramArguments names the installed copy, not the checkout's run.sh"
else fail "the plist does not name only $INSTALLED"; fi
for want in online.meridian.brightfield-nightly "<integer>2</integer>" "<integer>30</integer>" \
  CARGO_BUILD_JOBS RUST_TEST_THREADS BRIGHTFIELD_NIGHTLY_REPO "$ROOT" "$HOME/Library/Logs/brightfield-nightly"; do
  if grep -qF -- "$want" "$PLIST" 2>/dev/null; then pass "the plist carries $want"
  else fail "the plist lacks $want"; fi
done
if [ -x "$INSTALLED" ] && cmp -s "$INSTALLED" "$ROOT/scripts/nightly/run.sh"; then pass "the installed copy exists and matches the checkout's"
else fail "no executable installed copy at $INSTALLED"; fi

echo "== no machine path is tracked"
# The patterns are built from pieces so this file's own lines do not match
# them. --no-index also reads files git does not track yet, so the check holds
# before the first commit as well as after.
p1='/Us''ers/'; p2='/Vol''umes/'; p3='/ho''me/'
if hits=$(cd "$ROOT" && git grep --no-index -nE "$p1|$p2|$p3" -- scripts/nightly) && [ -n "$hits" ]; then
  fail "a machine path appears in scripts/nightly: $hits"
else pass "no user, volume or home path under scripts/nightly"; fi
if [ "${#REAL_HOME}" -ge 3 ] && hits=$(cd "$ROOT" && git grep --no-index -nF -- "$REAL_HOME" -- scripts/nightly) && [ -n "$hits" ]; then
  fail "the real home directory appears in scripts/nightly: $hits"
else pass "the real home directory appears nowhere under scripts/nightly"; fi

echo "== run.sh rejects an argument it does not know"
out=$(bash "$ROOT/scripts/nightly/run.sh" --in-tree 0000000000000000000000000000000000000000 --bogus 2>&1); status=$?
if [ "$status" -ne 0 ] && [ "${out#*unknown argument}" != "$out" ]; then pass "refused with: ${out}"
else fail "exit ${status}, output: ${out}"; fi

echo "== run.sh hands main's head to that head's own run.sh"
ORIGIN="$TMP/origin"; CLONE="$TMP/clone"
git init -q -b main "$ORIGIN"
mkdir -p "$ORIGIN/scripts/nightly"
# A toolchain that is not this checkout's, so the stub's first argument proves
# the channel came from the fetched tree.
printf '[toolchain]\nchannel = "9.9.9"\n' > "$ORIGIN/rust-toolchain.toml"
MARKER="marker-$$"
# The committed run.sh is the checkout's plus a line that records which copy
# ran, so a stage that ran the checkout's (or the installed) file instead of
# the worktree's would leave no marker naming a worktree path.
awk -v m="$MARKER" 'NR==1 { print; print "echo \"" m " $0\" >> \"$MARKER_FILE\""; next } { print }' \
  "$ROOT/scripts/nightly/run.sh" > "$ORIGIN/scripts/nightly/run.sh"
chmod +x "$ORIGIN/scripts/nightly/run.sh"
git -C "$ORIGIN" add -A >/dev/null && git -C "$ORIGIN" commit -qm "first" && git clone -q "$ORIGIN" "$CLONE"
# The clone is behind: the commit the nightly must run is only on origin.
echo "second" > "$ORIGIN/note.txt"
git -C "$ORIGIN" add note.txt && git -C "$ORIGIN" commit -qm "second"
MAIN_SHA=$(git -C "$ORIGIN" rev-parse main)
export MARKER_FILE="$TMP/marker.log"

run_installed() {
  HOME="$HOME" BRIGHTFIELD_NIGHTLY_REPO="$CLONE" "$@" "$INSTALLED" >"$TMP/run.out" 2>&1
}
# Another run's tree for the same commit, which this run must not touch.
OTHER="$HOME/.cache/meridian/gate/trees/nightly-${MAIN_SHA:0:7}.other"
mkdir -p "$OTHER" && echo keep > "$OTHER/file"
# A tree a killed run left two days ago, which this run reclaims.
OLD="$HOME/.cache/meridian/gate/trees/nightly-0000000.old"
# A second one that cannot be deleted: a file in a directory without write
# permission. The run must warn and carry on. Skipped as root, which ignores it.
STUCK="$HOME/.cache/meridian/gate/trees/nightly-0000000.stuck"
mkdir -p "$STUCK" && echo x > "$STUCK/f" && chmod 555 "$STUCK"
touch -t "$(date -v-2d +%Y%m%d%H%M 2>/dev/null || date -d '2 days ago' +%Y%m%d%H%M)" "$STUCK"
mkdir -p "$OLD" && touch -t "$(date -v-2d +%Y%m%d%H%M 2>/dev/null || date -d '2 days ago' +%Y%m%d%H%M)" "$OLD"
: > "$STUB_LOG"
run_installed env; status=$?
if [ "$status" -eq 0 ]; then pass "exited 0"; else fail "exited $status: $(cat "$TMP/run.out")"; fi
if [ "$(id -u)" -eq 0 ]; then echo "   skipped: root can remove a tree with no write permission"
elif [ -d "$STUCK" ] && grep -qF "could not remove stale tree $STUCK" "$TMP/run.out"; then pass "a stale tree that cannot be removed was warned about, and the run went on"
else fail "no warning for the undeletable tree: $(cat "$TMP/run.out")"; fi
chmod 755 "$STUCK" 2>/dev/null; rm -rf "$STUCK"
TREES="$HOME/.cache/meridian/gate/trees"
TREE=$(sed -n 's/^pwd=//p' "$STUB_LOG" | head -n 1)
if grep -qxF "args=+9.9.9 xtask nightly --sha $MAIN_SHA" "$STUB_LOG"; then pass "cargo saw +<tree's channel> xtask nightly --sha <origin's main>, no --trial"
else fail "cargo's arguments were: $(grep '^args=' "$STUB_LOG")"; fi
case "$TREE" in "$TREES/nightly-${MAIN_SHA:0:7}."*) tree_ok=1 ;; *) tree_ok=0 ;; esac
if [ "$tree_ok" = 1 ] && grep -qxF "head=$MAIN_SHA" "$STUB_LOG" && grep -qxF "dirty=0" "$STUB_LOG"; then
  pass "cargo ran in a clean worktree at that sha"
else fail "cargo's surroundings were: $(cat "$STUB_LOG")"; fi
if grep -qF "$MARKER $TREE/scripts/nightly/run.sh" "$MARKER_FILE" 2>/dev/null; then pass "the worktree's own run.sh ran"
else fail "no marker from the worktree's run.sh: $(cat "$MARKER_FILE" 2>/dev/null)"; fi
if [ ! -e "$TREE" ] && ! git -C "$CLONE" worktree list | grep -qF "$TREE"; then pass "the worktree is gone afterwards"
else fail "the worktree is still there"; fi
if [ ! -e "$OLD" ]; then pass "a tree two days old was reclaimed"
else fail "the old tree is still there"; fi
if [ -f "$OTHER/file" ]; then pass "another run's tree for the same commit survived"
else fail "a pre-existing tree was removed"; fi
if cmp -s "$INSTALLED" "$ORIGIN/scripts/nightly/run.sh"; then pass "the installed copy now matches the fetched ref's"
else fail "the installed copy still differs from the fetched ref's run.sh"; fi
leftovers=$(find "$(dirname "$INSTALLED")" -type f ! -name run.sh)
if [ -z "$leftovers" ]; then pass "no temporary file is left beside the installed copy"
else fail "left beside it: $leftovers"; fi

echo "== the worktree is removed, and the exit code passed on, when the build fails"
: > "$STUB_LOG"
reset_kata
run_installed env STUB_EXIT=1; status=$?
if [ "$status" -eq 1 ]; then pass "exited with the build's code, 1"; else fail "exited $status: $(cat "$TMP/run.out")"; fi
TREE=$(sed -n 's/^pwd=//p' "$STUB_LOG" | head -n 1)
if grep -q '^args=' "$STUB_LOG"; then pass "the stub did run"; else fail "the stub never ran: $(cat "$TMP/run.out")"; fi
if [ ! -e "$TREE" ] && ! git -C "$CLONE" worktree list | grep -qF "$TREE"; then pass "the worktree is gone afterwards"
else fail "the worktree is still there after a failed build"; fi
if [ ! -s "$KATA_LOG" ]; then pass "a stop after xtask started is left to xtask to report"
else fail "the launcher called kata after xtask started: $(cat "$KATA_LOG")"; fi

echo "== another ref is a trial run at that ref's sha"
git -C "$ORIGIN" checkout -q -b trial-x
echo "third" > "$ORIGIN/note.txt"
echo "# trial launcher" >> "$ORIGIN/scripts/nightly/run.sh"
git -C "$ORIGIN" add note.txt scripts/nightly/run.sh && git -C "$ORIGIN" commit -qm "third"
BEFORE_TRIAL=$(cksum < "$INSTALLED")
TRIAL_SHA=$(git -C "$ORIGIN" rev-parse trial-x)
: > "$STUB_LOG"
run_installed env BRIGHTFIELD_NIGHTLY_REF=trial-x; status=$?
if [ "$status" -eq 0 ]; then pass "exited 0"; else fail "exited $status: $(cat "$TMP/run.out")"; fi
if grep -qxF "args=+9.9.9 xtask nightly --sha $TRIAL_SHA --trial" "$STUB_LOG"; then pass "cargo saw the branch's sha and --trial"
else fail "cargo's arguments were: $(grep '^args=' "$STUB_LOG")"; fi

if [ "$(cksum < "$INSTALLED")" = "$BEFORE_TRIAL" ]; then pass "a trial run left the installed copy unchanged"
else fail "a trial run replaced the installed copy"; fi

echo "== a main run that stops before xtask starts raises nightly-missing"
BROKEN="$TMP/broken"
git init -q "$BROKEN" && git -C "$BROKEN" remote add origin "$TMP/nowhere"
DAY=$(date +%Y%m%d)
KEY="the nightly launcher stopped on $DAY before the gate started"
reset_kata
HOME="$HOME" BRIGHTFIELD_NIGHTLY_REPO="$BROKEN" bash "$ROOT/scripts/nightly/run.sh" >"$TMP/run.out" 2>&1; status=$?
if [ "$status" -ne 0 ]; then pass "exited $status"; else fail "exited 0 though the fetch failed"; fi
for want in "kata list --label nightly-missing --json --project brightfield" \
  "kata create main's nightly gate has not given a verdict --body The nightly gate posts a" \
  "--label nightly-missing --force-new --agent --project brightfield" \
  "kata meta set bf-1 work.attention needs-human --agent --project brightfield" \
  "kata meta set bf-1 work.attention_msg main's nightly gate has not given a verdict --agent --project brightfield" \
  "kata comment bf-1 --body $KEY: fetch origin main --agent --project brightfield"; do
  if grep -qF -- "$want" "$KATA_LOG"; then pass "kata saw: $want"
  else fail "kata never saw '$want'; it saw: $(cat "$KATA_LOG")"; fi
done
: > "$KATA_LOG"
HOME="$HOME" BRIGHTFIELD_NIGHTLY_REPO="$BROKEN" bash "$ROOT/scripts/nightly/run.sh" >"$TMP/run.out" 2>&1
if grep -qF "kata meta set bf-1 work.attention needs-human" "$KATA_LOG" \
  && ! grep -qE '^kata (create|comment) ' "$KATA_LOG"; then
  pass "a second stop the same day flags the open issue again, with no new issue or comment"
else fail "the second stop's kata calls were: $(cat "$KATA_LOG")"; fi

reset_kata
HOME="$HOME" BRIGHTFIELD_NIGHTLY_REPO="$BROKEN" BRIGHTFIELD_NIGHTLY_REF=trial-x \
  bash "$ROOT/scripts/nightly/run.sh" >"$TMP/run.out" 2>&1; status=$?
if [ "$status" -ne 0 ] && [ ! -s "$KATA_LOG" ]; then pass "a trial whose fetch fails raises nothing"
else fail "exit $status; kata saw: $(cat "$KATA_LOG")"; fi

reset_kata
: > "$STUB_LOG"
HOME="$HOME" BRIGHTFIELD_NIGHTLY_REPO="$CLONE" STUB_NO_TOOLCHAIN=1 \
  bash "$ROOT/scripts/nightly/run.sh" >"$TMP/run.out" 2>&1; status=$?
if [ "$status" -ne 0 ] && grep -qF "kata comment bf-1 --body $KEY: start cargo +9.9.9 --agent" "$KATA_LOG"; then
  pass "a toolchain stage two cannot start is reported, naming it"
else fail "exit $status; kata saw: $(cat "$KATA_LOG"); run said: $(cat "$TMP/run.out")"; fi
if ! grep -q 'xtask nightly' "$STUB_LOG"; then pass "xtask was never run"
else fail "xtask ran without a toolchain: $(cat "$STUB_LOG")"; fi

echo "== run.sh raises nightly-missing with the strings flags.rs uses"
FLAG_LINES=$(sed -n "/<<'FLAG'\$/,/^FLAG\$/p" "$ROOT/scripts/nightly/run.sh" | sed '1d;$d')
if [ "$(printf '%s\n' "$FLAG_LINES" | wc -l | tr -d ' ')" -eq 3 ]; then pass "run.sh carries a title, a body and an attention line"
else fail "run.sh's FLAG block is not three lines: $FLAG_LINES"; fi
while IFS= read -r line; do
  if grep -qF -- "\"$line\"" "$ROOT/xtask/src/flags.rs"; then pass "flags.rs spells \"${line:0:40}...\" the same"
  else fail "flags.rs has no string \"$line\"; keep run.sh's FLAG block in step with NIGHTLY_MISSING"; fi
done <<< "$FLAG_LINES"
if grep -qF 'label: "nightly-missing"' "$ROOT/xtask/src/flags.rs"; then pass "flags.rs labels the issue nightly-missing too"
else fail "flags.rs no longer labels NIGHTLY_MISSING nightly-missing"; fi

echo "== run.sh without a repository says so"
out=$(env -u BRIGHTFIELD_NIGHTLY_REPO "$INSTALLED" 2>&1); status=$?
if [ "$status" -ne 0 ] && [ "${out#*BRIGHTFIELD_NIGHTLY_REPO}" != "$out" ]; then pass "refused, naming BRIGHTFIELD_NIGHTLY_REPO"
else fail "exit ${status}, output: ${out}"; fi

echo "== uninstall removes what install wrote"
BRIGHTFIELD_NIGHTLY_DRY_RUN=1 bash "$ROOT/scripts/nightly/install.sh" uninstall >"$TMP/uninstall.out" 2>&1; status=$?
if [ "$status" -eq 0 ] && [ ! -e "$PLIST" ] && [ ! -e "$INSTALLED" ]; then pass "plist and installed copy are gone"
else fail "uninstall exited $status; plist/copy left: $(ls "$PLIST" "$INSTALLED" 2>&1)"; fi
if [ ! -e "$LAUNCHCTL_LOG" ]; then pass "launchctl was still never called"
else fail "launchctl was called: $(cat "$LAUNCHCTL_LOG")"; fi

echo
if [ "$failures" -ne 0 ]; then echo "launcher-selftest: $failures check(s) FAILED"; exit 1; fi
echo "launcher-selftest: all checks passed"
