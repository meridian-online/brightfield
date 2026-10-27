#!/usr/bin/env bash
# Stage the pinned FineType bundle where a `cargo run` build looks for it.
#
#   scripts/stage-finetype-bundle-for-source-run.sh [PROFILE]
#
# PROFILE is the cargo profile directory (default: debug; `release` for a
# `--release` build). The bundle lands in `finetype/` beside the built
# executable, which is what `brightfield_engine::semantic::bundle_beside`
# checks first, so a run from source classifies columns and names their types
# the way a packaged build does. It goes under the target directory, which git
# ignores; nothing here is committed, and `cargo clean` removes it.
#
# The pin is `packaging/finetype-pin.env`; the download and its checks are
# `scripts/fetch-finetype-bundle.sh`'s. CARGO_TARGET_DIR is honoured.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)

profile=${1:-debug}
target_dir=${CARGO_TARGET_DIR:-$ROOT/target}
triple=$(rustc -vV | sed -n 's/^host: //p')
[ -n "$triple" ] || { echo "stage-finetype-bundle: rustc reports no host triple" >&2; exit 1; }

dest="$target_dir/$profile/finetype"
rm -rf "$dest"
mkdir -p "$dest"
"$HERE/fetch-finetype-bundle.sh" "$triple" "$dest"
echo "staged the pinned FineType bundle for $triple at $dest"
