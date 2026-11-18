#!/usr/bin/env bash
# The nightly gate's launcher, in two stages.
#
# Stage one is what launchd runs: a copy of this file that scripts/nightly/
# install.sh put under ~/.cache/meridian/gate/nightly/. It fetches a ref from
# `origin` of the nightly's own clone of Brightfield, adds a detached worktree
# at it, and runs that worktree's own copy of this file with `--in-tree <sha>`.
# The clone is only an object store: its branch and working files are never
# read, so every line of code the nightly runs is the fetched ref's.
#
# The clones live on the internal disk, under ~/.cache/meridian/gate/nightly/,
# because a launchd job cannot read an external volume without a disk-access
# grant. Stage one makes them when they are missing and fetches them
# otherwise: brightfield/ and open-analytics/. Both are only object stores:
# each run adds its own detached worktree of open-analytics' fetched main and
# names it to stage two in OPEN_ANALYTICS_DIR. BRIGHTFIELD_NIGHTLY_REPO
# names another Brightfield repository in place of the clone, and
# BRIGHTFIELD_NIGHTLY_REMOTE and BRIGHTFIELD_NIGHTLY_OA_REMOTE name other
# remotes to clone from, which is how the self-test avoids the network.
#
# Stage two (`--in-tree`) runs inside that worktree and builds nothing itself
# beyond calling `cargo xtask nightly --sha <sha>` with the toolchain the tree
# pins. It does not own the worktree: stage one removes it after stage two
# exits, whatever its exit code, and exits with that code.
#
# BRIGHTFIELD_NIGHTLY_REF names the ref to fetch (default `main`). Any other
# ref runs as a trial (`--trial`), so a branch can be rehearsed without its
# verdict being taken for main's.
#
# Once `cargo xtask nightly` starts, it reports its own stops. A main run that
# stops before then (a clone or fetch fails, the worktree cannot be made, the tree's
# toolchain is not installed, xtask does not build) raises the same
# `nightly-missing` Kata issue from here instead, so a launcher that fails
# every night does not go unseen until a pull request's gate runs.
set -euo pipefail

# rustup's proxies come first on PATH, in both stages and in everything they
# start, and Homebrew's directory second. A launchd job runs `bash -lc`, whose
# login profile rebuilds PATH with macOS's path_helper: the system directories
# go first and the plist's entries after them. That finds Apple's python3, too
# old for the release-readback checks, and can put Homebrew's cargo ahead of
# rustup's; Homebrew's cargo is not a rustup proxy, so it rejects
# `cargo +<channel>` and would ignore the tree's rust-toolchain.toml.
cargo_bin="${CARGO_HOME:-$HOME/.cargo}/bin"
for brew_bin in /opt/homebrew/bin /usr/local/bin; do
  if [ -d "$brew_bin" ]; then
    export PATH="$brew_bin:$PATH"
    break
  fi
done
export PATH="$cargo_bin:$PATH"

die() { echo "nightly: $*" >&2; exit 2; }

# Stage two names each step in the file BRIGHTFIELD_NIGHTLY_STAGE_FILE names,
# when stage one set it, so stage one can tell whether xtask was reached.
mark() {
  if [ -n "${BRIGHTFIELD_NIGHTLY_STAGE_FILE:-}" ]; then
    printf '%s\n' "$1" > "$BRIGHTFIELD_NIGHTLY_STAGE_FILE" || true
  fi
}

in_tree() {
  local sha="" trial=""
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --in-tree) [ "$#" -ge 2 ] || die "--in-tree needs a sha"; sha="$2"; shift 2 ;;
      --trial) trial="--trial"; shift ;;
      *) die "unknown argument: $1 (usage: run.sh [--in-tree <sha> [--trial]])" ;;
    esac
  done
  [[ "$sha" =~ ^[0-9a-f]{40}$ ]] || die "--in-tree needs a 40-character lowercase hex sha, got '${sha}'"

  # The tree is this file's own: scripts/nightly/run.sh two levels down.
  local tree
  tree="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
  cd "$tree"
  local channel
  mark "read the tree's toolchain"
  channel=$(sed -n 's/^channel[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' rust-toolchain.toml | head -n 1)
  [ -n "$channel" ] || die "no channel in this tree's rust-toolchain.toml"

  # Start the toolchain and build xtask first, so a missing toolchain or an
  # xtask that does not compile is told apart from a stop xtask reports
  # itself. The build is the one the `xtask` alias makes, so the run below
  # reuses it.
  mark "start cargo +${channel}"
  cargo "+${channel}" --version >/dev/null
  mark "build xtask with cargo +${channel}"
  cargo "+${channel}" build --locked --quiet --package xtask
  mark "xtask"
  # ${trial:+...} so no empty argument reaches xtask when this is main's run.
  cargo "+${channel}" xtask nightly --sha "$sha" ${trial:+"$trial"}
}

# Raise the `nightly-missing` issue for a main run that stopped at "$1"
# before xtask started, the way xtask/src/flags.rs raises it: find the open
# issue or open one, flag it for Hugh, and comment once per day. Every
# tracker failure is printed and passed over.
# shellcheck disable=SC2329 # invoked by on_exit, the EXIT trap below
raise_missing() {
  local why="$1" day key issue shown title body attention
  # NIGHTLY_MISSING's title, body and attention line in xtask/src/flags.rs,
  # one per line; launcher-selftest.sh checks flags.rs still spells each so.
  { IFS= read -r title; IFS= read -r body; IFS= read -r attention; } <<'FLAG'
main's nightly gate has not given a verdict
The nightly gate posts a `success` or `failure` `nightly-gate` status on main each night. Main has gone more than 36 hours without one, or the nightly stopped before its steps, so main's state is unknown. The nightly may not be scheduled, the machine may have been off, or the run may have stopped early; its log says which. `cargo xtask gate` raises this issue but never closes it: only the nightly's next verdict does.
main's nightly gate has not given a verdict
FLAG
  day=$(date +%Y%m%d)
  key="the nightly launcher stopped on ${day}"
  echo "nightly: ${key} before the gate started: ${why}; raising nightly-missing" >&2
  say() { echo "nightly: the issue tracker call failed, and the launcher carries on: $*" >&2; }
  kt() { kata "$@" --project brightfield; }
  open_missing() {
    kt list --label nightly-missing --json \
      | python3 -c 'import json, sys; i = json.load(sys.stdin).get("issues") or []; print(i[0]["short_id"] if i else "")'
  }
  if ! issue=$(open_missing); then say "kata list"; return 0; fi
  if [ -z "$issue" ]; then
    # --force-new past a closed look-alike, as flags.rs does, after finding no
    # open one.
    if ! kt create "$title" --body "$body" --label nightly-missing --force-new --agent >/dev/null; then
      say "kata create"; return 0
    fi
    if ! issue=$(open_missing) || [ -z "$issue" ]; then say "kata create left no open nightly-missing issue"; return 0; fi
  fi
  kt meta set "$issue" work.attention needs-human --agent >/dev/null || say "kata meta set work.attention"
  kt meta set "$issue" work.attention_msg "$attention" --agent >/dev/null || say "kata meta set work.attention_msg"
  if ! shown=$(kt show "$issue" --json); then say "kata show"; return 0; fi
  if ! printf '%s' "$shown" | python3 -c '
import json, sys
key = sys.argv[1]
try:
    comments = json.load(sys.stdin).get("comments") or []
except ValueError:
    sys.exit(1)
sys.exit(0 if any(key in (c.get("body") or "") for c in comments) else 1)' "$key"; then
    kt comment "$issue" --body "${key} before the gate started: ${why}" --agent >/dev/null || say "kata comment"
  fi
  return 0
}

if [ "${1:-}" = "--in-tree" ]; then
  in_tree "$@"
  exit $?
fi
[ "$#" -eq 0 ] || die "unknown argument: $1 (usage: run.sh [--in-tree <sha> [--trial]])"

installed_dir="$HOME/.cache/meridian/gate/nightly"
repo="${BRIGHTFIELD_NIGHTLY_REPO:-$installed_dir/brightfield}"
oa_dir="$installed_dir/open-analytics"
ref="${BRIGHTFIELD_NIGHTLY_REF:-main}"
case "$ref" in -*) die "BRIGHTFIELD_NIGHTLY_REF must not start with '-': $ref" ;; esac

# From here every exit goes through on_exit: it removes the tree this run
# made, if any, and when a main run fails before xtask started, raises
# nightly-missing naming the step it stopped at. A trial raises nothing.
stage="fetch brightfield origin ${ref}"
tree=""
private_ref=""
oa_tree=""
oa_private_ref=""
stage_file="$(mktemp "${TMPDIR:-/tmp}/brightfield-nightly-stage.XXXXXX")"
export BRIGHTFIELD_NIGHTLY_STAGE_FILE="$stage_file"
# shellcheck disable=SC2329 # invoked by the EXIT trap below
cleanup() {
  git -C "$repo" worktree remove --force "$tree" >/dev/null 2>&1 || rm -rf "$tree"
  git -C "$repo" worktree prune >/dev/null 2>&1 || true
}
# shellcheck disable=SC2329 # invoked by the EXIT trap below
on_exit() {
  local status=$? reached
  set +e
  [ -z "$tree" ] || cleanup
  if [ -n "$oa_tree" ]; then
    git -C "$oa_dir" worktree remove --force "$oa_tree" >/dev/null 2>&1 || rm -rf "$oa_tree"
    git -C "$oa_dir" worktree prune >/dev/null 2>&1
  fi
  [ -z "$oa_private_ref" ] || git -C "$oa_dir" update-ref -d "$oa_private_ref" >/dev/null 2>&1
  [ -z "${private_ref:-}" ] || git -C "$repo" update-ref -d "$private_ref" >/dev/null 2>&1
  reached=$(cat "$stage_file" 2>/dev/null)
  rm -f "$stage_file"
  if [ "$status" -ne 0 ] && [ "$ref" = "main" ] && [ "$reached" != "xtask" ]; then
    raise_missing "${reached:-$stage}"
  fi
  exit "$status"
}
trap on_exit EXIT

stage="find rustup's cargo"
[ -x "$cargo_bin/rustup" ] || die "rustup is not in $cargo_bin, so cargo +<channel> cannot work"
stage="fetch brightfield origin ${ref}"

# FETCH_HEAD is one file per checkout, so an overlapping run (a trial started
# by hand during the scheduled one) could overwrite it between the fetch and
# the read. Fetch into a ref only this run knows, read the commit from it, and
# delete it on exit. The self-update below reads the commit this resolves.
#
# A clone that does not exist yet is made first, and only the default one is:
# a repository BRIGHTFIELD_NIGHTLY_REPO names is the caller's to have made.
# Brightfield's is made without a working tree, since only its objects are read.
if [ -z "${BRIGHTFIELD_NIGHTLY_REPO:-}" ] && [ ! -d "$repo/.git" ]; then
  stage="clone brightfield"
  mkdir -p "$installed_dir"
  git clone --quiet --no-checkout "${BRIGHTFIELD_NIGHTLY_REMOTE:-https://github.com/meridian-online/brightfield.git}" "$repo"
  stage="fetch brightfield origin ${ref}"
fi
private_ref="refs/nightly/${ref}.$$"
git -C "$repo" fetch --quiet origin "+refs/heads/${ref}:${private_ref}"

# open-analytics is an object store too, and its fetch ref is run-private for
# the same reason as Brightfield's. Each run checks out its own worktree of it
# below, since a shared working tree would be rewritten under an overlapping
# run's gate. The nightly tests open-analytics' main.
if [ ! -d "$oa_dir/.git" ]; then
  stage="clone open-analytics"
  mkdir -p "$installed_dir"
  git clone --quiet --no-checkout "${BRIGHTFIELD_NIGHTLY_OA_REMOTE:-https://github.com/meridian-online/open-analytics.git}" "$oa_dir"
  stage="fetch open-analytics origin main"
fi
stage="fetch open-analytics origin main"
oa_private_ref="refs/nightly/main.$$"
git -C "$oa_dir" fetch --quiet origin "+refs/heads/main:${oa_private_ref}"
stage="resolve open-analytics' fetched commit"
oa_sha=$(git -C "$oa_dir" rev-parse --verify "${oa_private_ref}^{commit}")

stage="resolve the fetched commit"
sha=$(git -C "$repo" rev-parse --verify "${private_ref}^{commit}")
stage="replace the installed run.sh"

# The installed copy follows main: if the fetched ref's run.sh differs, replace
# this file with it before handing over, so a change to stage one reaches the
# next night without anyone re-running install. Only the installed copy is
# replaced, and only from main: a trial branch must not install its launcher
# for the nights after it. `run-now` runs the checkout's file, which is left
# alone. The temp file sits beside the copy so mv is an atomic rename, and the
# running shell keeps reading the old inode.
self_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
if [ "$ref" = "main" ] && [ -d "$installed_dir" ] && [ "$self_dir" = "$(cd "$installed_dir" && pwd -P)" ] \
  && [ "$(basename "${BASH_SOURCE[0]}")" = "run.sh" ]; then
  fresh="$(mktemp "$installed_dir/run.sh.XXXXXX")"
  if git -C "$repo" show "${sha}:scripts/nightly/run.sh" > "$fresh" 2>/dev/null && [ -s "$fresh" ]; then
    if cmp -s "$fresh" "$installed_dir/run.sh"; then
      rm -f "$fresh"
    else
      chmod +x "$fresh"
      mv -f "$fresh" "$installed_dir/run.sh"
      echo "nightly: replaced the installed run.sh with ${ref}'s"
    fi
  else
    rm -f "$fresh"
  fi
fi

# Each run makes its own tree, and removes only the one it made. A fixed name
# per commit would let a hand-started run overlapping the scheduled one (say,
# while it waits on the gate lock) delete the other's tree mid-build.
stage="make the tree"
trees="$HOME/.cache/meridian/gate/trees"
mkdir -p "$trees"
# A run killed by SIGKILL or a reboot never reaches its trap and leaves its
# tree behind. A whole run, lock wait included, ends well inside a day, so a
# nightly-* tree older than that belongs to no live run. Git is asked first so
# its worktree record goes too; rm -rf only handles a path git does not know.
# Nothing outside $trees, and nothing younger than a day, is touched.
while IFS= read -r stale; do
  git -C "$repo" worktree remove --force "$stale" >/dev/null 2>&1 || rm -rf "$stale" \
    || echo "nightly: could not remove stale tree $stale" >&2
done < <(find "$trees" -mindepth 1 -maxdepth 1 -type d -name 'nightly-*' -mmin +1440)
git -C "$repo" worktree prune >/dev/null 2>&1 || true
git -C "$oa_dir" worktree prune >/dev/null 2>&1 || true
tree="$(mktemp -d "$trees/nightly-${sha:0:7}.XXXXXX")"
stage="add the worktree"
git -C "$repo" worktree add --quiet --detach "$tree" "$sha"
stage="add the open-analytics worktree"
oa_tree="$(mktemp -d "$trees/nightly-oa-${oa_sha:0:7}.XXXXXX")"
git -C "$oa_dir" worktree add --quiet --detach "$oa_tree" "$oa_sha"
export OPEN_ANALYTICS_DIR="$oa_tree"
stage="find the tree's run.sh"
[ -f "$tree/scripts/nightly/run.sh" ] \
  || die "${ref} (${sha:0:7}) has no scripts/nightly/run.sh"
stage="run the tree's run.sh"

args=(--in-tree "$sha")
[ "$ref" = "main" ] || args+=(--trial)
echo "nightly: running ${ref} at ${sha}"
status=0
bash "$tree/scripts/nightly/run.sh" "${args[@]}" || status=$?
exit "$status"
