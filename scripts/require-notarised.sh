#!/usr/bin/env bash
# Refuse to publish the cask unless the build was notarised.
#
# Usage:  scripts/require-notarised.sh SIGNING
#
# SIGNING is the build job's `signing` output. Exit 0 when it is exactly
# `notarised`; exit 1 with an ::error:: naming the value for `adhoc`, an empty
# value or anything else.
#
# Why: the cask installs the app bundle, which Gatekeeper opens only when it is
# notarised. An ad-hoc build installs and then refuses to launch on a stranger's
# machine, so the release must go red before the tap changes, not after.
set -uo pipefail

signing="${1-}"
if [[ "$signing" == "notarised" ]]; then
	echo "build was notarised"
	exit 0
fi
echo "::error::the cask needs a notarised build, but the build's signing output is '${signing}'"
exit 1
