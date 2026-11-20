# shellcheck shell=bash
# extension_macho_end EXT — print the byte offset where a DuckDB loadable
# extension's Mach-O ends, and print nothing when it has no __LINKEDIT segment.
#
# Sourced, not run. DuckDB appends its metadata trailer after the Mach-O, and
# two scripts cut the file there: scripts/set-extension-rpath-id.sh, which has to
# take the trailer off before install_name_tool will touch the file, and
# scripts/split-extension.sh, which ships the two halves as separate files
# because codesign cannot sign the joined one. The offset is read here so the
# two cannot disagree about where the trailer starts.
#
# Where __LINKEDIT ends is where the Mach-O ends; everything after it is the
# trailer. The linker writes __LINKEDIT as the last segment in the file.
extension_macho_end() {
  otool -l "$1" | awk '
    /segname __LINKEDIT/ { inside = 1; next }
    inside && $1 == "fileoff"  { off = $2 }
    inside && $1 == "filesize" { print off + $2; exit }
  '
}
