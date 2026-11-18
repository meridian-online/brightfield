#!/usr/bin/env bash
# Install, remove, inspect or run now the nightly gate's LaunchAgent.
#
# Usage:
#   scripts/nightly/install.sh install
#       Renders the plist template with your home into
#       ~/Library/LaunchAgents, copies run.sh to
#       ~/.cache/meridian/gate/nightly/run.sh (the plist runs that copy, never
#       the checkout's file, so the code that runs is the fetched ref's), makes
#       the log directory and loads the agent. Idempotent: it reloads. The
#       nightly's own clones of Brightfield and open-analytics, beside the
#       copy, are made by the first run and need nothing from this checkout.
#
#   scripts/nightly/install.sh uninstall
#       Unloads the agent and removes the plist and the installed copy. Logs
#       are left in place.
#
#   scripts/nightly/install.sh status
#       Shows whether the plist is installed and loaded.
#
#   scripts/nightly/install.sh run-now
#       Runs the nightly in the foreground, as launchd would, against the
#       nightly's own clones. BRIGHTFIELD_NIGHTLY_REF picks the ref (default
#       main; any other is a trial run), and BRIGHTFIELD_NIGHTLY_REPO may
#       name another Brightfield repository in place of the clone.
#
# BRIGHTFIELD_NIGHTLY_DRY_RUN=1 makes install and uninstall write and remove
# files without ever calling launchctl, which is how the self-test runs them.
set -euo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
LABEL="online.meridian.brightfield-nightly"
PLIST_SRC="$REPO_ROOT/scripts/nightly/$LABEL.plist.in"
PLIST_DST="$HOME/Library/LaunchAgents/$LABEL.plist"
INSTALLED_DIR="$HOME/.cache/meridian/gate/nightly"
LOG_DIR="$HOME/Library/Logs/brightfield-nightly"

dry_run() { [ "${BRIGHTFIELD_NIGHTLY_DRY_RUN:-}" = "1" ]; }

# A path goes into sed's replacement, where \, | and & mean something.
sed_escape() { printf '%s' "$1" | sed 's/[\\|&]/\\&/g'; }

cmd_install() {
  [ -f "$PLIST_SRC" ] || { echo "error: template not found: $PLIST_SRC" >&2; exit 1; }
  mkdir -p "$LOG_DIR" "$INSTALLED_DIR" "$(dirname "$PLIST_DST")"

  # Copy through a temp file and rename, so a run that starts mid-install
  # never reads half a script.
  local tmp
  tmp="$(mktemp "$INSTALLED_DIR/run.sh.XXXXXX")"
  cp "$REPO_ROOT/scripts/nightly/run.sh" "$tmp"
  chmod +x "$tmp"
  mv -f "$tmp" "$INSTALLED_DIR/run.sh"

  sed -e "s|@HOME@|$(sed_escape "$HOME")|g" \
    "$PLIST_SRC" > "$PLIST_DST.tmp"
  if command -v plutil >/dev/null 2>&1; then
    plutil -lint "$PLIST_DST.tmp" >/dev/null || { rm -f "$PLIST_DST.tmp"; echo "error: the rendered plist is invalid" >&2; exit 1; }
  fi

  if ! dry_run && launchctl list "$LABEL" >/dev/null 2>&1; then
    echo "already loaded: unloading first"
    launchctl unload "$PLIST_DST" || true
  fi
  mv -f "$PLIST_DST.tmp" "$PLIST_DST"
  if dry_run; then
    echo "dry run: launchctl not called"
  else
    launchctl load "$PLIST_DST"
  fi
  echo "installed: $PLIST_DST"
  echo "runs:      $INSTALLED_DIR/run.sh, daily at 02:30"
  echo "logs:      $LOG_DIR/{stdout,stderr}.log"
}

cmd_uninstall() {
  if [ -f "$PLIST_DST" ]; then
    dry_run || launchctl unload "$PLIST_DST" || true
    rm -f "$PLIST_DST"
    echo "uninstalled: $PLIST_DST"
  else
    echo "not installed: $PLIST_DST"
  fi
  rm -f "$INSTALLED_DIR/run.sh"
}

cmd_status() {
  if [ -f "$PLIST_DST" ]; then echo "plist installed: $PLIST_DST"; else echo "plist NOT installed"; fi
  if [ -f "$INSTALLED_DIR/run.sh" ]; then echo "installed copy: $INSTALLED_DIR/run.sh"; else echo "installed copy: NOT present"; fi
  if launchctl list "$LABEL" >/dev/null 2>&1; then
    echo "launchctl: loaded"
    launchctl list "$LABEL" | head -20
  else
    echo "launchctl: NOT loaded"
  fi
}

cmd_run_now() {
  # Same caps the plist sets, since a shell started here may not have them.
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-6}" RUST_TEST_THREADS="${RUST_TEST_THREADS:-6}" \
    "$REPO_ROOT/scripts/nightly/run.sh"
}

case "${1:-}" in
  install) cmd_install ;;
  uninstall) cmd_uninstall ;;
  status) cmd_status ;;
  run-now) cmd_run_now ;;
  *) echo "usage: $0 {install|uninstall|status|run-now}" >&2; exit 2 ;;
esac
