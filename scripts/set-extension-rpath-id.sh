#!/usr/bin/env bash
# set-extension-rpath-id.sh EXT — give a DuckDB loadable extension an `@rpath/`
# install name, in place, keeping its metadata trailer byte for byte.
#
# WHY: the install name is fixed before the manifest is written, so the manifest
# in `bundle-manifest.sha256` records the file that ships and the application
# does not refuse it as "not the file that was packaged". It was chosen when a
# Homebrew formula unpacked the tarball into a Cellar: Homebrew rewrites the
# install name of every dylib it installs to an absolute path under its own
# prefix and re-signs the file, unless the name starts with `@rpath` and the
# formula declares `preserve_rpath`. The cask installs the app as built, so
# nothing rewrites the name now. It stays `@rpath/…` so the manifest matches
# whatever installs the bundle, a relocating installer included.
#
# The install name is never read when the extension loads: DuckDB opens it by
# path. Only a program linking against the dylib would use it, and none does.
#
# WHY NOT `install_name_tool` ON THE FILE: DuckDB appends its metadata trailer
# after the Mach-O, and install_name_tool refuses a file whose __LINKEDIT
# segment does not reach its end. So the trailer is cut off at the end of
# __LINKEDIT, the name changed, the file re-signed ad hoc (a changed load
# command invalidates the signature, and arm64 will not load an unsigned
# dylib), and the trailer put back. The trailer is compared before and after:
# scripts/check-bundled-extension.sh and DuckDB's LOAD both read it.
#
# Idempotent: an extension whose name already starts with `@rpath/` is left
# as it is. Exit 0 done, 1 refused (not a dylib, no trailer, a tool failed).
set -euo pipefail

EXT="${1:?usage: scripts/set-extension-rpath-id.sh EXTENSION}"
fail() { echo "set-extension-rpath-id: $*" >&2; exit 1; }

[ -f "$EXT" ] || fail "no file at ${EXT}"
for tool in otool install_name_tool codesign; do
  command -v "$tool" >/dev/null 2>&1 || fail "$tool is not on PATH (it is a macOS developer tool)"
done

# `otool -D` prints the file name, then the install name; a file that is not a
# dylib prints the file name alone.
old_id=$(otool -D "$EXT" | sed -n 2p)
[ -n "$old_id" ] || fail "${EXT} has no install name — it is not a Mach-O dylib"
case "$old_id" in
  @rpath/*)
    echo "   ${EXT##*/}: install name already ${old_id}"
    exit 0
    ;;
esac
new_id="@rpath/${old_id##*/}"

# Everything after the end of the Mach-O is the trailer; the helper says where
# that is, for this script and scripts/split-extension.sh alike.
# shellcheck source=scripts/lib/extension-macho-end.sh
. "$(dirname "$0")/lib/extension-macho-end.sh"
linkedit_end=$(extension_macho_end "$EXT")
size=$(wc -c < "$EXT" | tr -d ' ')
[ -n "$linkedit_end" ] || fail "${EXT} has no __LINKEDIT segment"
trailer_len=$((size - linkedit_end))
[ "$trailer_len" -ge 512 ] \
  || fail "${EXT} carries ${trailer_len} bytes after __LINKEDIT — shorter than DuckDB's metadata trailer"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
head -c "$linkedit_end" "$EXT" > "$work/macho"
tail -c "$trailer_len" "$EXT" > "$work/trailer"

install_name_tool -id "$new_id" "$work/macho" || fail "install_name_tool could not rename ${EXT}"
codesign --force --sign - "$work/macho" 2>/dev/null || fail "codesign could not re-sign ${EXT}"
cat "$work/macho" "$work/trailer" > "$work/out"

got_id=$(otool -D "$work/out" | sed -n 2p)
[ "$got_id" = "$new_id" ] || fail "${EXT} reads '${got_id}' after the rename, not '${new_id}'"
tail -c "$trailer_len" "$work/out" | cmp -s - "$work/trailer" \
  || fail "${EXT}'s metadata trailer changed in the rename"

cat "$work/out" > "$EXT"
echo "   ${EXT##*/}: install name ${old_id} -> ${new_id}, re-signed ad hoc, trailer kept"
