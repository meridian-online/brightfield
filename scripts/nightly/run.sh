#!/usr/bin/env bash
# The nightly gate's launcher, in two stages.
#
# Stage one is what launchd runs: a copy of this file that scripts/nightly/
# install.sh put under ~/.cache/meridian/gate/nightly/. It fetches a ref from
# `origin` of the repository BRIGHTFIELD_NIGHTLY_REPO names, adds a detached
# worktree at it, and runs that worktree's own copy of this file with
# `--in-tree <sha>`. The repository is only an object store: its branch and
# working files are never read, so every line of code the nightly runs is the
# fetched ref's.
#
# Stage two (`--in-tree`) runs inside that worktree and builds nothing itself
# beyond calling `cargo xtask nightly --sha <sha>` with the toolchain the tree
# pins. It does not own the worktree: stage one removes it after stage two
# exits, whatever its exit code, and exits with that code.
#
# BRIGHTFIELD_NIGHTLY_REF names the ref to fetch (default `main`). Any other
# ref runs as a trial (`--trial`), so a branch can be rehearsed without its
# verdict being taken for main's.
set -euo pipefail

die() { echo "nightly: $*" >&2; exit 2; }

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
  channel=$(sed -n 's/^channel[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' rust-toolchain.toml | head -n 1)
  [ -n "$channel" ] || die "no channel in this tree's rust-toolchain.toml"

  # ${trial:+...} so no empty argument reaches xtask when this is main's run.
  cargo "+${channel}" xtask nightly --sha "$sha" ${trial:+"$trial"}
}

if [ "${1:-}" = "--in-tree" ]; then
  in_tree "$@"
  exit $?
fi
[ "$#" -eq 0 ] || die "unknown argument: $1 (usage: run.sh [--in-tree <sha> [--trial]])"

repo="${BRIGHTFIELD_NIGHTLY_REPO:-}"
[ -n "$repo" ] || die "BRIGHTFIELD_NIGHTLY_REPO is not set; it names the checkout whose origin is fetched"
ref="${BRIGHTFIELD_NIGHTLY_REF:-main}"
case "$ref" in -*) die "BRIGHTFIELD_NIGHTLY_REF must not start with '-': $ref" ;; esac

git -C "$repo" fetch --quiet origin "$ref"
sha=$(git -C "$repo" rev-parse --verify "FETCH_HEAD^{commit}")

# The installed copy follows main: if the fetched ref's run.sh differs, replace
# this file with it before handing over, so a change to stage one reaches the
# next night without anyone re-running install. Only the installed copy is
# replaced; `run-now` runs the checkout's file, which is left alone. The temp
# file sits beside the copy so mv is an atomic rename, and the running shell
# keeps reading the old inode.
installed_dir="$HOME/.cache/meridian/gate/nightly"
self_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
if [ -d "$installed_dir" ] && [ "$self_dir" = "$(cd "$installed_dir" && pwd -P)" ] \
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

tree="$HOME/.cache/meridian/gate/trees/nightly-${sha:0:7}"
cleanup() {
  git -C "$repo" worktree remove --force "$tree" >/dev/null 2>&1 || rm -rf "$tree"
  git -C "$repo" worktree prune >/dev/null 2>&1 || true
}
trap cleanup EXIT
# A tree left by a run that was killed would make `worktree add` refuse.
if [ -e "$tree" ]; then cleanup; fi
mkdir -p "$(dirname "$tree")"
git -C "$repo" worktree add --quiet --detach "$tree" "$sha"
[ -f "$tree/scripts/nightly/run.sh" ] \
  || die "${ref} (${sha:0:7}) has no scripts/nightly/run.sh"

args=(--in-tree "$sha")
[ "$ref" = "main" ] || args+=(--trial)
echo "nightly: running ${ref} at ${sha}"
status=0
bash "$tree/scripts/nightly/run.sh" "${args[@]}" || status=$?
exit "$status"
