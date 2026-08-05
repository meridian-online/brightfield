#!/usr/bin/env bash
# Regenerate packaging/Brightfield.icns from packaging/Brightfield.svg.
#
#   packaging/make-icns.sh
#
# The .icns is vendored (scripts/package.sh copies it into the bundle), so this
# script is not part of the build and CI never runs it. It is here because the
# alternative to a script is a paragraph describing what somebody once did in a
# terminal, and a paragraph cannot be re-run.
#
# Needs rsvg-convert (librsvg) for the rasterisation and iconutil, which ships
# with macOS, for the container. Both are developer-machine tools.
#
# The size set below is the one `iconutil -c icns` accepts: each entry is a
# <pixels>:<iconset filename> pair, and the @2x names are what makes an icon
# render sharp on a Retina display.
set -euo pipefail
cd "$(dirname "$0")"

command -v rsvg-convert >/dev/null || {
  echo "make-icns.sh: rsvg-convert not found (brew install librsvg)" >&2
  exit 1
}
command -v iconutil >/dev/null || {
  echo "make-icns.sh: iconutil not found — this script needs macOS" >&2
  exit 1
}

SIZES=(
  16:icon_16x16.png
  32:icon_16x16@2x.png
  32:icon_32x32.png
  64:icon_32x32@2x.png
  128:icon_128x128.png
  256:icon_128x128@2x.png
  256:icon_256x256.png
  512:icon_256x256@2x.png
  512:icon_512x512.png
  1024:icon_512x512@2x.png
)

WORK=$(mktemp -d "${TMPDIR:-/tmp}/bf-icns.XXXXXX")
trap 'rm -rf "$WORK"' EXIT
SET="$WORK/Brightfield.iconset"
mkdir -p "$SET"

for entry in "${SIZES[@]}"; do
  px="${entry%%:*}"
  out="${entry#*:}"
  rsvg-convert -w "$px" -h "$px" Brightfield.svg -o "$SET/$out"
done

iconutil -c icns "$SET" -o Brightfield.icns
echo "wrote $(pwd)/Brightfield.icns"
ls -l Brightfield.icns
