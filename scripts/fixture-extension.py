#!/usr/bin/env python3
"""Write a fixture DuckDB loadable extension: a body plus a metadata trailer.

    scripts/fixture-extension.py OUT PLATFORM DUCKDB_VERSION EXT_VERSION ABI

Not an extension anything can LOAD — the body is filler. It carries a real
trailer, which is what `scripts/check-bundled-extension.sh` and
`brightfield_engine::semantic::read_stamp` read, so the guards over a bundle
can be exercised on every pull request with no network and no 17 MB model.
Nothing here needs a compiler either, though
scripts/check-artifact-type-source-selftest.sh — one of the two self-tests
building fixtures with this — does, for its own reason: it reads the host
triple from `rustc -vV` so as not to ask the code under test what the host is.

The trailer is the last 512 bytes: eight 32-byte NUL-padded ASCII fields
written LAST-FIRST, then 256 bytes of signature space. Field order therefore
puts the magic ("4") at offset 224 and the platform, DuckDB version, extension
version and ABI below it.

Written in python rather than printf/dd because the fields are NUL-padded and
a shell variable cannot carry a NUL. It lives here, shared, because three
self-tests need the same trailer and three copies of these offsets is how one
of them ends up testing a shape the check does not read.
"""

import sys


def main() -> int:
    if len(sys.argv) != 6:
        print(__doc__.strip().splitlines()[2].strip(), file=sys.stderr)
        return 2
    out, platform, duckdb_version, ext_version, abi = sys.argv[1:6]

    def pad(s: str) -> bytes:
        raw = s.encode()
        if len(raw) > 32:
            raise SystemExit(f"fixture-extension: {s!r} does not fit a 32-byte field")
        return raw.ljust(32, b"\0")

    body = b"a plausible shared library body" * 40
    trailer = b"\0" * 96
    trailer += pad(abi) + pad(ext_version) + pad(duckdb_version) + pad(platform) + pad("4")
    trailer += b"\0" * 256
    assert len(trailer) == 512, len(trailer)
    with open(out, "wb") as fh:
        fh.write(body + trailer)
    return 0


if __name__ == "__main__":
    sys.exit(main())
