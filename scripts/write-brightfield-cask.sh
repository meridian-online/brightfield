#!/usr/bin/env bash
# Write the Homebrew cask the tap serves, onto stdout.
#
#   scripts/write-brightfield-cask.sh TAG BASE_URL ARM_SHA256 INTEL_SHA256
#
# Exit 0 on success, 2 on a usage error: a missing argument, a TAG that is not
# `v<digit>...`, a checksum that is not a sha256, or a BASE_URL that does not
# end in `/<TAG>`.
#
# THIS FILE IS THE CASK'S SOURCE OF TRUTH. A hand edit to Casks/brightfield.rb
# in meridian-online/homebrew-tap is overwritten by the next release, because
# .github/workflows/release.yml regenerates the file wholesale from here.
# Changes to the cask belong in this script.
#
# IT IS A SCRIPT RATHER THAN A HEREDOC INSIDE THE WORKFLOW for the reason that
# runs through this whole area: a heredoc in YAML can only be exercised by
# pushing a tag, so anything wrong with it is discovered on a release, in a
# repository with no CI of its own, by a stranger's `brew install`. As a script
# it is run by scripts/check-cask-layout-selftest.sh on every pull request.
#
# WHY A CASK. The release ships a notarised Brightfield.app in a disk image.
# A cask installs the app whole, so its signature and notarisation ticket stay
# intact; a formula would have had to unpack the app into a Cellar, where
# Homebrew relocates what it installs and would invalidate the signature.
#
# THE URL IS WRITTEN WITH `#{version}` AND `#{arch}` LEFT FOR RUBY. BASE_URL is
# the release download base for TAG, ending in `/<TAG>`; the script strips that
# tail and writes the url as `<base>/v#{version}/brightfield-v#{version}-
# #{arch}-apple-darwin.dmg`, so the cask names its assets by the same
# `brightfield-<tag>-<arch>-apple-darwin.dmg` pattern the release uploads and
# `brew livecheck` and `brew bump-cask-pr` can rewrite only `version`. If a
# later edit loses `#{arch}`, both architectures fetch the same disk image;
# scripts/check-cask-layout.sh refuses that.
#
# WHY AN EXEC SHIM RATHER THAN `binary "Brightfield.app/Contents/MacOS/..."`.
#
# The app bundles `finetype/`, the semantic type classifier, and
# `brightfield_engine::semantic::bundle_beside` looks for it relative to the
# executable's own directory. `std::env::current_exe` on macOS returns the path
# the process was invoked with, unresolved (`_NSGetExecutablePath` does not
# follow a link), so a binary reached through a symlink in Homebrew's bin
# reports bin as its own directory and finds no type source. Measured against
# the v0.1.8 app binary: through a symlink, `--check-type-source` reported no
# type source and exited 2; through the shim below, it exited 0. The shim
# `exec`s the binary inside the app by its absolute path, so the process's own
# path is the real one. `{{appdir}}` is Homebrew's placeholder for the
# directory the app was installed to, which is why it is written literally and
# not expanded here. The cask's `binary` stanza links the shim, not the app's
# executable.
#
# THE PREFLIGHT REFUSES AN INSTALL OVER THE OLD FORMULA. The formula this cask
# replaces also owned `bin/brightfield`, and Homebrew would fail on the link
# with a message that does not name the cause. The check resolves the link and
# refuses when it points into the formula's Cellar, naming the command that
# removes it. It uses `preflight_steps`, not a `preflight do` Ruby block, which
# Homebrew 7 deprecates, and not `conflicts_with formula:`, which it removed.
# The heredocs below reach the cask verbatim: `$link`, `$@` and `{{appdir}}`
# are literal in the cask, which is why the script writes them from quoted
# heredocs.
set -euo pipefail

USAGE="usage: scripts/write-brightfield-cask.sh TAG BASE_URL ARM_SHA256 INTEL_SHA256"
usage() { echo "write-brightfield-cask: $*" >&2; exit 2; }

[ "$#" -eq 4 ] || usage "$USAGE"
TAG="$1"
BASE="$2"
ARM_SHA="$3"
INTEL_SHA="$4"

printf '%s' "$TAG" | grep -Eq '^v[0-9]' \
  || usage "the tag must look like v0.1.8, not '${TAG}'"
VERSION="${TAG#v}"

# A checksum that is not a digest reaches a stranger's `brew install` as a
# mismatch rather than a red release run, so it stops here.
for pair in "arm:${ARM_SHA}" "intel:${INTEL_SHA}"; do
  printf '%s' "${pair#*:}" | grep -Eq '^[0-9a-f]{64}$' \
    || usage "the ${pair%%:*} checksum is not a sha256: '${pair#*:}'"
done

case "$BASE" in
  */"$TAG") URL_BASE="${BASE%/"$TAG"}" ;;
  *) usage "BASE_URL must end in /${TAG}, got '${BASE}'" ;;
esac

# The part with values in it: unquoted, so the shell fills the version, the
# checksums and the url base. `#{version}` and `#{arch}` pass through as Ruby.
cat <<CASK
cask "brightfield" do
  arch arm: "aarch64", intel: "x86_64"

  version "${VERSION}"
  sha256 arm:   "${ARM_SHA}",
         intel: "${INTEL_SHA}"

  url "${URL_BASE}/v#{version}/brightfield-v#{version}-#{arch}-apple-darwin.dmg"
CASK

# The rest: quoted, so nothing in it is expanded by the shell.
cat <<'CASK'
  name "Brightfield"
  desc "Grammar-of-graphics renderer for Meridian data"
  homepage "https://meridian.online/brightfield"

  # The releases feed lists every release, pre-release or full. Releases
  # before 0.1.9 were pre-releases, which the GitHub strategies skip.
  livecheck do
    url "https://github.com/meridian-online/brightfield/releases.atom"
    strategy :page_match
    regex(%r{/releases/tag/v?(\d+(?:\.\d+)+)["<]}i)
  end

  depends_on :macos

  app "Brightfield.app"
  binary "brightfield.wrapper.sh", target: "brightfield"

  preflight_steps do
    run "/bin/sh", args: ["-c", <<~SH]
      prefixes="/opt/homebrew /usr/local"
      [ -n "${HOMEBREW_PREFIX:-}" ] && prefixes="$HOMEBREW_PREFIX $prefixes"
      for prefix in $prefixes; do
        link="$prefix/bin/brightfield"
        case "$(readlink "$link" 2>/dev/null)" in
          */Cellar/brightfield/*)
            echo "The brightfield formula is still installed, and it owns $link." >&2
            echo "Remove it first:  brew uninstall --formula brightfield" >&2
            exit 1 ;;
        esac
      done
    SH
    write_file "brightfield.wrapper.sh", <<~SH
      #!/bin/sh
      exec '{{appdir}}/Brightfield.app/Contents/MacOS/brightfield' "$@"
    SH
    set_permissions "brightfield.wrapper.sh", "0755"
  end

  # The full edition's identifier is online.meridian.brightfield.direct.
  # Releases up to 0.1.9 used online.meridian.brightfield, whose files an
  # upgraded install still holds. FineType's cache keeps the older name.
  zap trash: [
    "~/Library/Application Support/online.meridian.brightfield",
    "~/Library/Application Support/online.meridian.brightfield.direct",
    "~/Library/Caches/online.meridian.brightfield",
    "~/Library/Caches/online.meridian.brightfield.direct",
    "~/Library/Preferences/online.meridian.brightfield.direct.plist",
    "~/Library/Preferences/online.meridian.brightfield.plist",
    "~/Library/Saved Application State/online.meridian.brightfield.direct.savedState",
    "~/Library/Saved Application State/online.meridian.brightfield.savedState",
  ]
end
CASK
