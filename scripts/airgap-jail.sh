# The read jail, sourced by scripts/verify-airgapped.sh and by
# scripts/airgap-jail-selftest.sh, so the profile the self-test proves is the
# profile the type-source leg runs under rather than a copy of it.
#
#   . scripts/airgap-jail.sh
#   read_jail_profile DIR...    print a sandbox-exec profile on stdout
#
# The profile denies the network, as the jail for every other leg does, and it
# DENIES READING THE CONTENTS OF ANY FILE outside the directories named and the
# system paths listed below. It is deny-by-default on purpose: naming the ways
# a bundle can reach outside itself has been wrong three times running, and a
# list of what may be read is right about a route nobody has found yet.
#
# Metadata is not denied, only contents (`file-read-data`). A stat can tell a
# process that a path exists; it cannot load a model or a library from it, and
# denying it would mean granting every ancestor directory of every path by
# name to get dyld started.
#
# Each directory is resolved with `pwd -P` before it is granted, because the
# sandbox matches the real path and macOS's temp directories live behind the
# /var -> /private/var symlink: a grant of the path mktemp printed would grant
# nothing, and every read would be refused for a reason that has nothing to do
# with the artefact.
#
# macOS only. Linux's jail here is `unshare -rn`, a network namespace, and has
# no equivalent for reads; scripts/verify-airgapped.sh says so when it runs
# the leg there.

# THE SYSTEM GRANTS, each one taken from recorded runs of
# `brightfield --check-type-source` out of an unpacked release tarball on
# macOS 26, not guessed. One run allowed and reported every read
# (`(allow file-read-data (with report))`, read back from the kernel log with
# `log show --predicate 'sender == "Sandbox"'`); a second ran under this
# profile and reported what it refused. Outside the artefact and its sealed
# directory, those runs read the contents of:
#
# - `/`, as a directory, which dyld reads as it starts. The reporting run's log
#   coalesced it away; the run under this profile without it aborted (134) and
#   named it.
# - `/System` — dyld's shared cache, under /System/Volumes/Preboot/Cryptexes.
# - `/usr/lib`, as a directory. The OS libraries the binary links are in the
#   shared cache; scripts/package.sh audits that it links nothing else.
# - `/dev/autofs_nowait` and `/dev/dtracehelper`, which dyld and libSystem open.
#
# And one path it is REFUSED and does without: `~/.CFUserTextEncoding`, which
# CoreFoundation opens through the account's home directory rather than
# `$HOME` and falls back from when it cannot. Nothing in it can change a label.
#
# Both layouts ran under this profile on macOS 26 and passed: the tarball's
# binary and the disk image's Brightfield.app. The release's verify job runs on
# macos-15, where this profile has not yet run the binary; a path macOS 15
# needs and 26 did not fails that job closed, at run 0.
#
# A machine whose OS reads another system path fails the leg, and the kernel
# log names the path it refused. Record a run before adding it here, rather
# than widening a grant to make the leg pass.
READ_JAIL_SYSTEM_SUBPATHS=(
  /System
)
READ_JAIL_SYSTEM_LITERALS=(
  /
  /usr/lib
  /dev/autofs_nowait
  /dev/dtracehelper
)

read_jail_profile() {
  [ "$#" -gt 0 ] || { echo "read_jail_profile: name at least one directory to grant" >&2; return 1; }
  local grants="" dir real
  for dir in "$@"; do
    real=$(cd "$dir" && pwd -P) || { echo "read_jail_profile: no such directory: $dir" >&2; return 1; }
    # A quote or backslash would end the profile's string early and change
    # what it grants.
    case "$real" in *'"'*|*'\'*) echo "read_jail_profile: cannot grant a path with a quote or backslash: $real" >&2; return 1 ;; esac
    grants+=" (subpath \"${real}\")"
  done
  for dir in "${READ_JAIL_SYSTEM_SUBPATHS[@]}"; do grants+=" (subpath \"${dir}\")"; done
  for dir in "${READ_JAIL_SYSTEM_LITERALS[@]}"; do grants+=" (literal \"${dir}\")"; done
  printf '(version 1)(allow default)(deny network*)(deny file-read-data (require-not (require-any%s)))\n' "$grants"
}
