#!/usr/bin/env bash
# Measure what App Sandbox does to Brightfield, ad-hoc signed, with no Apple
# enrolment: the kernel applies the sandbox from the entitlements in the
# signature, whoever signed it.
#
#   scripts/sandbox-measure.sh baseline [--package APP] [BINARY]
#                                                  a build in .app variants
#   scripts/sandbox-measure.sh premises [APP]      the bookmark probe
#
# With --package, the variants are the Mac App Store package's: each carries
# APP's Contents/Frameworks (Meridian's DuckDB library), Contents/Helpers and
# Contents/Resources, around BINARY, which is then the managed edition's
# brightfield-shot that scripts/package-store.sh builds beside the app
# (dist/store/brightfield-shot). Given APP, premises runs the package's own
# DuckDB CLI, with its library, in place of the pinned release CLI.
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
    # A run stopped between its chmod 000 and the chmod back leaves models/
    # unreadable, and rm cannot descend into it.
    [ -d "$subject/models" ] && chmod 755 "$subject/models"
    rm -rf "$subject"
    mkdir -p "$work"
    cp -R "$root/examples/protocol/edgar_gleif" "$subject"
}

# The Store package the variants are laid out from, when baseline is given one.
package=""
downloads=""

# make_app NAME ENTITLEMENTS BINARY: a throwaway bundle around BINARY.
# ENTITLEMENTS names a file in sandbox-measure/entitlements/, or is a path.
make_app() {
    local name="$1" ent="$2" bin="$3" app="$apps/$1.app" part entfile
    entfile="$ents/$2.plist"
    [ -f "$ent" ] && entfile="$ent"
    rm -rf "$app"
    mkdir -p "$app/Contents/MacOS"
    cp "$bin" "$app/Contents/MacOS/brightfield"
    if [ -n "$package" ]; then
        for part in Frameworks Helpers Resources; do
            [ -d "$package/Contents/$part" ] && cp -R "$package/Contents/$part" "$app/Contents/$part"
        done
        # Inside out, as package-store.sh signs: the libraries and helpers ad-hoc
        # again, so the variant's own seal covers them.
        if [ -d "$app/Contents/Frameworks" ]; then
            find "$app/Contents/Frameworks" -type f -name '*.dylib' -exec codesign --force --sign - {} \; 2>/dev/null
        fi
        if [ -d "$app/Contents/Helpers" ]; then
            find "$app/Contents/Helpers" -type f -perm -u+x -exec codesign --force --sign - \
                --entitlements "$ents/helper-inherit.plist" {} \; 2>/dev/null
        fi
    fi
    cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>online.meridian.sandlab.$name</string>
  <key>CFBundleExecutable</key><string>brightfield</string>
  <key>CFBundlePackageType</key><string>APPL</string>
</dict></plist>
PLIST
    sed "s|@SUBJECT@|$subject|g" "$entfile" > "$work/$name.entitlements"
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
    if [ "${1:-}" = "--package" ]; then
        package="${2:?--package needs the path of the Store package, Brightfield.app}"
        [ -d "$package/Contents/Frameworks" ] || { echo "$package has no Contents/Frameworks: not a Store package" >&2; exit 2; }
        shift 2
    fi
    # brightfield-shot boots the real shell and renders offscreen: no window opens.
    local default="$root/target/release/brightfield-shot"
    # The package's own renderer, which package-store.sh leaves beside it.
    [ -n "$package" ] && default="$(dirname "$package")/brightfield-shot"
    local bin="${1:-$default}"
    [ -x "$bin" ] || { echo "no binary at $bin; build with: cargo build --release -p brightfield-shell" >&2; exit 2; }
    fresh_subject
    # models/ is set to mode 000 below; an interrupted run puts it back.
    # A CSV is put in ~/Downloads for the Store case below; it goes too.
    downloads="$HOME/Downloads/bf-sandbox-measure"
    trap '[ -d "$subject/models" ] && chmod 755 "$subject/models"; rm -rf "$downloads"' EXIT INT TERM
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
    if [ -n "$package" ]; then
        # The Store's own entitlements, verbatim, with no grant at all: the
        # Protocol is refused, and a CSV in Downloads opens, through
        # files.downloads.read-write alone, while the same CSV elsewhere does not.
        local store; store="$(make_app store "$root/packaging/store/Brightfield.entitlements" "$bin")"
        shot store "$store" "$m"
        mkdir -p "$downloads"
        printf 'name,count\nalpha,1\nbeta,2\n' > "$downloads/downloads.csv"
        cp "$downloads/downloads.csv" "$work/not-downloads.csv"
        shot store "$store" "$downloads/downloads.csv"
        shot store "$store" "$work/not-downloads.csv"
    fi
}

premises() {
    local package="${1:-}"
    local src="$here/sandbox-measure/probe/main.swift" app="$apps/Probe.app" folder="$work/probe-folder"
    local data="$HOME/Library/Containers/online.meridian.sandlab.probe/Data"
    mkdir -p "$apps" "$folder"
    mkdir -p "$work/probe-control"; echo "never-granted" > "$work/probe-control/probe.txt"
    echo "granted-by-open" > "$folder/probe.txt"
    rm -rf "$app"; mkdir -p "$app/Contents/MacOS" "$app/Contents/Helpers"
    swiftc -O -o "$app/Contents/MacOS/Probe" "$src" 2>&1 | grep -v warning || true
    cp "$app/Contents/MacOS/Probe" "$app/Contents/Helpers/probe-child"
    local cli="${BF_DUCKDB_CLI:-$root/target/duckdb-cli/duckdb}"
    if [ -n "$package" ]; then
        cli="$package/Contents/Helpers/duckdb"
        # The package's CLI links Meridian's DuckDB library from ../Frameworks.
        cp -R "$package/Contents/Frameworks" "$app/Contents/Frameworks"
        find "$app/Contents/Frameworks" -type f -name '*.dylib' -exec codesign --force --sign - {} \; 2>/dev/null
    fi
    [ -x "$cli" ] || { echo "no DuckDB CLI at $cli: set BF_DUCKDB_CLI or run scripts/fetch-duckdb-cli.sh" >&2; exit 2; }
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
    # A folder handed over by `open -a`, then passed to children.
    open -W -a "$app" "$folder"
    # The relaunch case: no document, resolve the saved security-scoped bookmark.
    open -W -a "$app"
    cat "$data/tmp/probe.log"
}

case "${1:-}" in
    baseline) shift; baseline "$@" ;;
    premises) shift; premises "$@" ;;
    *) echo "usage: $0 baseline [--package APP] [BINARY] | premises [APP]" >&2; exit 2 ;;
esac
