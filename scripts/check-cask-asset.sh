#!/usr/bin/env bash
# Prove that the disk images a Homebrew cask publishes actually exist and hash
# to the checksums written beside them.
#
# The release workflow generates Casks/brightfield.rb wholesale and pushes it
# to the tap. Without this check, the checksums in that file come from the
# `.sha256` sidecars downloaded next to the assets, and nothing ever fetches
# the assets themselves — so a URL that resolved to nothing, or a checksum
# written beside the wrong URL, produces a green release and fails later on a
# stranger's machine at `brew install`. The Intel image is the one that matters
# most: it is a cross-compile no runner in this repository can execute, so no
# other step in the release touches it after packaging.
#
# What this proves and what it does not:
#
#   PROVES  the URL the cask resolves to for each architecture is a
#           downloadable asset, and that asset hashes to the sha256 the cask
#           declares for that architecture.
#   DOES NOT PROVE  that the app inside the image runs. That needs a host of
#           the target architecture; see the comment above the `update-homebrew`
#           job in .github/workflows/release.yml for where this repository
#           stands on the Intel side of that.
#
# A cask writes one url with `#{version}` and `#{arch}` in it, and one sha256
# per architecture, so there is no url/checksum adjacency to read as there was
# in a formula. This script does what `brew` does instead: it reads `version`,
# the `arch arm: "...", intel: "..."` names, the `sha256 arm: "...", intel:
# "..."` digests and the top-level `url`, expands the url once per
# architecture, and pairs each expansion with that architecture's digest. That
# addresses the exact string a `brew install` would resolve — reconstructing
# the URL from the tag instead would re-derive the same mistake the generator
# made and agree with it. The url inside `livecheck do ... end` is the release
# feed, not a download, and is skipped.
#
# Usage:
#
#   scripts/check-cask-asset.sh [--min-pairs N] <path/to/cask.rb>
#
# --min-pairs N (default 1) is the refusal-to-pass-on-zero guard: a cask that
# lost an architecture would otherwise check the one that remains and report
# clean. The release workflow passes 2, one per darwin target. A url that lost
# `#{arch}` is refused for the same reason: it expands to the same asset for
# both architectures, so it would fetch the arm image twice and call the Intel
# one checked.
#
# Exit codes:
#   0  every url/sha256 pair in the file fetched and matched
#   1  an asset did not resolve, or its digest disagreed with the cask
#   2  the check could not run — bad usage, an unreadable or unparseable
#      cask, fewer pairs than --min-pairs, two architectures that expand to the
#      same url, or no sha256 tool on PATH. ALWAYS a hard failure: a check that
#      cannot run must never look like one that passed.
#
# Its own regression test is scripts/check-cask-asset-selftest.sh, which runs
# on every pull request via .github/workflows/public-hygiene.yml. This script
# needs published release assets, so it can only run on a tag; the self-test is
# what keeps it honest in between.
set -uo pipefail

usage() {
	cat >&2 <<'USAGE'
usage: check-cask-asset.sh [--min-pairs N] <path/to/cask.rb>

Fetches each architecture's url in the cask and compares its sha256 to the
checksum the cask declares for it. Exit 0 clean, 1 a missing or mismatched
asset, 2 the check could not run.
USAGE
}

MIN_PAIRS=1

while [[ $# -gt 0 ]]; do
	case "$1" in
	--min-pairs)
		if [[ $# -lt 2 ]]; then
			echo "check-cask-asset: --min-pairs needs a value" >&2
			exit 2
		fi
		MIN_PAIRS="$2"
		shift 2
		;;
	--min-pairs=*)
		MIN_PAIRS="${1#*=}"
		shift
		;;
	-h | --help)
		usage
		exit 2
		;;
	--)
		shift
		break
		;;
	-*)
		echo "check-cask-asset: unknown option: $1" >&2
		usage
		exit 2
		;;
	*)
		break
		;;
	esac
done

if [[ ! "$MIN_PAIRS" =~ ^[0-9]+$ ]] || [[ "$MIN_PAIRS" -lt 1 ]]; then
	echo "check-cask-asset: --min-pairs must be a positive integer, got '$MIN_PAIRS'" >&2
	exit 2
fi

if [[ $# -ne 1 ]]; then
	usage
	exit 2
fi

CASK="$1"

if [[ ! -f "$CASK" ]]; then
	echo "check-cask-asset: no such cask: $CASK" >&2
	exit 2
fi

command -v curl >/dev/null 2>&1 || {
	echo "check-cask-asset: curl is not on PATH" >&2
	exit 2
}

# shasum on macOS, sha256sum on most Linux images; both are present on the
# GitHub-hosted runners this runs on. Neither is assumed.
digest_of() {
	local file="$1" out
	if command -v shasum >/dev/null 2>&1; then
		out="$(shasum -a 256 "$file")" || return 1
	elif command -v sha256sum >/dev/null 2>&1; then
		out="$(sha256sum "$file")" || return 1
	else
		return 2
	fi
	# The tool's own exit code is read above; the field split cannot mask it.
	printf '%s' "${out%% *}"
}

if ! command -v shasum >/dev/null 2>&1 && ! command -v sha256sum >/dev/null 2>&1; then
	echo "check-cask-asset: neither shasum nor sha256sum is on PATH" >&2
	exit 2
fi

# ---------------------------------------------------------------------------
# Parse the cask.
#
# The sha256 stanza spans two lines, so a line that opens one stays "inside" it
# until a line stops ending in a comma. A `url` seen inside `livecheck do` is
# not the download and is skipped; the first one outside it is.
# ---------------------------------------------------------------------------
VERSION=""
URL_TEMPLATE=""
URL_LINE=0
ARM_NAME=""
INTEL_NAME=""
ARM_SHA=""
INTEL_SHA=""
SHA_LINE=0

in_livecheck=0
in_sha=0
lineno=0

while IFS= read -r line || [[ -n "$line" ]]; do
	lineno=$((lineno + 1))

	if [[ $in_livecheck -eq 1 ]]; then
		[[ "$line" =~ ^[[:space:]]*end[[:space:]]*$ ]] && in_livecheck=0
		continue
	fi

	if [[ "$line" =~ ^[[:space:]]*livecheck[[:space:]]+do ]]; then
		in_livecheck=1
		continue
	fi

	if [[ "$line" =~ ^[[:space:]]*arch[[:space:]] ]]; then
		[[ "$line" =~ arm:[[:space:]]*\"([^\"]*)\" ]] && ARM_NAME="${BASH_REMATCH[1]}"
		[[ "$line" =~ intel:[[:space:]]*\"([^\"]*)\" ]] && INTEL_NAME="${BASH_REMATCH[1]}"
	elif [[ "$line" =~ ^[[:space:]]*version[[:space:]]+\"([^\"]*)\" ]]; then
		VERSION="${BASH_REMATCH[1]}"
	elif [[ "$line" =~ ^[[:space:]]*url[[:space:]]+\"([^\"]*)\" ]]; then
		if [[ -n "$URL_TEMPLATE" ]]; then
			echo "check-cask-asset: $CASK:$lineno: a second top-level url; expected one" >&2
			exit 2
		fi
		URL_TEMPLATE="${BASH_REMATCH[1]}"
		URL_LINE=$lineno
	elif [[ "$line" =~ ^[[:space:]]*sha256[[:space:]] ]]; then
		in_sha=1
		SHA_LINE=$lineno
		[[ "$line" =~ arm:[[:space:]]*\"([^\"]*)\" ]] && ARM_SHA="${BASH_REMATCH[1]}"
		[[ "$line" =~ intel:[[:space:]]*\"([^\"]*)\" ]] && INTEL_SHA="${BASH_REMATCH[1]}"
		[[ "$line" =~ ,[[:space:]]*$ ]] || in_sha=0
	elif [[ $in_sha -eq 1 ]]; then
		[[ "$line" =~ arm:[[:space:]]*\"([^\"]*)\" ]] && ARM_SHA="${BASH_REMATCH[1]}"
		[[ "$line" =~ intel:[[:space:]]*\"([^\"]*)\" ]] && INTEL_SHA="${BASH_REMATCH[1]}"
		[[ "$line" =~ ,[[:space:]]*$ ]] || in_sha=0
	fi
done <"$CASK"

# One pair per architecture that has a name, a digest and a url to expand.
# An architecture with a digest and no name, or a name and no digest, is a
# cask that is not what it says it is; it is refused rather than skipped.
declare -a URLS=()
declare -a SHAS=()
declare -a ARCHS=()

if [[ -n "$ARM_NAME" && -z "$ARM_SHA" ]] || [[ -z "$ARM_NAME" && -n "$ARM_SHA" ]]; then
	echo "check-cask-asset: $CASK: the arm architecture name and its sha256 must come together" >&2
	exit 2
fi
if [[ -n "$INTEL_NAME" && -z "$INTEL_SHA" ]] || [[ -z "$INTEL_NAME" && -n "$INTEL_SHA" ]]; then
	echo "check-cask-asset: $CASK: the intel architecture name and its sha256 must come together" >&2
	exit 2
fi

if [[ -z "$VERSION" || -z "$URL_TEMPLATE" ]]; then
	echo "check-cask-asset: $CASK: no top-level version and url to expand" >&2
	exit 2
fi

expand() { # expand <arch-name>
	local u="${URL_TEMPLATE//'#{version}'/$VERSION}"
	printf '%s' "${u//'#{arch}'/$1}"
}

if [[ -n "$ARM_NAME" ]]; then
	ARCHS+=("arm")
	URLS+=("$(expand "$ARM_NAME")")
	SHAS+=("$ARM_SHA")
fi
if [[ -n "$INTEL_NAME" ]]; then
	ARCHS+=("intel")
	URLS+=("$(expand "$INTEL_NAME")")
	SHAS+=("$INTEL_SHA")
fi

pairs=${#URLS[@]}

if [[ $pairs -lt $MIN_PAIRS ]]; then
	echo "check-cask-asset: $CASK carries $pairs url/sha256 pair(s), expected at least $MIN_PAIRS" >&2
	echo "    a cask that lost an architecture must not report clean on the one that remains" >&2
	exit 2
fi

# Two architectures that expand to one url are one asset checked twice. Compare
# every pair rather than assume there are only two.
a=0
while [[ $a -lt $pairs ]]; do
	b=$((a + 1))
	while [[ $b -lt $pairs ]]; do
		if [[ "${URLS[$a]}" == "${URLS[$b]}" ]]; then
			echo "check-cask-asset: $CASK:$URL_LINE: the ${ARCHS[$a]} and ${ARCHS[$b]} architectures expand to the same url" >&2
			echo "    ${URLS[$a]}" >&2
			echo "    the url template has lost #{arch}; one image would be checked and called both" >&2
			exit 2
		fi
		b=$((b + 1))
	done
	a=$((a + 1))
done

TMP="$(mktemp -d)" || exit 2
trap 'rm -rf "$TMP"' EXIT

failures=0

echo "check-cask-asset: $CASK — $pairs url/sha256 pair(s)"

i=0
while [[ $i -lt $pairs ]]; do
	url="${URLS[$i]}"
	want="${SHAS[$i]}"
	arch="${ARCHS[$i]}"
	i=$((i + 1))

	if [[ ! "$want" =~ ^[0-9a-f]{64}$ ]]; then
		echo "FAIL  $CASK:$SHA_LINE: the $arch sha256 is not 64 lowercase hex digits" >&2
		echo "        url:      $url" >&2
		echo "        declared: $want" >&2
		failures=$((failures + 1))
		continue
	fi

	asset="$TMP/asset.$i"
	# No -f. Without it curl reports the status line instead of exiting 22 on
	# every 4xx alike, so a red run names 404 (the asset is not there) apart
	# from 403 (it is there and unreadable). --retry does not retry a 404.
	code="$(curl -sSL --retry 3 --retry-delay 2 --max-time 600 \
		-o "$asset" -w '%{http_code}' "$url" 2>"$TMP/curl.err")"
	rc=$?

	if [[ $rc -ne 0 ]]; then
		echo "FAIL  the cask's $arch url could not be fetched (curl exit $rc)" >&2
		echo "        url: $url" >&2
		sed 's/^/        /' "$TMP/curl.err" >&2
		failures=$((failures + 1))
		continue
	fi

	if [[ "$code" != "200" ]]; then
		echo "FAIL  the cask's $arch url does not resolve to an asset (HTTP $code)" >&2
		echo "        url: $url" >&2
		echo "        the checksum beside it was published against a file nobody can download" >&2
		failures=$((failures + 1))
		continue
	fi

	got="$(digest_of "$asset")"
	drc=$?
	if [[ $drc -ne 0 ]] || [[ ! "$got" =~ ^[0-9a-f]{64}$ ]]; then
		echo "check-cask-asset: could not hash the asset downloaded from $url" >&2
		exit 2
	fi

	bytes="$(wc -c <"$asset" | tr -d '[:space:]')"

	if [[ "$got" != "$want" ]]; then
		echo "FAIL  the asset does not match the checksum published beside it" >&2
		echo "        url:      $url" >&2
		echo "        declared: $want  ($CASK:$SHA_LINE, $arch)" >&2
		echo "        actual:   $got  ($bytes bytes fetched)" >&2
		failures=$((failures + 1))
		continue
	fi

	echo "  ok  $url"
	echo "      $got  ($bytes bytes)"
done

if [[ $failures -ne 0 ]]; then
	echo "check-cask-asset: $failures of $pairs pair(s) failed — do not publish this cask" >&2
	exit 1
fi

echo "check-cask-asset: $pairs of $pairs pair(s) fetched and matched"
exit 0
