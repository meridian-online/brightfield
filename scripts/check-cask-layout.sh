#!/usr/bin/env bash
# The tap cask installs the whole app and reaches the binary by an exec shim
# rather than a symlink.
#
#   scripts/check-cask-layout.sh CASK.rb
#
# Exit 0: accepted. Exit 1: refused, and stderr names the rule. Exit 2: the
# file is not there.
#
# The release's asset check proves the urls resolve and the checksums match,
# which is what `brew install` establishes before it opens the disk image.
# This proves what the cask does after: that it installs the app, and that the
# installed command can find the type source bundled in it.
#
# THE WAYS TO GET THIS WRONG all produce a cask that installs successfully.
#
#   `binary "#{appdir}/Brightfield.app/Contents/MacOS/brightfield"` links the
#   app's executable into bin. `std::env::current_exe` on macOS returns the
#   path the process was invoked with, unresolved, so the binary reached
#   through the link reports bin as its own directory and finds no `finetype/`
#   there. Measured against the v0.1.8 app binary: through a symlink,
#   `--check-type-source` reported no type source and exited 2; through the
#   shim, exit 0. The cask must link a shim that execs the real path.
#
#   A shim that execs a relative path, or the bare `brightfield`, runs
#   whatever the shell finds first, or nothing, instead of the app's binary.
#   It must exec the absolute `{{appdir}}/Brightfield.app/Contents/MacOS/
#   brightfield`.
#
#   A cask without the formula refusal in `preflight_steps` fails an upgrade
#   from the old formula on a link conflict that does not name the cause. A
#   `preflight do` block is the way Homebrew 7 deprecates, and `conflicts_with
#   formula:` it removed, so neither may stand in for it.
#
#   A url without `#{arch}` fetches one disk image for both architectures.
#
# THIS READS THE CASK AS TEXT, comments removed, and not the installed files,
# because every failure above puts the files in the right place. What is wrong
# is which path the process runs from, and no file list shows that.
set -euo pipefail

CASK="${1:-}"
[ -n "$CASK" ] || { echo "usage: scripts/check-cask-layout.sh CASK.rb" >&2; exit 2; }

fail() { echo "check-cask-layout: $*" >&2; exit 1; }

[ -f "$CASK" ] || { echo "check-cask-layout: no cask at ${CASK}" >&2; exit 2; }

# The cask without comment lines. Reading the whole file would let a comment
# satisfy any requirement below, a mistake this repository has already made
# once in a workflow scan. A shebang line is not a comment here, and is kept.
BODY=$({ grep -vE '^[[:space:]]*#([^!]|$)' "$CASK" || true; })
# `|| true` because grep exits 1 when every line was a comment, and under
# `set -o pipefail` that would kill this script before it could say so.

[ -n "$(printf '%s' "$BODY" | tr -d '[:space:]')" ] \
  || fail "${CASK} holds nothing but comments"

has() { printf '%s\n' "$BODY" | grep -Eq -- "$1"; }

# The known-wrong shapes first, so the refusal names the specific mistake
# rather than the general requirement it happens to break.
if has '^[[:space:]]*binary[[:space:]].*Contents/MacOS'; then
  fail "${CASK} links the app's executable into bin/ with a binary stanza. current_exe()
  on macOS returns the path the process was invoked with, so the binary reports bin/
  as its own directory and looks for bin/finetype. Link an exec shim instead."
fi

if has '^[[:space:]]*preflight[[:space:]]+do'; then
  fail "${CASK} uses a 'preflight do' block, which Homebrew 7 deprecates. Use preflight_steps."
fi

has '^[[:space:]]*app[[:space:]]+"Brightfield\.app"[[:space:]]*$' \
  || fail "${CASK} has no 'app \"Brightfield.app\"' stanza, so the notarised app is not installed"

has '^[[:space:]]*binary[[:space:]]+"brightfield\.wrapper\.sh",[[:space:]]*target:[[:space:]]*"brightfield"[[:space:]]*$' \
  || fail "${CASK} does not link the exec shim as bin/brightfield (binary \"brightfield.wrapper.sh\", target: \"brightfield\")"

# Every exec in the cask is the shim's, and each must be the absolute path of
# the binary inside the installed app.
EXECS=$(printf '%s\n' "$BODY" | grep -E '^[[:space:]]*exec[[:space:]]' || true)
[ -n "$EXECS" ] \
  || fail "${CASK} writes no shim that execs the app's binary. A symlink in bin/ is not
  equivalent: current_exe() on macOS does not resolve one, so the binary finds no finetype/."
# grep -v succeeds when some exec line is NOT the absolute one, and that is the refusal.
printf '%s\n' "$EXECS" \
  | grep -Eqv "^[[:space:]]*exec[[:space:]]+'\{\{appdir\}\}/Brightfield\.app/Contents/MacOS/brightfield'([[:space:]]|\$)" \
  && fail "${CASK}'s shim execs something other than the absolute path
  {{appdir}}/Brightfield.app/Contents/MacOS/brightfield. A relative path or a bare
  'brightfield' runs whatever the shell finds, not the app's binary."

# The preflight's own lines, and only those: the refusal, the shim and its
# permissions must be inside the block, where Homebrew runs them. The block ends
# at the first `end` indented as `preflight_steps` is, so heredoc text inside it
# cannot close it early.
PRE=$(printf '%s\n' "$BODY" | awk '
  /^[[:space:]]*preflight_steps[[:space:]]+do[[:space:]]*$/ { inside = 1; next }
  inside && /^  end[[:space:]]*$/                          { inside = 0 }
  inside                                                   { print }')
[ -n "$PRE" ] \
  || fail "${CASK} has no preflight_steps block, so nothing refuses an install over the old formula"
pre_has() { printf '%s\n' "$PRE" | grep -Eq -- "$1"; }

pre_has '/Cellar/brightfield/' \
  || fail "${CASK}'s preflight_steps does not test whether bin/brightfield resolves into the
  formula's Cellar (*/Cellar/brightfield/*), so nothing detects the old formula"
pre_has 'brew uninstall --formula brightfield' \
  || fail "${CASK}'s preflight_steps does not refuse an install over the old formula by
  naming 'brew uninstall --formula brightfield'"
pre_has '^[[:space:]]*write_file[[:space:]]+"brightfield\.wrapper\.sh"' \
  || fail "${CASK}'s preflight_steps does not write_file the shim brightfield.wrapper.sh"
pre_has '^[[:space:]]*set_permissions[[:space:]]+"brightfield\.wrapper\.sh",[[:space:]]*"0755"' \
  || fail "${CASK}'s preflight_steps does not set_permissions the shim to 0755, so bin/brightfield
  fails with 'permission denied'"

has '^[[:space:]]*url[[:space:]].*v#\{version\}' \
  || fail "${CASK}'s url does not contain v#{version}, so the release tag is not what is fetched"
has '^[[:space:]]*url[[:space:]].*#\{arch\}' \
  || fail "${CASK}'s url does not contain #{arch}, so both architectures fetch the same disk image"

echo "check-cask-layout: ${CASK} installs the app, execs its binary by absolute path from bin, and refuses an install over the old formula."
