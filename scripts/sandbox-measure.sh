#!/usr/bin/env bash
# Measure what App Sandbox does to Brightfield, ad-hoc signed, with no Apple
# enrolment: the kernel applies the sandbox from the entitlements in the
# signature, whoever signed it.
#
#   scripts/sandbox-measure.sh baseline [BINARY]   today's build in .app variants
#   scripts/sandbox-measure.sh premises            the bookmark probe
#
# Results print as tab-separated rows. docs/app-sandbox.md records a run and
# says what each row means. The subject Protocol is copied outside the repo,
# to ~/Library/Caches/bf-sandbox-measure, so nothing tracked is touched.
#
# The baseline runs brightfield-shot, which renders offscreen, and the probe is
# background-only: neither opens a window. Each case's preferences also answer
# the default-app offer, so nothing asks macOS to change a default.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/.." && pwd)"
ents="$here/sandbox-measure/entitlements"
work="${BF_SANDBOX_WORK:-$HOME/Library/Caches/bf-sandbox-measure}"
subject="$work/edgar_gleif"
apps="$work/apps"

fresh_subject() {
    rm -rf "$subject"
    mkdir -p "$work"
    cp -R "$root/examples/protocol/edgar_gleif" "$subject"
}

# make_app NAME ENTITLEMENTS BINARY: a throwaway bundle around BINARY.
make_app() {
    local name="$1" ent="$2" bin="$3" app="$apps/$1.app"
    rm -rf "$app"
    mkdir -p "$app/Contents/MacOS"
    cp "$bin" "$app/Contents/MacOS/brightfield"
    cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>online.meridian.sandlab.$name</string>
  <key>CFBundleExecutable</key><string>brightfield</string>
  <key>CFBundlePackageType</key><string>APPL</string>
</dict></plist>
PLIST
    sed "s|@SUBJECT@|$subject|g" "$ents/$ent.plist" > "$work/$name.entitlements"
    codesign --force --sign - --entitlements "$work/$name.entitlements" "$app" 2>/dev/null
    echo "$app"
}

container_of() { echo "$HOME/Library/Containers/online.meridian.sandlab.$1/Data"; }

# shot NAME APP PATH: open PATH for eight frames, print stderr's protocol line,
# the exit status and the PNG's hash.
shot() {
    local name="$1" app="$2" path="$3" data cfg png err status line
    data="$(container_of "$name")"
    if [ "$name" = unsandboxed ]; then data="$work/unsandboxed-data"; fi
    cfg="$data/bf-config-$RANDOM"; png="${SHOT_INTO:-$data/shot.png}"
    # --version exits before any window, and its launch makes the container.
    "$app/Contents/MacOS/brightfield" --help >/dev/null 2>&1 || true
    mkdir -p "$cfg"
    # An answered first-launch question, so the default-app offer never asks macOS
    # to change which app opens CSV, Parquet or DuckDB files.
    echo '{"default_app_asked": "declined"}' > "$cfg/preferences.json"
    err="$work/$name.stderr"
    set +e
    BRIGHTFIELD_PROTOCOL_OFFLINE=1 BRIGHTFIELD_CONFIG_DIR="$cfg" \
        "$app/Contents/MacOS/brightfield" --spec "$path" --out "$png" 2>"$err"
    status=$?
    set -e
    line="$(grep -E '^protocol |Error|refused|not permitted' "$err" | head -1 || true)"
    local hash="-"
    [ -f "$png" ] && hash="$(shasum -a 256 "$png" | cut -c1-12)" && rm -f "$png"
    printf '%s\t%s\t%s\t%s\t%s\n' "$name" "${png#$work/}" "$status" "$hash" "${line:-<no line>}"
}

baseline() {
    # brightfield-shot boots the real shell and renders offscreen: no window opens.
    local bin="${1:-$root/target/release/brightfield-shot}"
    [ -x "$bin" ] || { echo "no binary at $bin; build with: cargo build --release -p brightfield-shell" >&2; exit 2; }
    fresh_subject
    mkdir -p "$apps"
    printf 'case\twritten to\texit\tpng\tdetector\n'
    local plain; plain="$(make_app unsandboxed sandbox "$bin")"
    codesign --force --sign - "$plain" 2>/dev/null # no entitlements at all
    local m="$subject/arcform.yaml"
    shot unsandboxed "$plain" "$m"                 # the reference picture
    chmod 000 "$subject/models"
    shot unsandboxed "$plain" "$m"                 # models refused, no sandbox
    chmod 755 "$subject/models"
    shot sandbox "$(make_app sandbox sandbox "$bin")" "$m"            # nothing granted
    shot file-only "$(make_app file-only file-only "$bin")" "$m"      # the file alone
    shot dir-ro "$(make_app dir-ro dir-ro "$bin")" "$m"               # the folder, read-only
    shot dir-rw "$(make_app dir-rw dir-rw "$bin")" "$m"               # the folder, read-write
    chmod 000 "$subject/models"
    shot dir-rw "$apps/dir-rw.app" "$m"                               # folder granted, models refused
    chmod 755 "$subject/models"
    SHOT_INTO="$subject/export.png" shot dir-ro "$apps/dir-ro.app" "$m"   # writing into a read-only grant
    SHOT_INTO="$subject/export.png" shot dir-rw "$apps/dir-rw.app" "$m"   # writing into a read-write grant
}

premises() {
    local src="$here/sandbox-measure/probe/main.swift" app="$apps/Probe.app" folder="$work/probe-folder"
    local data="$HOME/Library/Containers/online.meridian.sandlab.probe/Data"
    mkdir -p "$apps" "$folder"
    mkdir -p "$work/probe-control"; echo "never-granted" > "$work/probe-control/probe.txt"
    echo "granted-by-open" > "$folder/probe.txt"
    rm -rf "$app"; mkdir -p "$app/Contents/MacOS" "$app/Contents/Helpers"
    swiftc -O -o "$app/Contents/MacOS/Probe" "$src" 2>&1 | grep -v warning || true
    cp "$app/Contents/MacOS/Probe" "$app/Contents/Helpers/probe-child"
    local cli="${BF_DUCKDB_CLI:-$root/../brightfield/target/duckdb-cli/duckdb}"
    cp "$cli" "$app/Contents/Helpers/duckdb"
    cp "$cli" "$folder/duckdb"; cp "$cli" "$work/probe-control/duckdb"
    rm -f "$folder/written.csv"
    cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>online.meridian.sandlab.probe</string>
  <key>CFBundleExecutable</key><string>Probe</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSBackgroundOnly</key><true/>
  <key>CFBundleDocumentTypes</key><array><dict>
    <key>CFBundleTypeRole</key><string>Viewer</string>
    <key>LSItemContentTypes</key><array><string>public.folder</string></array>
  </dict></array>
</dict></plist>
PLIST
    codesign --force --sign - -i online.meridian.sandlab.probe.child \
        --entitlements "$ents/helper-inherit.plist" "$app/Contents/Helpers/probe-child"
    cp "$cli" "$app/Contents/Helpers/duckdb-inherit"
    codesign --force --sign - -i online.meridian.sandlab.probe.duckdb \
        --entitlements "$ents/helper-inherit.plist" "$app/Contents/Helpers/duckdb-inherit"
    codesign --force --sign - --entitlements "$ents/app.plist" "$app"
    rm -f "$data/tmp/probe.log" "$data/tmp/scoped.bookmark"
    # Premise 10 and 8: a folder handed over by `open -a`, then passed to children.
    open -W -a "$app" "$folder"
    # The relaunch case: no document, resolve the saved security-scoped bookmark.
    open -W -a "$app"
    cat "$data/tmp/probe.log"
}

case "${1:-}" in
    baseline) shift; baseline "$@" ;;
    premises) premises ;;
    *) echo "usage: $0 baseline [BINARY] | premises" >&2; exit 2 ;;
esac
