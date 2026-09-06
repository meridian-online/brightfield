#!/usr/bin/env bash
# Write the Homebrew formula the tap serves, onto stdout.
#
#   scripts/write-brightfield-formula.sh TAG BASE_URL ARM_SHA256 INTEL_SHA256
#
# THIS FILE IS THE FORMULA'S SOURCE OF TRUTH. A hand edit to
# Formula/brightfield.rb in meridian-online/homebrew-tap is overwritten by the
# next release, because .github/workflows/release.yml regenerates the file
# wholesale from here. Changes to the formula belong in this script.
#
# IT IS A SCRIPT RATHER THAN A HEREDOC INSIDE THE WORKFLOW, and the reason is
# the same one that runs through this whole area: a heredoc in YAML can only be
# exercised by pushing a tag. Anything wrong with it is discovered on a
# release, in a repository with no CI of its own, by a stranger's `brew
# install`. As a script it is run by scripts/check-formula-layout-selftest.sh
# on every pull request.
#
# WHY libexec AND AN EXEC SCRIPT RATHER THAN `bin.install "brightfield"`.
#
# The tarball carries `finetype/` — the semantic type classifier — beside the
# executable, and `brightfield_engine::semantic::bundle_beside` looks for it at
# `<exe dir>/finetype` and `<exe dir>/../Resources/finetype`. `bin.install
# "brightfield"` takes the executable and discards everything around it, so the
# installed binary has no bundle beside it and reports storage types for every
# column. Installing the whole tree into `libexec` keeps the layout intact.
#
# `bin.write_exec_script` and NOT `bin.install_symlink`, and this is the part
# that looks interchangeable and is not. `std::env::current_exe` on macOS
# returns the path the process was invoked with, unresolved —
# `_NSGetExecutablePath` does not follow the link — so a binary reached through
# a symlink in `bin` reports `bin` as its own directory and looks for
# `bin/finetype`, which is not there. Measured against the installed
# brightfield 0.1.4: the same binary with the same bundle beside it finds the
# bundle when invoked by its real path and reports none when invoked through an
# identical symlink. An exec script is a wrapper that `exec`s the libexec
# binary by absolute path, so the process's own path is the real one.
#
# A formula using a symlink passes a check that the bundle directory is
# installed and fails the thing the bundle is for, which is why
# scripts/check-formula-layout.sh reads the install block rather than the
# Cellar's file list.
set -euo pipefail

TAG="${1:?usage: scripts/write-brightfield-formula.sh TAG BASE_URL ARM_SHA256 INTEL_SHA256}"
BASE="${2:?usage: scripts/write-brightfield-formula.sh TAG BASE_URL ARM_SHA256 INTEL_SHA256}"
ARM_SHA="${3:?usage: scripts/write-brightfield-formula.sh TAG BASE_URL ARM_SHA256 INTEL_SHA256}"
INTEL_SHA="${4:?usage: scripts/write-brightfield-formula.sh TAG BASE_URL ARM_SHA256 INTEL_SHA256}"

# A checksum that is not a digest reaches a stranger's `brew install` as a
# mismatch rather than a red release run, so it stops here.
for pair in "arm:${ARM_SHA}" "intel:${INTEL_SHA}"; do
  printf '%s' "${pair#*:}" | grep -Eq '^[0-9a-f]{64}$' || {
    echo "write-brightfield-formula: the ${pair%%:*} checksum is not a sha256: '${pair#*:}'" >&2
    exit 1; }
done

cat <<FORMULA
class Brightfield < Formula
  desc "Grammar-of-graphics renderer for Meridian data (macOS)"
  homepage "https://github.com/meridian-online/brightfield"
  license "MIT"

  depends_on :macos

  on_macos do
    if Hardware::CPU.arm?
      url "${BASE}/brightfield-${TAG}-aarch64-apple-darwin.tar.gz"
      sha256 "${ARM_SHA}"
    else
      url "${BASE}/brightfield-${TAG}-x86_64-apple-darwin.tar.gz"
      sha256 "${INTEL_SHA}"
    end
  end

  # The whole tarball, not the binary alone. finetype/ is the semantic type
  # classifier and it has to stay beside the executable; an exec script rather
  # than a symlink because current_exe() on macOS does not resolve one, so a
  # symlinked binary looks for the bundle in bin/ and finds nothing.
  def install
    libexec.install Dir["*"]
    bin.write_exec_script libexec/"brightfield"
  end

  test do
    assert_match "brightfield", shell_output("#{bin}/brightfield --version")
    # Exits 0 only when the bundled extension loaded, the model beside it
    # loaded, and a column got a label; shell_output raises on anything else,
    # and exit 2 is "no bundle found" — what an install that dropped the tree
    # around the binary looks like. The path it reports is the assertion:
    # libexec means current_exe() resolved to the real binary, which is the
    # whole reason this formula writes an exec script instead of a symlink.
    assert_match "libexec/finetype", shell_output("#{bin}/brightfield --check-type-source")
  end
end
FORMULA
