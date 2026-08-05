# Brand assets — all rights reserved

**The MIT licence covering this repository does not extend to `Brightfield.icns`
or `Brightfield.svg` in this directory.**

Both are artefacts of the Meridian prime mark, and the mark is the trademark and
copyright of Meridian. All rights reserved. `Brightfield.svg` places the mark,
white, on a black plate cut to the macOS icon grid; `Brightfield.icns` is that
file rasterised by `make-icns.sh`.

This mirrors the carve-out the masters themselves carry, in
`meridian-design/brand/LICENSE-BRAND.md` in the Meridian design-system
repository: the code grant is MIT, and each non-code artefact carries the terms
that actually govern it.

`Info.plist`, `make-icns.sh` and `README.md` in this directory are source, and
the repository's MIT grant covers them as it covers the rest of the tree.

## What you may do

- Read, build and run this repository, including the packaging that copies these
  files into `Brightfield.app`.
- Reproduce the mark **unmodified** when referring to Meridian — writing about
  it, linking to it, listing it among tools you use.

## What you may not do

- Use the mark or any derivative to identify your own product, service,
  organisation or site.
- Modify, recolour, redraw or re-proportion it.
- Imply endorsement, affiliation or origin that does not exist.
- Take these two files under the MIT terms of the surrounding repository. The
  MIT grant covers the source code here. It does not cover them, and a trademark
  could not be granted by it in any case.

## Why they sit inside an MIT repository

Because `scripts/package.sh` copies the `.icns` into the application bundle it
builds, and a bundle with no icon is not the thing this repository ships. The
carve-out is the price of that, and it is stated here.

Questions: <https://github.com/meridian-online/brightfield>.
