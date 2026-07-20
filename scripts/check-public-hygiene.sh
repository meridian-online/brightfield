#!/usr/bin/env bash
# Public-hygiene gate: stop private planning identifiers reaching this public repo.
#
# This repository is public. The planning that drives it is not. Identifiers that
# only resolve inside the private planning tracker — decision records, task ids,
# milestone ids, acceptance-criterion shorthand, document ids, card ids, spec AC
# ids — are meaningless to anyone reading this repo and leak the shape of private
# work. They have reached main repeatedly under a convention-only rule. This is
# the enforcement: it runs in CI, and it is meant to be the first job to go red.
#
# Usage (no arguments, from anywhere inside the repo):
#
#   ./scripts/check-public-hygiene.sh
#
# Exit codes:
#   0  clean
#   1  one or more violations found
#   2  the allowlist is malformed (an entry without an explanation)
#
# Scope: `git grep` over TRACKED files only. Build artifacts, target/, node
# modules and anything else ignored are invisible to this gate by construction —
# so a dirty working tree full of generated files can never make it cry wolf.
#
# ---------------------------------------------------------------------------
# On false positives
# ---------------------------------------------------------------------------
# A gate that cries wolf gets disabled within a week, which is worse than no
# gate. Every pattern below was measured against the real tree before it was
# committed, and every one of them reports zero on a clean checkout. The
# non-obvious guards, and what they are protecting:
#
# * Identifier guard. Most patterns are wrapped in (?<![-_A-Za-z0-9]) ...
#   (?![-_A-Za-z0-9]) so they only fire on a FREE-STANDING reference. Spec AC
#   ids are embedded in hundreds of legitimate Rust identifiers here — test
#   function names (`fn xyz_acNN_does_a_thing`) and temp-dir literals
#   (`bf-xyz-acNN-<pid>`). Those are names, not pointers; renaming them would
#   touch executable code. The guard keeps the gate off them.
#
# * Bare ticket refs. A bare capital-T-plus-digits collides head-on with Rust
#   generic parameters (`<T1, T2>`, `impl<T1: Clone>`). Two deliberate limits:
#   the general rule requires TWO OR MORE digits (real ticket ids here are
#   two-digit; generic params are effectively always single-digit), and the
#   lookarounds exclude the angle-bracket / comma / colon context generics
#   appear in. It also excludes a preceding digit, which is what keeps ISO
#   timestamps (`...-20T10:30:00`) out. A single-digit ticket ref is caught only
#   in its parenthesised form. The deliberate gap: a bare one-digit reference in
#   running prose is NOT caught — that was the price of not flagging generics.
#
# * Milestone ids. Lowercase `m-<digits>` only, with the identifier guard on the
#   left, so CSS custom properties in the token pipeline (`--m-accent-bg`,
#   and any future `--m-<digit>`) cannot trip it.
#
# * Acceptance criteria. Case-SENSITIVE. The lowercase form collides with
#   lockfile git revisions, where a hex rev can end `...ac#4afea48...`.
#
# * Card ids. Three or four digits required; the workflow vocabulary word
#   "card" and the literal UI cards in the renderer carry no number and are
#   left alone.
#
# If a pattern flags something legitimate, FIX THE PATTERN. The allowlist is for
# genuine content that must stay, not for papering over a bad regex.
# ---------------------------------------------------------------------------

set -uo pipefail

# Run from the repo root regardless of where the caller invoked us.
REPO_ROOT="$(git rev-parse --show-toplevel)" || {
	echo "check-public-hygiene: not inside a git repository" >&2
	exit 2
}
cd "$REPO_ROOT" || exit 2

ALLOWLIST="scripts/public-hygiene-allowlist.txt"

# The rules use PCRE lookarounds, which git only offers when it was built with
# PCRE. Fail loudly rather than silently matching nothing — a gate that quietly
# no-ops is the failure mode this whole file exists to prevent.
# git grep exits 0 on a match, 1 on no match, and >1 on an error such as "cannot
# use Perl-compatible regexes when not compiled with USE_LIBPCRE".
git grep -qP -e 'zzzz(?<!qqqq)' -- . >/dev/null 2>&1
pcre_rc=$?
if [[ $pcre_rc -gt 1 ]]; then
	echo "check-public-hygiene: this git cannot run PCRE patterns (git grep -P exited $pcre_rc)" >&2
	echo "    install a git built with PCRE, or the gate cannot run" >&2
	exit 2
fi

# ---------------------------------------------------------------------------
# Rules: "<label>|<PCRE>". Anything git grep -P accepts.
# ---------------------------------------------------------------------------
RULES=(
	'private-decision-record|(?i)(?<![-_A-Za-z0-9])decision-[0-9]+'
	'planning-task-id|(?i)(?<![-_A-Za-z0-9])task-[0-9]+'
	'bare-ticket-ref|(?<![-_A-Za-z0-9<,])T[0-9]{2,3}(?![-_A-Za-z0-9>,:])'
	'bare-ticket-ref|\(T[0-9]{1,3}\)'
	'milestone-id|(?<![-_A-Za-z0-9])m-[0-9]+(?![-_A-Za-z0-9])'
	'acceptance-criterion|(?<![A-Za-z0-9])AC#[0-9]+'
	'private-doc-id|(?i)(?<![-_A-Za-z0-9])doc-[0-9]+(?![-_A-Za-z0-9])'
	'planning-card-id|(?i)(?<![-_A-Za-z0-9])cards?[ _-][0-9]{3,4}(?![0-9])'
	'spec-ac-id|(?<![-_A-Za-z0-9])[A-Za-z]{2,6}[-_ ]ac[-_]?[0-9]+[a-z]?(?![-_A-Za-z0-9])'
	'spec-ac-id|(?<![-_A-Za-z0-9])ac[-_][0-9]+[a-z]?(?![-_A-Za-z0-9])'
)

# ---------------------------------------------------------------------------
# Allowlist.
#
# Format, one entry per line:
#
#     <tracked/file/path>:<exact offending text>  # why this one is legitimate
#
# Line numbers are deliberately NOT part of an entry — they drift on every edit
# and an allowlist that silently stops matching is worse than none. Blank lines
# and whole-line `#` comments are ignored.
#
# The trailing `#` explanation is REQUIRED. An entry without one is a hard error
# (exit 2), so the escape hatch cannot be used silently.
# ---------------------------------------------------------------------------
declare -a ALLOW_KEYS=()
if [[ -f "$ALLOWLIST" ]]; then
	lineno=0
	bad_allow=0
	while IFS= read -r raw || [[ -n "$raw" ]]; do
		lineno=$((lineno + 1))
		# Strip trailing CR (in case someone edits on Windows) and outer space.
		line="${raw%$'\r'}"
		[[ -z "${line// /}" ]] && continue
		[[ "${line#"${line%%[![:space:]]*}"}" == \#* ]] && continue

		if [[ "$line" != *"#"* ]]; then
			echo "check-public-hygiene: $ALLOWLIST:$lineno: entry has no explanation" >&2
			echo "    $line" >&2
			echo "    every entry must end with '# <why this is legitimate>'" >&2
			bad_allow=1
			continue
		fi

		entry="${line%%#*}"
		reason="${line#*#}"
		# Trim.
		entry="${entry#"${entry%%[![:space:]]*}"}"
		entry="${entry%"${entry##*[![:space:]]}"}"
		reason="${reason#"${reason%%[![:space:]]*}"}"
		reason="${reason%"${reason##*[![:space:]]}"}"

		if [[ -z "$reason" ]]; then
			echo "check-public-hygiene: $ALLOWLIST:$lineno: explanation is empty" >&2
			echo "    $line" >&2
			bad_allow=1
			continue
		fi
		if [[ -z "$entry" ]]; then
			echo "check-public-hygiene: $ALLOWLIST:$lineno: no <path>:<text> before the '#'" >&2
			bad_allow=1
			continue
		fi
		if [[ "$entry" != *:* ]]; then
			echo "check-public-hygiene: $ALLOWLIST:$lineno: expected <path>:<text>, got '$entry'" >&2
			bad_allow=1
			continue
		fi
		ALLOW_KEYS+=("$entry")
	done <"$ALLOWLIST"
	if [[ $bad_allow -ne 0 ]]; then
		exit 2
	fi
fi

is_allowed() {
	local key="$1" k
	for k in "${ALLOW_KEYS[@]+"${ALLOW_KEYS[@]}"}"; do
		[[ "$k" == "$key" ]] && return 0
	done
	return 1
}

# ---------------------------------------------------------------------------
# Scan.
# ---------------------------------------------------------------------------
violations=0
allowed=0

for rule in "${RULES[@]}"; do
	label="${rule%%|*}"
	pattern="${rule#*|}"

	# -I skips binary files, -n gives line numbers, -o prints just the match.
	while IFS= read -r hit; do
		[[ -z "$hit" ]] && continue
		file="${hit%%:*}"
		rest="${hit#*:}"
		line="${rest%%:*}"
		text="${rest#*:}"

		if is_allowed "$file:$text"; then
			allowed=$((allowed + 1))
			continue
		fi

		violations=$((violations + 1))
		printf '%s:%s: %s: %s\n' "$file" "$line" "$label" "$text"
		# Show the offending source line so the fix is obvious without opening
		# the file. `sed -n Np` is cheap and the file is tracked, so it exists.
		src="$(sed -n "${line}p" -- "$file" 2>/dev/null)"
		[[ -n "$src" ]] && printf '    | %s\n' "$src"
	# The allowlist is excluded from the scan: by construction it quotes the
	# exact text it is waving through, so scanning it would make every entry
	# self-violating. It is the one file with that property — the checker
	# script itself is scanned (its patterns contain no literal ids).
	done < <(git grep -PIn -o -e "$pattern" -- . ":(exclude)$ALLOWLIST" 2>/dev/null)
done

if [[ $violations -gt 0 ]]; then
	echo
	echo "check-public-hygiene: FAILED — $violations private planning identifier(s) in tracked files."
	echo
	echo "These identifiers only resolve inside the private planning tracker and must not"
	echo "appear in a public repo. Delete the pointer and, if it carried meaning, replace it"
	echo "with the actual rationale in plain English."
	echo
	echo "If a match is genuinely legitimate, first try to make the pattern more precise in"
	echo "scripts/check-public-hygiene.sh. Only if that is impossible, add a line to"
	echo "$ALLOWLIST in the form:"
	echo
	echo "    path/to/file:<exact matched text>  # why this is legitimate"
	exit 1
fi

if [[ $allowed -gt 0 ]]; then
	echo "check-public-hygiene: clean ($allowed allowlisted match(es))."
else
	echo "check-public-hygiene: clean."
fi
exit 0
