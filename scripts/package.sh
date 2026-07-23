#!/usr/bin/env bash
# Package the single-binary distribution: one native executable, no runtime
# dependencies beyond the OS graphics stack.
#
#   scripts/package.sh [VERSION] [TARGET]
#
#   VERSION  asset version label, e.g. v0.2.0 (default: v<crate>-local)
#   TARGET   Rust target triple (default: the host)
#
# Produces, under dist/:
#
#   brightfield-<VERSION>-<TARGET>.tar.gz          the distributable
#   brightfield-<VERSION>-<TARGET>.tar.gz.sha256   its checksum sidecar
#
# — the same asset naming the install.meridian.online convention consumes
# (`<tool>-<version>-<target>.tar.gz` + `.sha256` per asset), so the tag-driven
# release workflow and this script are the same packaging, not two.
#
# The tarball contains:
#   brightfield     the binary (built from the brightfield-shell crate)
#   LICENSE
#   README.txt      what it is, how to open the bundled examples
#   examples/       the self-contained specs (inline data, no network) plus the
#                   vendored Protocol manifests — everything the air-gapped
#                   smoke test (scripts/verify-airgapped.sh) needs is inside
#                   the artifact it tests. examples/live/ is deliberately NOT
#                   shipped: it needs a DuckDB extension download, which is
#                   exactly what a sealed artifact must not depend on.
#
# After staging, the script audits the binary's linked libraries (otool -L /
# ldd) against an OS allowlist and fails on anything else — the mechanical form
# of "no runtime deps beyond graphics drivers". (On macOS and Linux the GPU
# stack is reached via OS frameworks / dlopen at runtime, so it never appears
# as a link-time dependency; anything non-OS that does appear is a packaging
# regression.)
set -euo pipefail
cd "$(dirname "$0")/.."

# The repo's exact toolchain pin (the same one CI nails via its toolchain
# action; there is no rust-toolchain.toml). Overridable, but the default must
# not be "whatever cargo the shell finds" — the workspace floor is above some
# installed defaults, and a release artifact should be built by the pinned
# compiler, not by luck.
export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-1.95.0}"

CRATE_VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' crates/brightfield-shell/Cargo.toml | head -1)
VERSION="${1:-v${CRATE_VERSION}-local}"
HOST=$(rustc -vV | sed -n 's/^host: //p')
TARGET="${2:-$HOST}"

NAME="brightfield-${VERSION}-${TARGET}"
STAGE="dist/${NAME}"

echo "== build (release, locked): ${TARGET}"
BUILD=(cargo build --release --locked -p brightfield-shell --bin brightfield-shell)
BIN="target/release/brightfield-shell"
if [ "$TARGET" != "$HOST" ]; then
  BUILD+=(--target "$TARGET")
  BIN="target/${TARGET}/release/brightfield-shell"
fi
"${BUILD[@]}"

echo "== stage: ${STAGE}"
rm -rf "$STAGE"
mkdir -p "$STAGE/examples"
# Distributed under the product's name; the crate keeps its own.
cp "$BIN" "$STAGE/brightfield"
cp LICENSE "$STAGE/LICENSE"
cp examples/*.yaml "$STAGE/examples/"
cp -R examples/protocol "$STAGE/examples/protocol"
cat > "$STAGE/README.txt" <<EOF
brightfield ${VERSION} (${TARGET})

One native binary. No server, no webview, no language runtime, no network —
the only thing it asks of the machine is a working graphics driver.

Run it on a bundled example:

  ./brightfield examples/bars.yaml

Open a Protocol manifest (rendered from the manifest alone, no run):

  BRIGHTFIELD_PROTOCOL_OFFLINE=1 ./brightfield examples/protocol/edgar_gleif/arcform.yaml

Unattended smoke test (renders, screenshots itself, exits; exit 0 means the
PNG landed):

  ./brightfield examples/bars.yaml --shot-after 45 --shot-out smoke.png

Flags: [SPEC.yaml] [--theme light|dark] [--flow vertical|horizontal]
       [--shot-out PATH] [--shot-after N]
EOF

echo "== audit: linked libraries"
audit_ok=1
case "$TARGET" in
  *-apple-darwin)
    # otool reads any Mach-O regardless of host arch. Skip the first line
    # (the file's own name); allow OS library dirs only.
    while IFS= read -r line; do
      lib=$(echo "$line" | awk '{print $1}')
      case "$lib" in
        /usr/lib/*|/System/Library/*) ;;
        *) echo "   NON-OS LINKAGE: $lib"; audit_ok=0 ;;
      esac
    done < <(otool -L "$STAGE/brightfield" | tail -n +2)
    ;;
  *-linux-*)
    if [ "$TARGET" = "$HOST" ]; then
      while IFS= read -r line; do
        lib=$(echo "$line" | awk '{print $1}')
        case "$lib" in
          linux-vdso*|ld-linux*|/lib*/ld-linux*) ;;
          libc.so*|libm.so*|libdl.so*|libpthread.so*|librt.so*|libgcc_s.so*|libutil.so*) ;;
          *) echo "   NON-OS LINKAGE: $line"; audit_ok=0 ;;
        esac
      done < <(ldd "$STAGE/brightfield")
    else
      echo "   (cross build: ldd audit needs the target OS — run it there)"
    fi
    ;;
  *)
    echo "   (no audit rule for ${TARGET})"
    ;;
esac
[ "$audit_ok" -eq 1 ] || { echo "audit FAILED: the binary links non-OS libraries"; exit 1; }
echo "   clean: OS libraries only"

echo "== archive"
tar -czf "dist/${NAME}.tar.gz" -C dist "$NAME"
(cd dist && shasum -a 256 "${NAME}.tar.gz" > "${NAME}.tar.gz.sha256")
rm -rf "$STAGE"

echo "== done"
ls -lh "dist/${NAME}.tar.gz" "dist/${NAME}.tar.gz.sha256"
echo "verify air-gapped: scripts/verify-airgapped.sh dist/${NAME}.tar.gz"
