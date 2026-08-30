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
# THE SHAPES ARE NOT IN THIS FILE. scripts/public-hygiene-rules.txt holds them —
# every rule, and the reasoning behind its non-obvious guards — and
# scripts/check-history-hygiene.sh reads the same file for commit messages and
# pull request title and body. They were two lists inside two scripts until a
# leak used the surface only one of them read: a private planning identifier
# reached this repository once in a pull request body and once in a commit
# message, and this gate reported clean both times because neither surface was
# ever its job.
#
# Usage (no arguments, from anywhere inside the repo):
#
#   ./scripts/check-public-hygiene.sh
#
# Exit codes:
#   0  clean
#   1  one or more violations found
#   2  the gate could not run correctly — a broken rule, a missing or malformed
#      rules file, a malformed allowlist entry, a stale allowlist entry, or a
#      git without PCRE. ALWAYS a hard failure: a gate that cannot run must
#      never look like a gate that passed.
#
# Its own regression test is scripts/check-public-hygiene-selftest.sh.
#
# What this covers, and what it does NOT
# --------------------------------------
# COVERED: the content of TRACKED files in the current checkout, via `git grep`.
# Build artifacts, target/, node modules and anything else ignored are invisible
# by construction, so a dirty working tree full of generated files can never make
# the gate cry wolf.
#
# COVERED BY A SIBLING, scripts/check-history-hygiene.sh, which reads the same
# scripts/public-hygiene-rules.txt this file reads:
#   * commit messages, over the range the workflow hands it;
#   * a pull request's title and body, including on a body edited after the last
#     run (.github/workflows/pr-text-hygiene.yml triggers on `edited`).
#
# STILL NOT COVERED, and you have to watch these yourself:
#   * review comments on a pull request,
#   * branch names,
#   * commits reachable only from a range neither workflow was asked to scan,
#   * anything in the current tree's history that is no longer in any tracked
#     file and no longer in any reachable commit message,
#   * issues, releases, wiki, and everything else that lives on the forge rather
#     than in the repository.
# Those are real leak vectors here and nothing in either gate inspects them.
#
# A published pull request body revision cannot be fixed by editing the body —
# the forge retains prior revisions and serves them to any reader with access —
# and neither gate performs or decides the removal of one; see the header of
# scripts/check-history-hygiene.sh for why.
#
# ---------------------------------------------------------------------------
# On false positives
# ---------------------------------------------------------------------------
# A gate that cries wolf gets disabled within a week, which is worse than no
# gate. Every pattern in scripts/public-hygiene-rules.txt was measured against
# the real tree before it was committed, and
# scripts/public-hygiene-innocent-strings.txt is a tracked fixture of
# innocent-but-similar-looking strings the gate must stay silent on. That
# fixture is scanned like any other tracked file, so a pattern that starts
# biting real-world prose or Rust turns the gate red on its own fixture.
#
# If a pattern flags something legitimate, FIX THE PATTERN in
# scripts/public-hygiene-rules.txt and add the innocent string to the fixture.
# The allowlist is for genuine content that must stay, not for papering over a
# bad regex.
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
# Rules. They are NOT in this file.
#
# scripts/public-hygiene-rules.txt holds them, as "<label>|<PCRE>" lines, and
# scripts/check-history-hygiene.sh reads the same file for commit messages and
# pull request text. Several labels carry more than one pattern. Matches are
# deduplicated per (label, file, line), so one offending line is reported once
# per rule even when two of that rule's patterns fire on it.
#
# A pattern may never match a `|`: the allowlist format is pipe-separated and
# relies on matched text being unable to collide with the separator. Everything
# else the patterns can emit — `#`, `-`, `_`, spaces, parentheses — is fine.
# ---------------------------------------------------------------------------
RULES_FILE="scripts/public-hygiene-rules.txt"

if [[ ! -f "$RULES_FILE" ]]; then
	echo "check-public-hygiene: $RULES_FILE is missing — the gate has no rules to run" >&2
	exit 2
fi

declare -a RULES=()
rules_lineno=0
bad_rules=0
while IFS= read -r raw || [[ -n "$raw" ]]; do
	rules_lineno=$((rules_lineno + 1))
	line="${raw%$'\r'}"
	[[ -z "${line//[[:space:]]/}" ]] && continue
	[[ "${line#"${line%%[![:space:]]*}"}" == \#* ]] && continue
	if [[ "$line" != *"|"* ]]; then
		echo "check-public-hygiene: $RULES_FILE:$rules_lineno: expected '<label>|<pattern>'" >&2
		echo "    $line" >&2
		bad_rules=1
		continue
	fi
	r_label="${line%%|*}"
	r_pattern="${line#*|}"
	if [[ -z "$r_label" || -z "$r_pattern" ]]; then
		echo "check-public-hygiene: $RULES_FILE:$rules_lineno: label and pattern are both required" >&2
		bad_rules=1
		continue
	fi
	RULES+=("$r_label|$r_pattern")
done <"$RULES_FILE"
if [[ $bad_rules -ne 0 ]]; then
	exit 2
fi
if [[ ${#RULES[@]} -eq 0 ]]; then
	echo "check-public-hygiene: $RULES_FILE declares no rules — an empty gate reports clean" >&2
	exit 2
fi

# ---------------------------------------------------------------------------
# Allowlist.
#
# Format, one entry per line, THREE pipe-separated fields:
#
#     <tracked/file/path> | <exact offending text> | <why this is legitimate>
#
# `|` is the separator precisely because no rule pattern can ever match a `|`,
# so the offending text — which routinely contains `#`, `-` and `_` — can never
# collide with the separator. All three fields are required and none may be
# empty. Anything that does not parse is a hard error (exit 2), so the escape
# hatch cannot be used silently or reached by accident.
#
# Line numbers are deliberately NOT part of an entry — they drift on every edit.
# Instead every entry must MATCH SOMETHING: an entry that suppresses nothing is
# also a hard error, so a stale allowlist cannot rot into fake coverage.
#
# Blank lines and whole-line `#` comments are ignored.
# ---------------------------------------------------------------------------
declare -a ALLOW_PATH=()
declare -a ALLOW_TEXT=()
declare -a ALLOW_LINENO=()
declare -a ALLOW_HITS=()

trim() {
	local s="$1"
	s="${s#"${s%%[![:space:]]*}"}"
	s="${s%"${s##*[![:space:]]}"}"
	printf '%s' "$s"
}

if [[ -f "$ALLOWLIST" ]]; then
	lineno=0
	bad_allow=0
	while IFS= read -r raw || [[ -n "$raw" ]]; do
		lineno=$((lineno + 1))
		# Strip a trailing CR, in case someone edits on Windows.
		line="${raw%$'\r'}"
		trimmed="$(trim "$line")"
		[[ -z "$trimmed" ]] && continue
		[[ "$trimmed" == \#* ]] && continue

		# Split on `|` into exactly three fields, counting the separators
		# rather than reading into an array: `read -r -a` DISCARDS a trailing
		# empty field, so `path | text |` — an entry whose author could not
		# think of a reason, which is precisely the shape the next check
		# exists to refuse — arrived as two fields and was reported as a
		# malformed line instead of a reasonless one.
		seps="${line//[^|]/}"
		if [[ ${#seps} -ne 2 ]]; then
			echo "check-public-hygiene: $ALLOWLIST:$lineno: expected 3 '|'-separated fields, got $((${#seps} + 1))" >&2
			echo "    $line" >&2
			echo "    format: <tracked/file/path> | <exact offending text> | <why this is legitimate>" >&2
			bad_allow=1
			continue
		fi
		rest="${line#*|}"
		a_path="$(trim "${line%%|*}")"
		a_text="$(trim "${rest%%|*}")"
		a_reason="$(trim "${rest#*|}")"
		if [[ -z "$a_path" || -z "$a_text" || -z "$a_reason" ]]; then
			echo "check-public-hygiene: $ALLOWLIST:$lineno: path, text and explanation are all required" >&2
			echo "    $line" >&2
			bad_allow=1
			continue
		fi
		ALLOW_PATH+=("$a_path")
		ALLOW_TEXT+=("$a_text")
		ALLOW_LINENO+=("$lineno")
		ALLOW_HITS+=(0)
	done <"$ALLOWLIST"
	if [[ $bad_allow -ne 0 ]]; then
		exit 2
	fi
fi

ALLOW_COUNT=${#ALLOW_PATH[@]}

# Returns 0 — and marks the entry used — when this file/text pair is allowlisted.
is_allowed() {
	local file="$1" text="$2" i
	for ((i = 0; i < ALLOW_COUNT; i++)); do
		if [[ "${ALLOW_PATH[$i]}" == "$file" && "${ALLOW_TEXT[$i]}" == "$text" ]]; then
			ALLOW_HITS[i]=$((ALLOW_HITS[i] + 1))
			return 0
		fi
	done
	return 1
}

# ---------------------------------------------------------------------------
# Scan.
# ---------------------------------------------------------------------------
violations=0
allowed=0
# Newline-delimited "<label>:<file>:<line>" keys already reported. A plain string
# rather than an associative array on purpose: macOS still ships bash 3.2, which
# has no `declare -A`, and this gate has to run on a developer's machine as
# readily as on CI.
seen_keys=""

hits="$(mktemp)" || exit 2
errs="$(mktemp)" || exit 2
trap 'rm -f "$hits" "$errs"' EXIT

for rule in "${RULES[@]}"; do
	label="${rule%%|*}"
	pattern="${rule#*|}"

	# -I skips binary files, -n gives line numbers, -o prints just the match.
	#
	# The allowlist is excluded from the scan: by construction it quotes the
	# exact text it is waving through, so scanning it would make every entry
	# self-violating. It is the one file with that property — the checker script
	# itself is scanned (its patterns contain no literal ids).
	#
	# The exit code is checked BEFORE the output is read, and it is checked per
	# rule. git grep exits 0 on a match, 1 on no match, and >1 on an error — a
	# broken pattern exits 128 and prints nothing to stdout, which without this
	# check reads exactly like "no violations" and lets the gate report clean
	# while blind. Anything above 1 is fatal and names the rule.
	git grep -PIn -o -e "$pattern" -- . ":(exclude)$ALLOWLIST" >"$hits" 2>"$errs"
	grep_rc=$?
	if [[ $grep_rc -gt 1 ]]; then
		echo "check-public-hygiene: RULE FAILED TO RUN — '$label' (git grep exited $grep_rc)" >&2
		echo "    pattern: $pattern" >&2
		while IFS= read -r errline; do
			[[ -n "$errline" ]] && echo "    $errline" >&2
		done <"$errs"
		echo "    the gate cannot report clean while a rule is broken — fix the pattern" >&2
		exit 2
	fi

	while IFS= read -r hit; do
		[[ -z "$hit" ]] && continue
		file="${hit%%:*}"
		rest="${hit#*:}"
		line="${rest%%:*}"
		text="${rest#*:}"

		if is_allowed "$file" "$text"; then
			allowed=$((allowed + 1))
			continue
		fi

		key="$label:$file:$line"
		case $'\n'"$seen_keys" in
		*$'\n'"$key"$'\n'*) continue ;;
		esac
		seen_keys="$seen_keys$key"$'\n'

		violations=$((violations + 1))
		printf '%s:%s: %s: %s\n' "$file" "$line" "$label" "$text"
		# Show the offending source line so the fix is obvious without opening
		# the file. `sed -n Np` is cheap and the file is tracked, so it exists.
		src="$(sed -n "${line}p" -- "$file" 2>/dev/null)"
		[[ -n "$src" ]] && printf '    | %s\n' "$src"
	done <"$hits"
done

# A stale allowlist entry is a hole nobody is watching: it says "this exact text
# in this exact file is fine", and once the text has moved or gone it suppresses
# nothing while still looking like coverage. Hard error.
stale=0
for ((i = 0; i < ALLOW_COUNT; i++)); do
	if [[ ${ALLOW_HITS[$i]} -eq 0 ]]; then
		echo "check-public-hygiene: $ALLOWLIST:${ALLOW_LINENO[$i]}: stale entry — it suppresses nothing" >&2
		echo "    ${ALLOW_PATH[$i]} | ${ALLOW_TEXT[$i]}" >&2
		stale=1
	fi
done
if [[ $stale -ne 0 ]]; then
	echo "    the text or the file has changed. Correct the entry, or delete it." >&2
	exit 2
fi

if [[ $violations -gt 0 ]]; then
	echo
	echo "check-public-hygiene: FAILED — $violations private planning identifier(s) in tracked files."
	echo
	echo "These identifiers only resolve inside the private planning tracker and must not"
	echo "appear in a public repo. Delete the pointer and, if it carried meaning, replace it"
	echo "with the actual rationale in plain English."
	echo
	echo "If a match is genuinely legitimate, first try to make the pattern more precise in"
	echo "scripts/check-public-hygiene.sh, adding the innocent string to"
	echo "scripts/public-hygiene-innocent-strings.txt so it stays fixed. Only if that is"
	echo "impossible, add a line to $ALLOWLIST in the form:"
	echo
	echo "    path/to/file | <exact matched text> | why this is legitimate"
	exit 1
fi

if [[ $allowed -gt 0 ]]; then
	echo "check-public-hygiene: clean ($allowed allowlisted match(es))."
else
	echo "check-public-hygiene: clean."
fi
exit 0
