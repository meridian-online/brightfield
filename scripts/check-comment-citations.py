#!/usr/bin/env python3
"""Gate the citations in changed comments: a named symbol or path must resolve.

WHY THIS EXISTS
    Over two days this repo shipped ~20 false statements in comments across two
    PRs, and every blocking review finding was prose rather than logic. They
    share one shape: a claim about code the author was NOT looking at, plausible
    because it was almost true, or true of a neighbour, or true last week.

    Two of them were the same sentence, twice: a const attributed to
    `brightfield-sql` that lives in `brightfield-render`, then "corrected" into a
    universal claim that was also false. Both were one grep from being refuted.
    The grep is cheap; performing it is what did not happen. So it happens here.

WHAT IS CHECKED (changed comment lines only, `origin/main...HEAD`, read at HEAD)
    A  ATTRIBUTION — "`<crate>`'s `<SYMBOL>`" or "`<SYMBOL>` in `<crate>`"
       asserts that a symbol is DEFINED in a named crate. Verified by looking
       for an actual definition (`const`/`static`/`fn`/`struct`/`enum`/`trait`/
       `type`/`macro_rules!`) under that crate. A mere mention does not count:
       "brightfield-sql's AGGREGATE_COUNT_COL" is false precisely because that
       crate only ever *reads* the literal.
    B  PATH — a backticked repo-relative path (optionally `:LINE`) must exist.
    C  PACKAGE — a backticked package name the comment pins to a version
       ("`foo` (v1.2.3)", "`foo 1.2.3`", "v1.2.3 of `foo`") or labels as a
       package/crate/library ("the `foo` crate"). Resolved against the
       workspace crates and the `name = "..."` entries in Cargo.lock, so a
       package this tree does not build against does not resolve and must be
       registered below. Attribution (A) needs a backticked SYMBOL to fire, and
       a claim that names a package and a version need carry no symbol at all —
       which is the gap this closes.
    D  COMPLETENESS — a quantifier that asserts a completeness (only, always,
       every, never, nothing, none, all, everything, anything) in the same
       sentence as a named symbol, where THAT SENTENCE names no test that
       resolves. A, B and C
       ask whether a cited thing is where the comment says it is; D asks
       whether a claim about ALL of something has anything holding it. It
       cannot judge the claim — no gate can — so it asks for the citation that
       lets a reader judge it: name the test, or drop the quantifier.

       Judging nothing, its whole design is about not crying wolf. It
       needs a SYMBOL, not merely a backticked token: this repo backticks
       channel names, role names, kind ids and column values, and a rule that
       read those as symbols would report ordinary prose. What counts as a
       symbol is in symbol_citations(). And it needs the quantifier and the
       symbol in ONE sentence.

       A sentence, not a line. Rules A, B and C read one line at a time, but a
       sentence wraps, and reading a line as if the wrap ended it made rule D
       blind to a claim whose subject sat on the next line. So D joins the
       CONTIGUOUS comment lines around each one it is checking and splits that
       into sentences; the commit that landed the block reader carries what
       that recovered on the corpus this rule is graded against. Blank comment
       lines and list markers break the join: two bullets are two claims, and
       joining them would pair one item's quantifier with the next item's
       symbol. A claim is reported once, at the line its quantifier is on.

       A sentence, not a PARAGRAPH either — and that is the other half. The
       citation that discharges a claim has to sit in the sentence the
       quantifier is in. Read across the paragraph, as this did until the
       sentence rule, a test named about one claim silenced a different claim
       three lines away that it says nothing about. The two readings are
       deliberately different sizes and STATEMENT_END records why: a claim is
       read from a `;`-split clause, a citation from the whole sentence the
       clause sits in.

WHAT IS *NOT* CHECKED (scope, stated so nobody reads this as more than it is)
    - Findings land on ADDED comment lines in the diff. Rule D reads the
      unchanged neighbours of one, because a sentence wraps, but it reports at
      the line its quantifier is on and admits the claim only when the diff
      added THAT line — so a reword under an old quantifier is not a finding.
      This stops new claims rather than auditing old ones — but note the
      residue, which is the same for rules A/B/C: rewording an unrelated word
      ON a line that carries an old quantifier does add that line, so its
      pre-existing claim is reported. `--all` audits the tree.
    - Only `.rs` files, and only `//`, `///`, `//!` lines.
    - Bare `a::b` Rust paths are deliberately NOT resolved. Precision matters
      more than recall here — a gate that cries wolf gets disabled, and this
      repo has three defused guards already. Adding path resolution means
      teaching it modules, re-exports and glob imports; until then a `::` path
      is out of scope rather than half-checked.
    - A package name has to be BACKTICKED, and pinned to a version or labelled
      a package/crate/library, before it is treated as a citation. "tracks egui
      0.35" is prose about a dependency, not a claim this can resolve. The
      version itself is not compared against Cargo.lock: a comment may be
      describing the version a workaround was written for.
    - It checks that a cited thing EXISTS where claimed. It cannot check what
      the code DOES. "merging under-reports every group but one" was false and
      no citation gate would ever have caught it — that needs a reviewer.
    - Rule D does not read the test it asks for. A comment that names one is
      past it, whatever the test asserts. It buys a reader a place to look and
      an author a moment of doubt, not a proof. What it does insist on is that
      the name RESOLVE and that it sit in the claim's own sentence — see
      cites_test() for what counts and in which order. Three shapes shipped
      false through the looser readings this replaces, and each is now a named
      case in --self-test: a name whose own spelling merely LOOKED like a test
      (`foo_test`) discharged a claim on its spelling alone, whether or not
      anything by that name existed; a real citation in one sentence
      discharged a claim in another; and

          /// [`kind_options`] is the only thing that enumerates kinds; the
          /// `the_hero_weight_is_the_pane_share` test holds that.

      went silent over a function that does not exist, because the fake name
      carries no `test` component of its own and the bare word `test` beside
      it forgave the paragraph. What still forgives, and it is the residue
      this leaves rather than an oversight: a sentence carrying the bare word
      `test` and NO backticked snake_case name at all — "…and that is tested",
      over a CamelCase subject — has no name to resolve, so it discharges.
      Name the test and that gap closes itself.
    - The workspace-wide resolver has a residual COLLISION risk of its own,
      in the opposite direction from the defect above: it is a name lookup
      with no notion of RELEVANCE, so a name that resolves ANYWHERE in the
      12-crate workspace discharges a claim ANYWHERE else in it, whether or
      not the two have anything to do with each other. Measured on this
      tree: a comment describing a keyboard shortcut — "the user presses
      `za`" — went silent because `za` is ALSO a `#[cfg(test)]` helper's name
      in an unrelated file, and a comment describing a trait method shared by
      many structs resolves through whichever ONE of them happens to be a
      test fixture. Three filters in resolvable_test_names() close the worst
      of it — a name with no underscore never resolves, in either the
      `tests/` or the `#[cfg(test)]` half; a METHOD never resolves at all;
      and a name production itself defines never resolves through either
      attribute-free half. What none of them closes is a name that exists
      ONLY as a test-side helper and is cited as the SUBJECT of the claim
      rather than as its measurement, which is a question about relevance
      and not about existence. Two comments in
      crates/brightfield-shell/tests/ghosted_histogram.rs are silent for
      exactly that reason. This is the same
      shape as rule A's `defines()` — an existence check, not a relevance
      check — just wider, because the enumeration it resolves against is
      wider.
    - Rule D's RECALL is PARTIAL. It was measured against the comment lines a
      review wave in this repo made an author delete, and the measurement is in
      the commit that landed the block reader. What that measurement leaves
      behind is refusal rather than oversight, in these shapes, each held by a
      control case in --self-test:
        * the sentence carries no backticked token at all. A quantifier with no
          symbol is prose a citation gate has no way to judge, and reporting it
          would be reporting English.
        * its backticked tokens are bare lowercase words the file it sits in
          does not define as items — a protocol status value quoted in prose,
          say. There is no route from such a word to anything a gate can
          resolve, and treating every backticked word as a symbol was measured
          on this tree: it adds sentences about parameters and literal values,
          which is the shape that gets a gate switched off.
        * the claim's own sentence names a test that resolves, which is the
          exemption above.
        * the diff adds the SYMBOL half of a wrapped claim and leaves the
          quantifier where it already was. Reported at the quantifier's line
          and admitted on it, that claim is old debt by this gate's reckoning
          even though the author has just widened it. The cost is asymmetric:
          this shape costs a reviewer a read, and the reading that would catch
          it strays onto lines the author never touched. Measured over the 41
          most recent `.rs`-touching commits: this shape misses 3 while still
          catching 468, where the other reading strays 11 times across 10 of
          those commits.
      A sentence rule D misses costs a reviewer a read. A sentence it reports
      wrongly costs the gate itself.
    - A diff run reads its content at the SAME commit its diff came from, so an
      uncommitted edit moves neither the lines it checks nor the numbers it
      reports. The resolvers are the exception and read the working tree: crate
      sources, Cargo.lock and the test functions rule D exempts on are the code
      the author has now, which is the code a citation should resolve against.

ESCAPE HATCH
    A claim about code outside this repo (upstream libraries, a sibling repo)
    cannot resolve here and must be registered in ACKNOWLEDGED with a reason,
    not silently skipped. Writing the reason is the point. The registry keys on
    the name as the comment writes it and each entry carries the KIND of thing
    it stands in for — a symbol for A, a package for C, a test for D. An entry
    excuses the rule it was registered for and no other; see Ack.

USAGE
    scripts/check-comment-citations.py             # gate the diff vs origin/main
    scripts/check-comment-citations.py --all       # audit every comment in tree
    scripts/check-comment-citations.py --self-test # prove the gate can fail
"""

from __future__ import annotations

import re
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import NamedTuple

ROOT = Path(__file__).resolve().parent.parent

class Ack(NamedTuple):
    """One registered external name: which RULE may spend it, and why it is absent.

    The kind is not decoration. Rule D discharges a completeness claim when the
    claim's sentence names a TEST, and an entry registered here for rule A's or
    rule C's purpose is not a test — it is a symbol or a package this tree does
    not contain. Read as one flat set, `queryFieldInfo` — a mosaic function
    registered so that prose ABOUT mosaic resolves — discharged a completeness
    claim in crates/brightfield-shell/tests/binned_histogram.rs as if it were
    the measurement holding it, while the three names that sentence actually
    cites resolve to nothing. So each entry says which rule may spend it:

      symbol   an item outside this repo, for rule A and for the symbol half of
               a claim. Never a citation of a test.
      package  a package this tree does not build against, for rule C.
      test     a test outside this workspace. The only kind rule D accepts.
    """

    kind: str
    reason: str


# Symbols and packages named in comments that this repo does not contain. Each
# needs a reason — the point of the registry is that "it's external" gets
# written down rather than assumed — and a KIND, because an entry excuses the
# rule it was registered for and not every rule at once.
ACKNOWLEDGED: dict[str, Ack] = {
    "markPlotSpec": Ack(
        "symbol", "mosaic/vgplot source; this tree vendors only the YAML spec corpus"
    ),
    "channelOption": Ack("symbol", "mosaic/vgplot source, same reason"),
    "queryFieldInfo": Ack("symbol", "mosaic/vgplot source, same reason"),
    "isColorChannel": Ack("symbol", "mosaic/vgplot source, same reason"),
    "literalToSQL": Ack("symbol", "mosaic sql package, not vendored"),
    "egui_code_editor": Ack(
        "package",
        "evaluated against the spec editor and rejected, so it is deliberately "
        "absent from Cargo.lock; the comparison is in "
        "crates/brightfield-shell/src/editor.rs",
    ),
}


def acknowledged(name: str, kind: str) -> bool:
    """Is `name` registered for THIS rule's purpose?

    Membership alone is not an answer. A name registered as an external SYMBOL
    answers rule A and says nothing about whether a test holds a claim, which
    is the distinction rule D turns on.
    """
    entry = ACKNOWLEDGED.get(name)
    return entry is not None and entry.kind == kind

# An item definition. The leading `\b` is load-bearing: without it `retype only`
# in a doc comment resolves as a definition of `only`, because `type` matches
# inside `retype`.
DEFN = r"\b(?:const|static|fn|struct|enum|trait|type|union|mod)\s+{sym}\b|\bmacro_rules!\s+{sym}\b"

# The same keywords, collecting the NAMES instead of asking about one. DEFN
# answers "is this name defined here"; this enumerates what a file defines, for
# production_item_names().
ITEM_DEFN = re.compile(
    r"\b(?:const|static|fn|struct|enum|trait|type|union|mod)\s+([A-Za-z_][A-Za-z0-9_]*)\b"
    r"|\bmacro_rules!\s+([A-Za-z_][A-Za-z0-9_]*)\b"
)

COMMENT = re.compile(r"^\s*(?://!|///|//)\s?(.*)$")

# A Rust item name as a comment writes one. Named once because rules A and D
# both need it and a second spelling would be a second thing to keep true.
SYMBOL = r"[A-Za-z_][A-Za-z0-9_]*"

# `brightfield-sql`'s `SYMBOL`   /   `SYMBOL` in `brightfield-sql`
ATTR_POSSESSIVE = re.compile(rf"`([a-z][a-z0-9-]*)`'s\s+`({SYMBOL})`")
ATTR_IN = re.compile(rf"`({SYMBOL})`\s+in\s+`([a-z][a-z0-9-]*)`")

# A backticked repo-relative path, optionally with :LINE
PATH_REF = re.compile(r"`((?:crates|scripts|examples|vendor|\.github)/[A-Za-z0-9_./-]+?)(?::\d+)?`")

# A package name as crates.io and npm spell one: lowercase, hyphen/underscore
# separated. Uppercase is excluded so a backticked SYMBOL is not read as a
# package — `AGGREGATE_COUNT_COL` is rule A's business, not rule C's.
PKG_NAME = r"[a-z][a-z0-9]*(?:[-_][a-z0-9]+)*"

# A version, in two shapes: `v`-prefixed, or three numeric components. Two bare
# components are excluded deliberately, because durations, thresholds and ranges
# are written that way — "costs ~2.5 ms", "> 1.0", "clamped to [0.0, 1.0]". The
# lookarounds stop `127.0.0.1` from yielding `0.0.1`. Each exclusion is held by
# a control case in --self-test.
PKG_VERSION = r"(?<![\d.])(?:v\d+\.\d+(?:\.\d+)*|\d+\.\d+\.\d+)(?![\d.])"

# The gap excludes backticks, so a name cannot be paired with a version that
# belongs to some other backticked span later in the line.
PKG_GAP = r"[^`]{0,24}?"

# `color-name` (v1.1.4)   /   `color-name 1.1.4`   /   v1.1.4 of `color-name`
PKG_AFTER = re.compile(rf"`({PKG_NAME})`{PKG_GAP}{PKG_VERSION}")
PKG_INLINE = re.compile(rf"`({PKG_NAME})\s+{PKG_VERSION}`")
PKG_BEFORE = re.compile(rf"{PKG_VERSION}{PKG_GAP}`({PKG_NAME})`")

# the `color-name` package   /   package `color-name`
PKG_NOUN = r"(?:package|crate|library)"
PKG_LABELLED = re.compile(rf"`({PKG_NAME})`\s+{PKG_NOUN}\b")
PKG_LABELLED_BEFORE = re.compile(rf"\b{PKG_NOUN}\s+`({PKG_NAME})`")

PKG_PATTERNS = (PKG_AFTER, PKG_INLINE, PKG_BEFORE, PKG_LABELLED, PKG_LABELLED_BEFORE)

# --- rule D ----------------------------------------------------------------

# The quantifiers are the ones this project's prose rule names, plus `none` and
# the two `-thing` forms that say the same in another word order. Each asserts a
# completeness somebody has to enumerate, so each needs the enumeration cited.
#
# The MEMBERSHIP of this alternation is the rule, and dropping a word from it is
# silent: the gate stays green and simply stops seeing that claim. So --self-test
# spells the words out a SECOND time and requires a detection for each. The
# second spelling is deliberate — a fixture list derived from this one would be
# deleted by the same edit it exists to catch. These are English words rather
# than names from this repo, so they travel to a sibling repo as they stand.
#
# `re.I` is here because a claim opening a sentence is capitalised; it is pinned
# by a self-test case whose quantifier is sentence-initial.
#
# The lookarounds exclude a hyphen, which is the difference between a claim and
# an adjective: `read-only`, `colour-only`, `all-null` and `never-run` describe a
# thing rather than asserting anything about how much of it there is.
QUANTIFIER = re.compile(
    r"(?<![-\w])(only|always|every|never|nothing|none|all|everything|anything)(?![-\w])",
    re.I,
)

# `Foo` / `a_b` / `a::B` / `f()` — a backticked token, plus the `::` path and
# the `()` call form a comment writes. Which of these counts as a SYMBOL is
# decided in symbol_citations(), not here.
SYMBOL_TICKED = re.compile(rf"`({SYMBOL}(?:::{SYMBOL})*)(?:\(\))?`")

# Anything between a pair of backticks — code, not prose.
TICKED_SPAN = re.compile(r"`[^`]*`")

# One sentence. `;` ends a segment too: two independent clauses joined by a
# semicolon are two claims, and pairing the quantifier of one with the symbol of
# the other is the false positive this rule can least afford. A control case in
# --self-test puts a quantifier on one side of a `;` and a symbol on the other
# and requires silence, so dropping the `;` from this class reddens the gate
# rather than quietly widening it.
SENTENCE_END = re.compile(r"(?<=[.!?;])\s+")

# The same split for the DISCHARGE only, and deliberately without the `;`. The
# two readings are different sizes on purpose. A claim is read from the smaller
# unit, because pairing one clause's quantifier with another clause's symbol is
# the false positive this rule can least afford. A citation is read from the
# whole sentence, because "X is the only thing that does Y; `t` holds that" is
# one statement to a reader, and the semicolon is where this repo habitually
# puts the citation — including in the comment that shipped false.
STATEMENT_END = re.compile(r"(?<=[.!?])\s+")

# A comment line that begins a new PARAGRAPH rather than continuing the previous
# line's sentence: a list item. An empty comment line does the same and is
# handled in segments(). Rule D joins wrapped lines, and a list is the shape
# where that join would be wrong — two bullets are two claims.
SEGMENT_START = re.compile(r"^\s*(?:[-*+]\s|\d+[.)]\s)")

# The WORD, one of the two ways a paragraph names a test — but only as the
# word ITSELF: `test`, `tests`, `tested`, `testing`, each bounded on both
# sides. A prior version of this pattern anchored only the left side
# (`(?:\b|_)test`), so it matched "test" as a mere PREFIX or an
# underscore-joined FRAGMENT of any longer identifier — `foo_test`,
# `test_foo`, even `testify`, all satisfied it without the identifier
# resolving to anything. That is the defect this closes: an identifier that
# merely SPELLS like a test name is not prose about testing, it is a
# citation, and a citation has to resolve — see LOOKS_NAMED and cites_test().
TEST_WORD = re.compile(r"\b(?:tests?|tested|testing)\b", re.I)

# An identifier-shaped token whose OWN spelling makes it look like it is
# naming a test: `test` as a whole UNDERSCORE-DELIMITED component —
# `foo_test`, `test_foo`, `foo_test_bar` — the shape Rust's snake_case gives
# a test's name. `^`/`$` anchor to the token's own start and end, since this
# runs against one already-extracted identifier, not a paragraph.
#
# Deliberately NOT the plain left-anchored `(?:\b|_)test` this repo's prose
# rule used: that also matches "test" as a mere PREFIX of a longer plain
# word — "testable", "testify" — words with no underscore at all, meaning no
# Rust identifier convention behind them. Requiring the underscore (or the
# token boundary) on BOTH sides is what keeps ordinary English out of a rule
# whose job is deciding whether a NAME must resolve: an English adjective is
# not a citation, and forcing one to resolve is exactly the kind of noise
# that gets a gate switched off (see the module docstring).
#
# A token matching this must resolve; the bare-word forgiveness in
# cites_test() does not reach it, because it is not a mention of testing in
# general, it is a specific, checkable name.
LOOKS_NAMED = re.compile(r"(?:^|_)test(?:_|$)", re.I)

# A BACKTICKED token spelled the way Rust spells a test function: lowercase,
# with at least one underscore. LOOKS_NAMED above only catches a name with the
# word `test` in its own spelling, and the name that shipped false did not have
# one — `the_hero_weight_is_the_pane_share`, in a sentence whose bare word
# `test` then forgave it. A snake_case name a comment puts in backticks is the
# author asserting "this is code"; in the sentence carrying a completeness
# claim it is a citation, so it has to resolve, and the bare word beside it
# does not stand in for it. Uppercase is excluded because `AGGREGATE_COUNT_COL`
# is a const rather than a test, and `::` is excluded with it because a path
# names an item in the production tree.
SNAKE_CITED = re.compile(r"[a-z][a-z0-9]*(?:_[a-z0-9]+)+")

# A test attribute: `#[test]`, `#[tokio::test]`, `#[test(...)]`. NOT `#[cfg(test)]`
# — that marks a module of test-only code, and the helpers inside it are not
# tests a comment can cite as its measurement.
TEST_ATTR = r"#\[(?:[A-Za-z_][A-Za-z0-9_]*::)*test(?:\([^\]]*\))?\]"

# The function such an attribute introduces, across any further attributes
# (`#[ignore = "…"]`, `#[should_panic(…)]`) sitting between the two.
TEST_FN_DEFN = re.compile(
    rf"{TEST_ATTR}\s*(?:#\[[^\]]*\]\s*)*(?:pub\s+)?(?:async\s+)?fn\s+({SYMBOL})"
)

# Any `fn`, anywhere, no attribute required, METHOD or free function. Nothing
# resolves against this pattern directly, because it reads far more than a
# citation: free_fn_names() filters it down to the functions outside every
# `impl` and `trait` block, and resolvable_test_names() filters that again to
# the ones carrying an underscore. What survives both is the shape rule D
# accepts — a helper an integration test calls (`fn collect_inputs(…)`) carries
# no attribute of its own, so TEST_FN_DEFN above never sees it, and a comment
# citing it by name points a reader at something they still find. Read
# resolvable_test_names() for the enumeration itself; this is only its raw
# material.
FN_DEFN_ANY = re.compile(rf"\bfn\s+({SYMBOL})")

# An `impl` or `trait` BLOCK, so its body can be excluded. Anchored to the
# start of a line, which is where Rust puts the keyword and is what keeps
# `-> impl Iterator<…> {` in a return type from being read as one.
IMPL_BLOCK = re.compile(
    r"(?m)^[ \t]*(?:pub(?:\([^)]*\))?\s+)?(?:default\s+)?(?:unsafe\s+)?"
    r"(?:impl|trait)\b[^;{]*\{"
)

# `#[cfg(test)]`, immediately followed (allowing other attributes and `pub`)
# by the `mod NAME {` it gates. Matched so the module's BODY can be found —
# see cfg_test_fn_names() — not so the module itself is treated as a test.
CFG_TEST_MOD = re.compile(
    r"#\[cfg\(test\)\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?"
    r"mod\s+[A-Za-z_][A-Za-z0-9_]*\s*\{"
)

# Any identifier-shaped token, backticked or bare. What rule D resolves against
# the test functions below — this repo writes a cited test name in backticks,
# but a bare one is the same citation. Resolving rather than pattern-matching is
# what stops an ordinary word from standing in for a test.
IDENT = re.compile(SYMBOL)


def crates() -> set[str]:
    d = ROOT / "crates"
    return {p.name for p in d.iterdir() if p.is_dir()} if d.is_dir() else set()


_TEST_FNS: set[str] | None = None


def test_functions() -> set[str]:
    """Every `#[test]` function name in the tree, read once.

    The narrow enumeration: only a function individually carrying a test
    attribute. Used to derive a fixture that IS genuinely a test in
    --self-test; the gate itself resolves against the wider
    resolvable_test_names() below, which this is a subset of.
    """
    global _TEST_FNS
    if _TEST_FNS is None:
        found: set[str] = set()
        for f in sorted(ROOT.glob("crates/**/*.rs")):
            if "target" in f.parts:
                continue
            try:
                found |= set(TEST_FN_DEFN.findall(f.read_text(errors="replace")))
            except OSError:
                pass
        _TEST_FNS = found
    return _TEST_FNS


def _brace_match_end(text: str, start: int) -> int:
    """Index just past the `}` balancing the `{` at `text[start - 1]`.

    A linear scan, not a parser — it does not know a brace inside a string or
    comment from a real one. Test modules do not commonly carry unbalanced
    braces in a string literal, and the cost of being wrong is bounded: a
    name inside the misread span is one this gate resolves that a stricter
    reader would not, which is the direction a citation gate is allowed to be
    wrong in (see the module docstring on precision over recall).
    """
    depth = 1
    i = start
    n = len(text)
    while i < n and depth:
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
        i += 1
    return i


def free_fn_names(text: str) -> set[str]:
    """`fn` names defined OUTSIDE every `impl` and `trait` block in this text.

    A METHOD is not a test. `#[test]` does not apply to one, `cargo test` does
    not run one, and a comment writing `empty_state` is naming the production
    field of that name far more often than a fixture type's method — which is
    how one method on a `#[cfg(test)]` fixture in
    crates/brightfield-workbench/src/item.rs discharged completeness claims in
    three unrelated test files. So the impl and trait spans are found first and
    every `fn` inside one is dropped.

    The spans come from _brace_match_end(), a brace count rather than a parser.
    Its failure mode here is one-directional: a `{` miscounted inside a string
    literal makes a span too long, so a free function after it is mistaken for
    a method and stops resolving, and the gate reports a claim it could have
    discharged. That costs a reader a citation to write, not a false silence.
    """
    spans = [(m.end(), _brace_match_end(text, m.end())) for m in IMPL_BLOCK.finditer(text)]
    return {
        m.group(1)
        for m in FN_DEFN_ANY.finditer(text)
        if not any(s <= m.start() < e for s, e in spans)
    }


def cfg_test_fn_names(text: str) -> set[str]:
    """FREE `fn` names defined inside a `#[cfg(test)]` module in this file's text.

    A helper beside the `#[test]` functions that call it — `fn cp(s: &str) ->
    ComponentPath` next to the tests that use it — is not itself attributed,
    so TEST_FN_DEFN never finds it. This walks the module's brace depth from
    its opening `{` to the matching `}` and reads every `fn` in between.

    Underscore-bearing names only. A short, single-word helper name is a
    real and MEASURED collision risk here in a way it is not for
    test_functions(): a genuine `#[cfg(test)]` helper this tree defines can
    be `fn za(m: &mut ProtocolModel) -> bool` (a synthetic keystroke for a
    protocol test), and a comment naming a keyboard shortcut — "the user
    presses `za`" — cites the SAME token with no test in mind at all. That
    comment's completeness claim resolved silently once this enumeration
    reached it, which is a false NEGATIVE — the gate stops reporting a
    finding it is right to report — and the shape this whole rule exists to
    avoid, just from the opposite direction of the defect the card closes.
    Rust's own snake_case convention makes a multi-word, descriptive helper
    name (`ensure_dataset`, `a_protocol_and_a_chart`) the common case; a bare
    single word is the collision-prone exception, so it is excluded here
    rather than resolved.
    """
    names: set[str] = set()
    for m in CFG_TEST_MOD.finditer(text):
        end = _brace_match_end(text, m.end())
        names |= {n for n in free_fn_names(text[m.end():end]) if "_" in n}
    return names


def is_integration_test(path: Path) -> bool:
    """Is this `crates/<crate>/tests/<name>.rs` — a file Cargo builds as a test?"""
    return path.parent.name == "tests" and path.parent.parent.parent.name == "crates"


def tests_dir_fn_names(text: str) -> set[str]:
    """FREE `fn` names in an integration-test file, underscore-bearing only.

    A `crates/*/tests/*.rs` file is test code all the way down — Cargo builds
    each one as its own test binary — so a free function in it is a test helper
    whether or not it carries an attribute, and a comment citing one by name
    points a reader somewhere real. The same two filters as cfg_test_fn_names()
    apply and for the same measured reasons: a method is not a test, and a
    one-word name is not a citation.
    """
    return {n for n in free_fn_names(text) if "_" in n}


_PRODUCTION: set[str] | None = None


def production_item_names() -> set[str]:
    """Item names this workspace defines OUTSIDE its test code, read once.

    A citation gate cannot tell what a name MEANS, only where it is defined,
    and a name production defines is being cited as production far more often
    than as a measurement. `new_egui_renderer` is a function of
    crates/brightfield-shell/src/capture.rs; a helper in
    crates/brightfield-shell/tests/keyed_canvas.rs shadows it, and that
    coincidence discharged a completeness claim in capture.rs that is plainly
    describing the renderer. `role_of` and `idle_status_entry` did the same in
    three more places. So a name production defines does not resolve through
    the two attribute-free enumerations.

    A `#[test]` function is exempt from this: the attribute is proof the name
    IS a test, whatever else in the tree shares the spelling. Measured here,
    the refusal costs 13 of the 536 names the attribute-free half contributes.
    """
    global _PRODUCTION
    if _PRODUCTION is None:
        found: set[str] = set()
        for f in sorted(ROOT.glob("crates/**/*.rs")):
            if "target" in f.parts or is_integration_test(f):
                continue
            try:
                text = f.read_text(errors="replace")
            except OSError:
                continue
            hidden = [
                (m.start(), _brace_match_end(text, m.end()))
                for m in CFG_TEST_MOD.finditer(text)
            ]
            for m in ITEM_DEFN.finditer(text):
                if any(a <= m.start() < b for a, b in hidden):
                    continue
                found.add(m.group(1) or m.group(2))
        _PRODUCTION = found
    return _PRODUCTION


_RESOLVABLE: set[str] | None = None


def resolvable_test_names() -> set[str]:
    """Every name a comment's cited test can resolve against. Three sets:

      1. a function carrying a `#[test]` of its own, anywhere in the workspace
         — test_functions(), the narrowest and least ambiguous of the three.
      2. a FREE, underscore-bearing function in a `crates/*/tests/*.rs` file —
         tests_dir_fn_names(). An integration test's helper carries no
         attribute, so (1) never sees it.
      3. a FREE, underscore-bearing function in a `#[cfg(test)]` module —
         cfg_test_fn_names().

    Three things are excluded from (2) and (3), and each is the fix for a
    measured false silence rather than a matter of taste:

      * a METHOD, whatever module it sits in. See free_fn_names().
      * a name production also defines, in either half. See
        production_item_names().
      * a name with no underscore. This tree's integration-test files define
        free functions called `nothing`, `parse`, `read`, `run`, `block`,
        `column` and `key`, among many others of the same shape — ordinary
        English words this repo backticks in prose about something else
        entirely, and `block` alone discharged a completeness claim in
        crates/brightfield-shell/tests/data_file.rs that cites it as a field
        of a record. Rust's snake_case convention makes a multi-word name the
        common case for a real helper, so the one-word name is given up rather
        than resolved. --self-test pins this from both directions.
    """
    global _RESOLVABLE
    if _RESOLVABLE is None:
        wider: set[str] = set()
        for f in sorted(ROOT.glob("crates/**/*.rs")):
            if "target" in f.parts:
                continue
            try:
                text = f.read_text(errors="replace")
            except OSError:
                continue
            wider |= cfg_test_fn_names(text)
            if is_integration_test(f):
                wider |= tests_dir_fn_names(text)
        _RESOLVABLE = test_functions() | (wider - production_item_names())
    return _RESOLVABLE


def cites_test(text: str) -> bool:
    """Does this SENTENCE point a reader at a test THAT EXISTS?

    A sentence, not a paragraph. completeness_claims() calls this on the one
    sentence the quantifier falls in, because a test named in a NEIGHBOURING
    sentence holds nothing about this one — and reading the paragraph is what
    let a real citation two sentences away discharge a claim it had never been
    written about.

    Checked in this order:

      1. A name resolves against test_functions() — an individually
         `#[test]`-attributed function, this repo's narrowest and least
         ambiguous enumeration — or ACKNOWLEDGED AS A TEST, which is a kind
         the registry states and not mere membership of it. Checked whether the name is
         backticked or bare, this repo writes both; a descriptive test name
         (`a_band_click_resolves_to_a_structured_point_clause`) is long and
         specific enough that a bare mention is not read as ordinary prose.
      2. Failing that, a BACKTICKED name resolves against the wider
         resolvable_test_names() (a `tests/` helper or a `#[cfg(test)]`
         module function, neither individually `#[test]`-attributed) or
         ACKNOWLEDGED AS A TEST. Backticks required here and not in (1): this
         enumeration is ~3x larger and its names are ordinary, short helper
         names — `path`, `apply` — that DO turn up as bare prose, and a bare
         scan against it exempted claims that never cited anything. A
         backtick is this repo's own convention for "this word is code," and
         restricts the match to what the author marked as one.
      3. Failing both, this sentence carries a citation that did NOT resolve,
         which is a wrong citation rather than an absent one — so it does not
         fall through to (4), and the bare word `test` beside it does not
         excuse it. Two spellings say "this is a name" that loudly:
         a BACKTICKED snake_case token (SNAKE_CITED), and any token whose own
         spelling contains `test` as a component (LOOKS_NAMED — `foo_test`,
         `test_foo`). The first is the one that shipped false:
         `the_hero_weight_is_the_pane_share` names no function in this
         workspace, carries no `test` in its own spelling, and the bare word
         `test` next to it discharged the claim.
      4. Otherwise, the bare word (TEST_WORD) is prose about testing with no
         name to check, and is still forgiven — there is nothing to resolve.
         This is the forgiveness that remains, and it is narrow: it needs a
         sentence with the word `test` and NO backticked snake_case name at
         all, which in practice means a claim whose subject is CamelCase or an
         intra-doc link. Naming the test is what closes it.
    """
    names = IDENT.findall(text)
    if any(n in test_functions() or acknowledged(n, "test") for n in names):
        return True
    ticked = SYMBOL_TICKED.findall(text)
    if any(n in resolvable_test_names() or acknowledged(n, "test") for n in ticked):
        return True
    if any(SNAKE_CITED.fullmatch(n) for n in ticked):
        return False
    if any(LOOKS_NAMED.search(n) and not TEST_WORD.fullmatch(n) for n in names):
        return False
    return bool(TEST_WORD.search(text))


_CRATE_TEXT: dict[str, str] = {}


def crate_text(crate: str) -> str:
    """Every .rs byte in a crate, read once and cached.

    Pure Python on purpose. This used to shell out to `grep -E`, where `\\s`
    and `\\b` are GNU extensions: the self-test passed on macOS and failed on
    the Linux runner, for the same tree. A gate whose verdict depends on which
    grep is installed is worse than no gate, because it is only trustworthy
    where nobody is looking.
    """
    if crate not in _CRATE_TEXT:
        d = ROOT / "crates" / crate
        parts = []
        for f in sorted(d.glob("**/*.rs")):
            if "target" in f.parts:
                continue
            try:
                parts.append(f.read_text(errors="replace"))
            except OSError:
                pass
        _CRATE_TEXT[crate] = "\n".join(parts)
    return _CRATE_TEXT[crate]


def defines(crate: str, symbol: str) -> bool:
    """Is `symbol` really a thing in `crate`?

    Three ways to be one, all found by measuring the false positives this gate
    produced on its first whole-tree run (5 of 7 hits were wrong):

      1. an item definition — `const`/`fn`/`struct`/…  (including in `build.rs`,
         which is where the fixture that exposed the grep divergence lives)
      2. a MODULE or TEST FILE — "`X`'s `sampling` tests" names
         `crates/X/tests/sampling.rs`, not an item
      3. a reserved STRING LITERAL — "`X`'s `__some_alias` alias"

    A bare mention in prose still does not count, which is what keeps the
    original defect — a const attributed to a crate that only *reads* the
    value — detectable.
    """
    d = ROOT / "crates" / crate
    if not d.is_dir():
        return False
    if any(p for p in d.glob(f"**/{symbol}.rs") if "target" not in p.parts):
        return True
    text = crate_text(crate)
    if re.search(DEFN.format(sym=re.escape(symbol)), text):
        return True
    return f'"{symbol}"' in text


_LOCKED: set[str] | None = None


def locked_packages() -> set[str]:
    """Package names Cargo.lock records, normalised so `-` and `_` compare equal.

    Cargo treats the two as the same character in a package name and comments
    write whichever they saw, so `egui-tiles` must resolve against the lock's
    `egui_tiles`.

    A missing lockfile is a LOUD failure rather than an empty set: with no
    enumeration to resolve against, a package citation is reported and the
    reason is not in the message. That is how a gate gets switched off.
    """
    global _LOCKED
    if _LOCKED is None:
        lock = ROOT / "Cargo.lock"
        if not lock.is_file():
            sys.exit(
                "check-comment-citations: a comment cites a package and there is no\n"
                "Cargo.lock to resolve it against. Commit the lockfile, or drop the\n"
                "package rule from this script."
            )
        _LOCKED = {
            m.group(1).replace("-", "_")
            for m in re.finditer(r'^name = "([^"]+)"$', lock.read_text(errors="replace"), re.M)
        }
    return _LOCKED


def package_resolves(name: str) -> bool:
    """Does this repo actually build against a package by that name?

    A workspace crate or a Cargo.lock entry. Anything else is a claim about code
    that is not here, which is what ACKNOWLEDGED is for.
    """
    key = name.replace("-", "_")
    if key in {c.replace("-", "_") for c in crates()}:
        return True
    return key in locked_packages()


def package_citations(body: str) -> list[str]:
    """Package names this comment line cites, in first-seen order."""
    seen: list[str] = []
    for pattern in PKG_PATTERNS:
        for name in pattern.findall(body):
            if name not in seen:
                seen.append(name)
    return seen


def symbol_citations(text: str, file_text: str = "") -> list[str]:
    """Backticked tokens this repo would call a NAMED SYMBOL, in first-seen order.

    A backticked token is not enough. This repo backticks channel names, role
    names, kind ids and column values, and reading those as symbols would make
    rule D report ordinary prose, which is how a gate gets switched off. So a
    token qualifies on one of these marks, each of which a reader uses too, and
    each of which --self-test exercises ALONE so that deleting it reddens:

      * an intra-doc link, `` [`accept`] `` — the brackets say symbol outright,
        which is why lowercase alone does not disqualify a token
      * a `::` path
      * an `_`, or an uppercase letter — `snake_case` and `CamelCase`/`SCREAMING`
        are Rust item names and are not how English words arrive in a sentence
      * the FILE the comment lives in defines an item of that name. This is the
        one mark that resolves rather than guesses, and it is what lets a claim
        about a plain lowercase function — "`accept` has already refused
        anything with a control character" — be seen at all. Scoped to the one
        file on purpose: resolved across the workspace instead, the mark
        degenerates towards "any backticked word", and the sentences it then
        adds are about parameters and literal values. The three arms were
        measured against each other on this tree in the commit that landed
        this mark.

    A lowercase token carrying none of those is left to a reviewer, which is a
    measured miss rather than an oversight — see the module docstring.
    """
    seen: list[str] = []
    for m in SYMBOL_TICKED.finditer(text):
        name = m.group(1)
        linked = (
            m.start() > 0
            and text[m.start() - 1] == "["
            and text[m.end():m.end() + 1] == "]"
        )
        marked = (
            linked
            or "::" in name
            or "_" in name
            or any(c.isupper() for c in name)
            or (bool(file_text) and re.search(DEFN.format(sym=re.escape(name)), file_text))
        )
        if not marked:
            continue
        if name not in seen:
            seen.append(name)
    return seen


def sentences(text: str, end: re.Pattern[str] = SENTENCE_END) -> list[tuple[int, int, str]]:
    """(start, end, sentence) for each sentence of `text`, offsets kept.

    Text that ends mid-sentence yields that fragment as a sentence: a comment
    is prose someone stopped writing, not a grammar.

    `end` is which split — SENTENCE_END for a claim, STATEMENT_END for the
    citation that discharges one. See STATEMENT_END for why they differ.
    """
    spans: list[tuple[int, int, str]] = []
    pos = 0
    for m in end.finditer(text):
        spans.append((pos, m.start(), text[pos:m.start()]))
        pos = m.end()
    spans.append((pos, len(text), text[pos:]))
    return [s for s in spans if s[2].strip()]


def segments(lines: list[tuple[int, str]]) -> list[list[tuple[int, str]]]:
    """A comment block's paragraphs. A wrap continues one; a list starts one.

    Rule D reads a sentence, and a sentence wraps across lines. Joining a whole
    block indiscriminately would also join a bulleted list into one run-on
    "sentence" and pair one item's quantifier with the next item's symbol, so an
    empty comment line or a list marker ends the paragraph.
    """
    out: list[list[tuple[int, str]]] = []
    cur: list[tuple[int, str]] = []
    for lineno, body in lines:
        if not body.strip():
            if cur:
                out.append(cur)
                cur = []
            continue
        if cur and SEGMENT_START.match(body):
            out.append(cur)
            cur = []
        cur.append((lineno, body))
    if cur:
        out.append(cur)
    return out


def joined(segment: list[tuple[int, str]]) -> tuple[str, list[tuple[int, int, int]]]:
    """A paragraph as one string, plus (start, end, lineno) for each line in it.

    The spans are what lets a finding name the line a reader has to open, which
    is the half of a finding they act on.
    """
    parts: list[str] = []
    spans: list[tuple[int, int, int]] = []
    pos = 0
    for lineno, body in segment:
        text = body.strip()
        if parts:
            parts.append(" ")
            pos += 1
        spans.append((pos, pos + len(text), lineno))
        parts.append(text)
        pos += len(text)
    return "".join(parts), spans


class Claim(NamedTuple):
    """One completeness claim: where its quantifier is, and what it spans."""

    lineno: int
    lines: frozenset[int]
    word: str
    symbol: str


def completeness_claims(lines: list[tuple[int, str]], file_text: str = "") -> list[Claim]:
    """The claims a run of contiguous comment lines asserts over a named symbol.

    `file_text` is the source the comment lives in, for the resolving mark in
    symbol_citations().

    A claim is discharged by a test named in the sentence the QUANTIFIER falls
    in — see cites_test(). Reading the paragraph instead, which is what this
    did until the sentence rule, errs towards silence in a way that turned out
    to be the whole defect: a real citation in one sentence discharged a claim
    in another, and a broken citation was discharged by any bare word `test`
    beside it. The two units in play are different sizes and STATEMENT_END says
    why.
    """
    claims: list[Claim] = []
    for segment in segments(lines):
        text, spans = joined(segment)
        statements = sentences(text, STATEMENT_END)
        for start, _end, sentence in sentences(text):
            # The quantifier has to be PROSE. A backticked `all` is a column, a
            # field or a variant this repo happens to have named that, and it
            # asserts nothing — so the backticked spans are blanked before the
            # search, and go back in for the symbol. Blanked to the SAME LENGTH,
            # because the offset is what names the line in the finding.
            prose = TICKED_SPAN.sub(lambda m: " " * len(m.group(0)), sentence)
            q = QUANTIFIER.search(prose)
            if not q:
                continue
            syms = symbol_citations(sentence, file_text)
            if not syms:
                continue
            at = start + q.start()
            # The citation has to sit in the same STATEMENT as the quantifier.
            # Falling back to the claim's own segment keeps a quantifier the
            # statement split cannot place — there is no such text today, and a
            # silent None here would be a claim reported for the wrong reason.
            here = next((s for a, b, s in statements if a <= at < b), sentence)
            if cites_test(here):
                continue
            over = frozenset(
                ln for s, e, ln in spans if s < start + len(sentence) and e > start
            )
            where = next((ln for s, e, ln in spans if s <= at <= e), segment[0][0])
            claims.append(Claim(where, over, q.group(1), syms[0]))
    return claims


def path_exists(ref: str) -> bool:
    """Repo-relative, or crate-relative — comments write both and mean the same.

    `vendor/mosaic-specs/yaml/` is real, at `crates/brightfield-spec/vendor/…`.
    Flagging that as missing is noise; a path that resolves nowhere is not.
    """
    if (ROOT / ref).exists():
        return True
    return any((c / ref).exists() for c in (ROOT / "crates").iterdir() if c.is_dir())


def diff_revisions(repo: Path) -> tuple[str, str]:
    """The two commits a diff run reads: the merge base, and the tip. Or a LOUD failure.

    Resolved to hashes, once, because every other read in the run has to name
    the SAME pair — which files changed, which lines were added, and the file
    content those numbers index into. A line number means nothing except against
    one revision of a file.

    The one thing this gate must never do is pass vacuously. A shallow CI
    checkout has no merge base with origin/main, `git diff` then yields nothing,
    and "citations resolve" would be printed over an unread diff — which is
    precisely how a checker becomes decorative. So an unresolvable base is an
    error, not an empty list.
    """
    subprocess.run(["git", "-C", str(repo), "fetch", "-q", "--no-tags", "origin", "main"],
                   capture_output=True)
    base = subprocess.run(
        ["git", "-C", str(repo), "merge-base", "origin/main", "HEAD"],
        capture_output=True, text=True,
    )
    head = subprocess.run(
        ["git", "-C", str(repo), "rev-parse", "HEAD"],
        capture_output=True, text=True,
    )
    if base.returncode != 0 or not (base.stdout or "").strip():
        sys.exit(
            "check-comment-citations: no merge base with origin/main — refusing to\n"
            "report success over a diff it could not read. In CI this means the\n"
            "checkout is too shallow; set `fetch-depth: 0`."
        )
    if head.returncode != 0 or not (head.stdout or "").strip():
        sys.exit(
            "check-comment-citations: HEAD does not resolve to a commit, so there is\n"
            "no revision to read the diff's content at."
        )
    return base.stdout.strip(), head.stdout.strip()


class Source(NamedTuple):
    """Where a run reads file content — the same place its diff came from.

    A diff names lines by NUMBER, and a number only means something against one
    revision of the file. Taking the changed lines from `git diff` and the
    bodies from the working tree desynchronises the moment an edit is
    uncommitted: everything below the first inserted line is off by as many
    lines as were inserted, and the gate reports findings at lines the author
    never touched while missing the ones they did. So a run carries one Source
    and both halves read through it.

    `rev=None` is the working tree, which is what `--all` audits.
    """

    repo: Path
    rev: str | None = None

    def text(self, path: Path) -> str:
        """The file as this Source has it, read once. Absent reads as empty."""
        key = (self.repo, self.rev, path)
        if key not in _SOURCE_TEXT:
            _SOURCE_TEXT[key] = self._read(path)
        return _SOURCE_TEXT[key]

    def _read(self, path: Path) -> str:
        if self.rev is None:
            try:
                return path.read_text(errors="replace")
            except OSError:
                return ""
        rel = path.relative_to(self.repo).as_posix()
        out = subprocess.run(
            ["git", "-C", str(self.repo), "show", f"{self.rev}:{rel}"],
            capture_output=True,
        )
        if out.returncode != 0:
            return ""
        return out.stdout.decode(errors="replace")


_SOURCE_TEXT: dict[tuple[Path, str | None, Path], str] = {}

WORKTREE = Source(ROOT)


def changed_files(repo: Path, base: str, head: str) -> list[Path]:
    """The `.rs` files the diff between these two commits touched."""
    out = subprocess.run(
        ["git", "-C", str(repo), "diff", "--name-only", "--diff-filter=d", base, head],
        capture_output=True,
        text=True,
    )
    return [repo / f for f in (out.stdout or "").split() if f.endswith(".rs")]


def added_comment_lines(path: Path, repo: Path, base: str, head: str) -> list[tuple[int, str]]:
    """(line-number, comment-body) for lines this diff ADDED, numbered at `head`."""
    rel = path.relative_to(repo)
    out = subprocess.run(
        ["git", "-C", str(repo), "diff", "-U0", base, head, "--", str(rel)],
        capture_output=True,
        text=True,
    )
    hits: list[tuple[int, str]] = []
    lineno = 0
    for line in (out.stdout or "").splitlines():
        hunk = re.match(r"^@@ -\d+(?:,\d+)? \+(\d+)", line)
        if hunk:
            lineno = int(hunk.group(1))
            continue
        if line.startswith("+") and not line.startswith("+++"):
            m = COMMENT.match(line[1:])
            if m:
                hits.append((lineno, m.group(1)))
            lineno += 1
    return hits


def all_comment_lines(path: Path, source: Source) -> list[tuple[int, str]]:
    hits = []
    for i, line in enumerate(source.text(path).splitlines(), 1):
        m = COMMENT.match(line)
        if m:
            hits.append((i, m.group(1)))
    return hits


class Unit(NamedTuple):
    """One contiguous run of comment lines, and which of them are being checked.

    Rules A, B and C read a line. Rule D reads a sentence, which can wrap, so it
    needs the neighbours of the line in question even when they are unchanged.
    Reading them is not checking them: every rule here reports at a line in
    `scope`, D at the line its quantifier is on, so an unchanged neighbour can
    complete a sentence but cannot be the reason one is reported.

    `source` is where the bodies came from, and it stays with the unit: check()
    reads the surrounding file through it for the resolving mark, and names the
    finding relative to its repo.
    """

    path: Path
    lines: list[tuple[int, str]]
    scope: frozenset[int]
    source: Source = WORKTREE


def comment_blocks(lines: list[tuple[int, str]]) -> list[list[tuple[int, str]]]:
    """Split comment lines into runs of consecutive line numbers."""
    out: list[list[tuple[int, str]]] = []
    cur: list[tuple[int, str]] = []
    for lineno, body in lines:
        if cur and lineno != cur[-1][0] + 1:
            out.append(cur)
            cur = []
        cur.append((lineno, body))
    if cur:
        out.append(cur)
    return out


def units(path: Path, source: Source, scope: set[int] | None = None) -> list[Unit]:
    """The comment blocks of a file as `source` has it, restricted to `scope`.

    `scope=None` means every comment line, which is what `--all` audits.
    """
    blocks = comment_blocks(all_comment_lines(path, source))
    out = []
    for block in blocks:
        here = {n for n, _ in block}
        keep = here if scope is None else here & scope
        if keep:
            out.append(Unit(path, block, frozenset(keep), source))
    return out


def diff_units(repo: Path) -> list[Unit]:
    """The comment blocks this branch's diff touched, read at the tip it diffs to.

    The one place the diff and the content are paired. Both come from
    `diff_revisions`, so neither can be taken from somewhere the other was not.
    """
    base, head = diff_revisions(repo)
    source = Source(repo, head)
    return [
        u
        for f in changed_files(repo, base, head)
        for u in units(f, source, {n for n, _ in added_comment_lines(f, repo, base, head)})
    ]


def check(units: list[Unit]) -> list[str]:
    known = crates()
    failures: list[str] = []
    for unit in units:
        path, rel = unit.path, unit.path.relative_to(unit.source.repo)
        source = unit.source.text(path)
        for lineno, body in unit.lines:
            if lineno not in unit.scope:
                continue
            for crate, symbol in (
                [(c, s) for c, s in ATTR_POSSESSIVE.findall(body)]
                + [(c, s) for s, c in ATTR_IN.findall(body)]
            ):
                if crate not in known or acknowledged(symbol, "symbol"):
                    continue
                if not defines(crate, symbol):
                    where = [k for k in known if defines(k, symbol)]
                    hint = f" — it is defined in {', '.join(where)}" if where else " — no crate defines it"
                    failures.append(f"{rel}:{lineno} attributes `{symbol}` to `{crate}`{hint}")
            for ref in PATH_REF.findall(body):
                if not path_exists(ref):
                    failures.append(f"{rel}:{lineno} cites `{ref}`, which does not exist")
            for name in package_citations(body):
                if acknowledged(name, "package") or package_resolves(name):
                    continue
                failures.append(
                    f"{rel}:{lineno} cites the `{name}` package, which is not a crate "
                    f"here and not in Cargo.lock"
                )
        for claim in completeness_claims(unit.lines, source):
            # The QUANTIFIER's own line has to be one this run is checking —
            # the same test the three rules above apply to the line they read,
            # and it is the line the finding names. Admitting on any line the
            # sentence spans reports an old claim on a diff that only reworded
            # the prose under it, which is blocking on debt the author did not
            # write. Held by the wrapped-quantifier pairs in --self-test.
            if claim.lineno not in unit.scope:
                continue
            failures.append(
                f"{rel}:{claim.lineno} says \"{claim.word}\" of `{claim.symbol}` and "
                f"names no test — cite the test that holds it, or drop the quantifier"
            )
    return failures


def revision_guards() -> list[tuple[bool, str]]:
    """Gate a repo whose working tree has DRIFTED from HEAD, and check where it reports.

    Built here rather than derived from this checkout, because the shape needs
    an uncommitted edit and this gate does not get to dirty the tree it is
    checking. Two commits and one unstaged insertion:

        origin/main    two lines of code, no comments
        HEAD           adds a comment that claims nothing, and below it a
                       comment that claims something
        working tree   inserts one more comment ABOVE both, so the committed
                       text below it sits one line lower than the diff says

    Read the diff at HEAD and the content from disk and the two halves read
    different files under one name: the claim the branch added is missed, and
    the line number the diff reported lands on the uncommitted insertion
    instead — a finding at a line the author never wrote. Read both at HEAD and
    the insertion is invisible, which is what these two assert.
    """
    claim = "// every path through here re-checks [`Widget`]"
    head_lines = [
        "fn a() {}",
        "fn b() {}",
        "// [`Widget`] is rebuilt at boot",
        "fn c() {}",
        "",
        claim,
        "fn d() {}",
    ]
    drift = "// nothing else writes [`Widget`] here"
    disk_lines = head_lines[:2] + [drift] + head_lines[2:]
    at = head_lines.index(claim) + 1
    moved = disk_lines.index(drift) + 1

    with tempfile.TemporaryDirectory() as tmp:
        repo = Path(tmp).resolve()

        def git(*args: str) -> None:
            subprocess.run(
                [
                    "git", "-C", str(repo),
                    "-c", "user.name=citation gate",
                    "-c", "user.email=gate@invalid",
                    "-c", "commit.gpgsign=false",
                    "-c", f"core.hooksPath={repo / 'absent-hooks'}",
                    *args,
                ],
                capture_output=True,
                check=True,
            )

        rel = "src/a.rs"
        rs = repo / rel
        rs.parent.mkdir(parents=True)
        git("init", "-q")
        rs.write_text("\n".join(head_lines[:2]) + "\n")
        git("add", rel)
        git("commit", "-qm", "base")
        base = subprocess.run(
            ["git", "-C", str(repo), "rev-parse", "HEAD"], capture_output=True, text=True
        ).stdout.strip()
        git("update-ref", "refs/remotes/origin/main", base)
        rs.write_text("\n".join(head_lines) + "\n")
        git("add", rel)
        git("commit", "-qm", "head")
        rs.write_text("\n".join(disk_lines) + "\n")

        found = check(diff_units(repo))

    return [
        (
            any(m.startswith(f"{rel}:{at} ") for m in found),
            f"an uncommitted insertion does not move a finding: the claim the diff "
            f"added is reported at its committed line ({rel}:{at})",
        ),
        (
            len(found) == 1 and not any(m.startswith(f"{rel}:{moved} ") for m in found),
            f"and nothing is reported at {rel}:{moved}, where reading the content "
            f"from disk would have put a claim the diff never added",
        ),
    ]


def resolver_stub_guard() -> list[tuple[bool, str]]:
    """Prove --self-test would catch a resolver rubber-stamping every name.

    A resolver that always says yes is this rule's silent-failure mode: the
    gate stays green and simply stops checking a specific name — exactly the
    defect this card closes, just moved one level down, into the checker
    that is meant to prove the checker works. Written out rather than
    derived, like the QUANTIFIER guards above: it tests the WIRING between
    cites_test() and resolvable_test_names(), not a shape found in this
    tree's own crates.

    Stub resolvable_test_names() to accept anything, run ONE fixture whose
    cited name does not exist, and require it to wrongly PASS under the
    stub — proving the rejection is pinned to what the resolver decides, not
    merely to the shape of an unresolved name.
    """

    class _Everything:
        def __contains__(self, _item: object) -> bool:
            return True

    body = "[`Widget`] is the only thing that makes one; `not_a_real_test_case` holds that"
    probe = ROOT / "src" / "__resolver_stub_guard_probe__.rs"
    unit = Unit(probe, [(1, body)], frozenset({1}))

    honest = check([unit])

    module = sys.modules[__name__]
    real = getattr(module, "resolvable_test_names")
    setattr(module, "resolvable_test_names", lambda: _Everything())
    try:
        stubbed = check([unit])
    finally:
        setattr(module, "resolvable_test_names", real)

    return [
        (
            bool(honest),
            "the real resolver reports the fixture's unresolved name as a finding",
        ),
        (
            not stubbed,
            "and a resolver stubbed to accept every name lets the same fixture "
            "through — this case is pinned to the resolver's answer, not to the "
            "name's own shape",
        ),
    ]


def acknowledged_kind_guard() -> list[tuple[bool, str]]:
    """Prove rule D spends the registry's KIND, not mere membership of it.

    ACKNOWLEDGED holds names this repo does not contain, registered for three
    different rules. Read as one flat set it discharged a completeness claim
    over `queryFieldInfo` — a mosaic function registered so that prose ABOUT
    mosaic resolves — as if a test had been named, in a sentence whose three
    real citations resolve to nothing.

    Written out rather than derived, and for a reason worth stating: there is
    no `test`-kind entry in the registry today, because nothing in this tree
    cites a test outside the workspace. So the positive half cannot come from
    the registry. One fixture name is registered here as a symbol, checked,
    registered as a test, checked again, and removed — the SAME name and the
    SAME sentence, so the only thing that moved is the kind.
    """
    name = "aNameThisTreeDoesNotContain"
    body = f"[`Widget`] is the only thing that makes one; `{name}` holds that"
    probe = ROOT / "src" / "__acknowledged_kind_guard_probe__.rs"
    unit = Unit(probe, [(1, body)], frozenset({1}))

    if name in ACKNOWLEDGED:
        return [(False, f"the guard's fixture name `{name}` is already registered, "
                        f"so it cannot show what registering it does")]

    def under(kind: str) -> list[str]:
        ACKNOWLEDGED[name] = Ack(kind, "guard fixture, removed before this returns")
        try:
            return check([unit])
        finally:
            del ACKNOWLEDGED[name]

    as_symbol = under("symbol")
    as_test = under("test")

    return [
        (
            bool(as_symbol),
            "a registered external SYMBOL does not discharge a completeness claim",
        ),
        (
            not as_test,
            "and the same name registered as an external TEST does — rule D spends "
            "the registry's kind, not membership of it",
        ),
        (
            name not in ACKNOWLEDGED,
            "and the guard leaves the registry as it found it",
        ),
    ]


def self_test() -> int:
    """Prove the gate detects — on fixtures DERIVED from this repo, not named.

    The first port of this script to a second repo failed 6 of 10 cases purely
    because the fixtures named the first repo's crates. Hardcoded fixtures do
    not travel; derived ones do. If the repo is too small to derive a case, that
    is a loud failure rather than a silent skip.
    """
    known = sorted(crates())
    if len(known) < 2:
        print("self-test: need >= 2 crates to derive an attribution case")
        return 1

    # A symbol one crate defines and another does not — the real defect shape.
    sym = home = other = None
    for c in known:
        for f in sorted((ROOT / "crates" / c).glob("**/*.rs")):
            for m in re.finditer(r"\bconst\s+([A-Z][A-Z0-9_]{4,})\b", f.read_text(errors="replace")):
                cand = m.group(1)
                # The fixture must be valid under the SAME resolver the test
                # exercises, not merely under the regex that found it. When
                # those two disagreed, the self-test passed locally and failed
                # in CI, and the derivation had no way to notice.
                if not defines(c, cand):
                    continue
                elsewhere = [k for k in known if k != c and defines(k, cand)]
                if len(elsewhere) < len(known) - 1:
                    wrong = next((k for k in known if k != c and not defines(k, cand)), None)
                    if wrong:
                        sym, home, other = cand, c, wrong
                        break
            if sym:
                break
        if sym:
            break
    if not sym or not home or not other:
        print("self-test: could not derive a crate-exclusive symbol")
        return 1

    real_file = next(iter(sorted((ROOT / "crates" / home).glob("**/*.rs")))).relative_to(ROOT)
    missing = f"crates/{home}/src/definitely_not_here_{abs(hash(home)) % 9973}.rs"

    cases = [
        (f"`{other}`'s `{sym}` holds it", False,
         f"a const attributed to a crate that does not define it ({sym} -> {other})"),
        (f"`{sym}` in `{other}` holds it", False,
         "the same claim in the other word order"),
        (f"`{home}`'s `{sym}` holds it", True,
         f"STAYS GREEN: the true attribution ({sym} -> {home})"),
        (f"see `{real_file}:1` for why", True,
         "STAYS GREEN: a real path with a line number"),
        (f"see `{missing}` for why", False,
         "a cited file that is not there"),
        (f"`{home}`'s `TotallyMadeUpThing_{abs(hash(sym)) % 97}` is used", False,
         "a symbol no crate defines at all"),
    ]
    # --- the three PRECISION fixtures -------------------------------------
    # Each was a false positive on this gate's first whole-tree run, when it
    # scored 7 hits of which 5 were wrong. They are regression tests for the
    # noise, and a gate that cries wolf gets switched off — so they are derived
    # here too rather than named, which is how they went missing when this
    # script was first ported to a second repo.
    test_file = next(iter(sorted(ROOT.glob("crates/*/tests/*.rs"))), None)
    if test_file is None:
        print("self-test: no crates/*/tests/*.rs to derive the test-file case from")
        return 1
    test_crate, test_stem = test_file.parts[-3], test_file.stem

    literal = lit_crate = None
    for c in known:
        for f in sorted((ROOT / "crates" / c).glob("**/*.rs")):
            text = f.read_text(errors="replace")
            m = re.search(r'"([a-z_][a-z0-9_]{4,})"', text)
            # Wanted: a token present as a QUOTED STRING but not as an item.
            # `defines()` resolves it through the literal branch, which is the
            # behaviour under test — so check the item pattern directly here,
            # not `defines()`, or the condition is circular and never holds.
            if m and not re.search(
                DEFN.format(sym=re.escape(m.group(1))),
                text,
            ):
                literal, lit_crate = m.group(1), c
                break
        if literal:
            break
    if literal is None:
        print("self-test: no crate-local string literal to derive the literal case from")
        return 1

    rel_path = None
    for c in known:
        for cand in ("src/lib.rs", "src/main.rs", "tests"):
            if (ROOT / "crates" / c / cand).exists() and not (ROOT / cand).exists():
                rel_path = cand
                break
        if rel_path:
            break
    if rel_path is None:
        print("self-test: no crate-relative path to derive the path case from")
        return 1

    cases += [
        (f"its own guard lives in `{test_crate}`'s `{test_stem}` tests", True,
         f"STAYS GREEN: a test file, not an item ({test_crate}/tests/{test_stem}.rs)"),
        (f"must match `{lit_crate}`'s `{literal}` alias", True,
         f"STAYS GREEN: a string literal, not a const (\"{literal}\")"),
        (f"read from `{rel_path}`", True,
         f"STAYS GREEN: a crate-relative path, real under crates/*/{rel_path}"),
    ]

    ack_symbol = next((n for n, a in ACKNOWLEDGED.items() if a.kind == "symbol"), None)
    if ack_symbol is None:
        print("self-test: no ACKNOWLEDGED symbol-kind entry to derive the escape-hatch "
              "case from")
        return 1
    cases.append((f"`{ack_symbol}` is called here", True,
                  f"STAYS GREEN: ACKNOWLEDGED as an external symbol "
                  f"({ACKNOWLEDGED[ack_symbol].reason})"))

    # --- the PACKAGE fixtures ---------------------------------------------
    # The defect these exist for carries no symbol, so the cases above cannot
    # reach it: a colour table sourced to a named package at a named version
    # went through this gate green. The rejection cases below are what fail when
    # the package rule is removed.
    #
    # Derived, not named: the negative name is checked against `package_resolves`
    # — the same resolver the case exercises — because a fixture that turns out
    # to BE a dependency would pass for the wrong reason and read as detection.
    absent = "not-a-dependency-of-this-repo"
    if package_resolves(absent) or absent in ACKNOWLEDGED:
        print(f"self-test: {absent} resolves here, so it cannot serve as the negative case")
        return 1

    dep = next(
        (
            n
            for n in sorted(locked_packages())
            if re.fullmatch(PKG_NAME, n)
            and n not in ACKNOWLEDGED
            and n.replace("-", "_") not in {c.replace("-", "_") for c in known}
        ),
        None,
    )
    if dep is None:
        print("self-test: no Cargo.lock package to derive the resolvable-package case from")
        return 1

    ack_pkg = next(
        (
            n
            for n, a in ACKNOWLEDGED.items()
            if a.kind == "package" and re.fullmatch(PKG_NAME, n) and not package_resolves(n)
        ),
        None,
    )
    if ack_pkg is None:
        print("self-test: no ACKNOWLEDGED package-kind name to derive the escape-hatch case from")
        return 1

    cases += [
        (f"values were taken from the `{absent}` package (v1.1.4)", False,
         f"a bare upstream-package claim, no symbol to attribute ({absent})"),
        (f"ported from `{absent} 1.1.4`", False,
         "the same claim with the version inside the backticks"),
        (f"v1.1.4 of `{absent}` is the source", False,
         "the same claim with the version first"),
        (f"cross-checked against the `{absent}` crate", False,
         "labelled a crate, with no version at all"),
        (f"`{dep}` v1.1.4 behaves this way", True,
         f"STAYS GREEN: a package Cargo.lock records ({dep})"),
        (f"`{home}` v1.1.4 behaves this way", True,
         f"STAYS GREEN: a workspace crate ({home})"),
        (f"`{ack_pkg}` v1.1.4 was evaluated and rejected", True,
         f"STAYS GREEN: ACKNOWLEDGED as an external package "
         f"({ACKNOWLEDGED[ack_pkg].reason})"),
        (f"one `{absent}` sample costs ~2.5 ms", True,
         "STAYS GREEN: a duration is not a version"),
        (f"`{absent}` clamps to [0.0, 1.0]", True,
         "STAYS GREEN: a range is not a version"),
        (f"`{absent}` binds 127.0.0.1:1 to refuse connections", True,
         "STAYS GREEN: an address is not a version"),
    ]

    # A real `#[test]` function whose own NAME does not contain the word
    # `test`, so a case citing it is pinned to the resolution and not to the
    # word. Derived here rather than in the named-test block below because the
    # completeness fixtures need it too: the sentence rule is pinned by putting
    # this name in the claim's sentence and then in a neighbouring one.
    named_test = next(
        (n for n in sorted(test_functions()) if not TEST_WORD.search(n)),
        None,
    )
    if named_test is None:
        print("self-test: no test function whose own name lacks the word `test`, so the "
              "named-test case cannot be told apart from the word case")
        return 1

    # --- the COMPLETENESS fixtures ----------------------------------------
    # Rule D judges no claim, so its fixtures are about where it stops. The
    # rejections are shapes a review wave in this repo corrected by hand; the
    # controls are sentences from beside them that were correct.
    #
    # The material below is derived so that each MARK in symbol_citations()
    # gets a token qualifying through that mark ALONE. Deleting a mark then
    # reddens one named case instead of none — which is what a gate whose marks
    # were unpinned looks like from the outside: green, and blind.
    # A fixture name must not be one rule D would read as a citation, or the
    # case it is meant to redden exempts itself. `cites_test`, not the word
    # pattern alone: a derived `fn` name can BE a test function, and then the
    # paragraph built from it goes green for a reason the label does not say.
    def usable(name: str) -> bool:
        # ...and not a name the RESOLVER would accept either. The enumeration
        # reaches into `tests/` files and `#[cfg(test)]` modules, which is where
        # this derivation is reading, so a fixture drawn from one can be a name
        # rule D discharges on — and the case built from it then goes green for
        # a reason its label does not say.
        return (
            not cites_test(name)
            and not QUANTIFIER.fullmatch(name)
            and name not in resolvable_test_names()
            and not acknowledged(name, "test")
        )

    # The probe file is a REAL file, because the resolving mark reads the source
    # the comment sits in. A fixture written against a path that does not exist
    # cannot reach that mark at all.
    probe = lc_local = None
    for c in known:
        for f in sorted((ROOT / "crates" / c).glob("**/*.rs")):
            for n in re.findall(r"\bfn ([a-z][a-z0-9]{2,})\b", f.read_text(errors="replace")):
                if usable(n):
                    probe, lc_local = f, n
                    break
            if probe:
                break
        if probe:
            break
    if probe is None:
        print("self-test: no lowercase fn to derive the file-resolved-symbol case from")
        return 1
    probe_text = WORKTREE.text(probe)

    def defined_in_probe(name: str) -> bool:
        return bool(re.search(DEFN.format(sym=re.escape(name)), probe_text))

    # CamelCase, snake_case and a lowercase `a::b` path — one per lexical mark.
    # Each is required NOT to be an item of the probe file, or the resolving
    # mark would hold it up after its own mark was deleted and the mutation
    # would stay green.
    camel = snake = None
    lc_names: list[str] = []
    for c in known:
        for f in sorted((ROOT / "crates" / c).glob("**/*.rs")):
            text = f.read_text(errors="replace")
            if camel is None:
                camel = next((n for n in re.findall(r"\b(?:struct|enum|trait) ([A-Z][a-z][A-Za-z0-9]*)\b", text)
                              if usable(n) and not defined_in_probe(n)), None)
            if snake is None:
                snake = next((n for n in re.findall(r"\bfn ([a-z][a-z0-9]*_[a-z0-9_]+)\b", text)
                              if usable(n) and not defined_in_probe(n)), None)
            lc_names += [n for n in re.findall(r"\bmod ([a-z][a-z0-9]{2,})\b", text) if usable(n)]
            if camel and snake and len(lc_names) >= 1:
                break
        if camel and snake and lc_names:
            break
    if camel is None or snake is None or not lc_names:
        print("self-test: could not derive a CamelCase, a snake_case and a module name")
        return 1
    lc_path = f"{lc_names[0]}::{lc_local}"

    # `plain` is a lowercase word with no `_`, no capital and no definition in
    # the probe file: the token rule D deliberately does not read as a symbol.
    # Derived from a string literal in the tree rather than named, for the same
    # reason as the fixtures above.
    plain = None
    for c in known:
        for f in sorted((ROOT / "crates" / c).glob("**/*.rs")):
            plain = next((n for n in re.findall(r'"([a-z][a-z0-9]{2,})"', f.read_text(errors="replace"))
                          if usable(n) and not defined_in_probe(n) and n != lc_local), None)
            if plain:
                break
        if plain:
            break
    if plain is None:
        print("self-test: no plain lowercase literal to derive the not-a-symbol case from")
        return 1

    # One sentence per word of the QUANTIFIER alternation. Spelled out here, not
    # read from the pattern: a list derived from the thing under test is deleted
    # by the same edit it exists to catch. The subject is intra-doc-linked so
    # these cases turn on the quantifier and not on which symbol mark survives.
    quantified = {
        "only": f"[`{camel}`] is the only thing that makes one",
        "always": f"[`{camel}`] is always rebuilt before the first frame",
        "every": f"[`{camel}`] is re-checked on every path through here",
        "never": f"[`{camel}`] never carries a URL scheme",
        "nothing": f"nothing else writes [`{camel}`]",
        "none": f"of the shapes above, none reaches [`{camel}`]",
        "all": f"[`{camel}`] covers all of the kinds we ship",
        "everything": f"everything else falls to [`{camel}`]",
        "anything": f"[`{camel}`] refuses anything carrying a control character",
    }
    cases += [
        (body, False, f"the quantifier `{word}` is read as a claim", (f'"{word}"',))
        for word, body in quantified.items()
    ]

    cases += [
        # Detection, one per mark, each qualifying through that mark alone and
        # each carrying the same quantifier — so a mutation of the alternation
        # reddens the block above and a mutation of a mark reddens one line
        # here. One mutation, one set of names to read.
        (f"`{camel}` is the only thing that makes one", False,
         f"a capital marks a symbol ({camel})", (f"`{camel}`",)),
        (f"`{snake}` is the only thing re-checked on this path", False,
         f"an underscore marks a symbol ({snake})", (f"`{snake}`",)),
        (f"`{lc_path}` is the only route in from here", False,
         f"a `::` marks a symbol ({lc_path})", (f"`{lc_path}`",)),
        (f"[`{plain}`] is the only thing that refuses a URL scheme", False,
         f"an intra-doc link marks a symbol even in lowercase ([`{plain}`])",
         (f"`{plain}`",)),
        (f"`{lc_local}` is the only thing that makes one", False,
         f"a lowercase word this file DEFINES is a symbol ({lc_local} in {probe.name})",
         (f"`{lc_local}`",)),
        (f"Nothing else writes [`{camel}`]", False,
         "a quantifier opening a sentence is capitalised, and still a claim",
         ('"Nothing"',)),
        # Controls.
        (f"[`{camel}`] is the only thing that makes one; the `{named_test}` test "
         f"holds that", True,
         f"STAYS GREEN: a name that RESOLVES, with the bare word beside it "
         f"({named_test})"),
        (f"[`{camel}`] is the only thing that makes one. `{named_test}` holds that",
         False, f"a test named in the NEXT sentence discharges nothing: the claim\'s "
                f"own sentence is the unit ({named_test})",
         ('"only"', f"`{camel}`")),
        ("nothing else can be what reddens", True,
         "STAYS GREEN: a quantifier with no symbol is prose this cannot judge"),
        (f"[`{camel}`] states where that stops", True,
         "STAYS GREEN: a symbol with no quantifier claims no completeness"),
        (f"nothing else makes one. [`{camel}`] states where that stops", True,
         "STAYS GREEN: quantifier and symbol in different sentences"),
        (f"`{plain}` stays the only one of those", True,
         f"STAYS GREEN: a lowercase word this file does not define stays prose (`{plain}`)"),
        (f"the `{camel}` column is named `all` in the source", True,
         "STAYS GREEN: a backticked quantifier is a token, and asserts nothing"),
        (f"[`{camel}`] is read-only from here", True,
         "STAYS GREEN: a leading hyphen makes a quantifier an adjective (read-only)"),
        (f"[`{camel}`] is rebuilt from the all-null column", True,
         "STAYS GREEN: a trailing hyphen does too (all-null)"),
        (f"nothing else writes it; [`{camel}`] is rebuilt before the first frame", True,
         "STAYS GREEN: `;` ends a claim, so one clause's quantifier does not "
         "reach the next clause's symbol"),
        (f"The kinds are listed above. [`{camel}`] is the only thing that makes one",
         False, "a claim in the SECOND sentence of a line is still a claim",
         ('"only"', f"`{camel}`")),
    ]

    # --- the NAMED-TEST fixtures ------------------------------------------
    # Rule D asks an author to NAME the test. These pin that the name is what
    # satisfies it: the fixture is a real test function of this tree, cited in a
    # paragraph carrying no other reference to a test, and the negative is a
    # real function that is not one — so a rule that merely accepted anything
    # snake_case would redden here rather than pass quietly.
    cases += [
        (f"[`{camel}`] is the only thing that makes one; `{named_test}` holds that", True,
         f"STAYS GREEN: a test named by its function, no word `test` anywhere "
         f"({named_test})"),
        (f"[`{camel}`] is the only thing that makes one; held by {named_test}", True,
         "STAYS GREEN: the same citation without the backticks"),
        (f"[`{camel}`] is the only thing that makes one; `{snake}` holds that", False,
         f"a function that is NOT a test does not stand in for one ({snake})",
         ('"only"', f"`{camel}`")),
    ]

    # --- the RESOLVED-NAME fixtures ----------------------------------------
    # This is the defect the card exists to close: a completeness claim used
    # to be discharged by ANY paragraph carrying the word `test`, whether or
    # not the specific name it named existed. These pin the fix at both
    # halves — a name that resolves passes, a name that does not fails, EVEN
    # WHEN the bare word `test` sits right beside it — and separately pin the
    # broadened enumeration (a `tests/`-dir or `#[cfg(test)]`-module helper,
    # neither individually `#[test]`-attributed) that makes the wider
    # resolution meaningful, and the backtick it requires.
    fake_test_name = f"definitely_not_a_real_test_{abs(hash(home)) % 9973}"
    if fake_test_name in resolvable_test_names() or fake_test_name in ACKNOWLEDGED:
        print(f"self-test: {fake_test_name} resolves here, so it cannot serve as the "
              f"unresolved-name negative case")
        return 1

    # A name of the SAME snake_case shape whose own spelling carries no `test`
    # component, so LOOKS_NAMED never fires on it. This is the reviewer\'s
    # reproduction: the fake name was forgiven because the only thing the gate
    # would have insisted on resolving was a name that LOOKED like a test, and
    # this one does not.
    fake_plain_name = f"the_shape_this_gate_missed_{abs(hash(other)) % 9973}"
    if fake_plain_name in resolvable_test_names() or fake_plain_name in ACKNOWLEDGED:
        print(f"self-test: {fake_plain_name} resolves here, so it cannot serve as the "
              f"not-test-shaped negative case")
        return 1
    if LOOKS_NAMED.search(fake_plain_name) or TEST_WORD.search(fake_plain_name):
        print(f"self-test: {fake_plain_name} spells like a test, so it cannot tell the "
              f"spelling rule apart from the resolution rule")
        return 1

    # The three enumerations resolvable_test_names() unions, kept APART here so
    # a fixture can name WHICH one it exercises, and so a case meant to pin the
    # `tests/`-file half cannot pass through the `#[cfg(test)]` half instead.
    # The short and method pools are what the resolver\'s two filters refuse:
    # deleting a filter turns exactly the case built from that pool green.
    cfg_free: set[str] = set()
    cfg_short: set[str] = set()
    cfg_method: set[str] = set()
    tests_free: set[str] = set()
    tests_short: set[str] = set()
    for f in sorted(ROOT.glob("crates/**/*.rs")):
        if "target" in f.parts:
            continue
        text = f.read_text(errors="replace")
        for m in CFG_TEST_MOD.finditer(text):
            body = text[m.end():_brace_match_end(text, m.end())]
            free = free_fn_names(body)
            cfg_free |= {n for n in free if "_" in n}
            cfg_short |= {n for n in free if "_" not in n}
            cfg_method |= {n for n in FN_DEFN_ANY.findall(body) if n not in free and "_" in n}
        if f.parent.name == "tests" and f.parent.parent.parent.name == "crates":
            free = free_fn_names(text)
            tests_free |= {n for n in free if "_" in n}
            tests_short |= {n for n in free if "_" not in n}

    def fixture(pool: set[str], *, resolves: bool) -> str | None:
        """A name from `pool` rule D reads as an ordinary citation, and nothing else.

        `resolves` is asserted against the real resolver rather than assumed
        from which pool the name came out of: a case whose fixture the resolver
        disagrees about passes for a reason its label does not state, which is
        how a derived fixture goes quietly wrong.
        """
        return next(
            (
                n
                for n in sorted(pool)
                if not TEST_WORD.search(n)
                and not LOOKS_NAMED.search(n)
                and not QUANTIFIER.fullmatch(n)
                and n not in test_functions()
                and not acknowledged(n, "test")
                and (n in resolvable_test_names()) == resolves
            ),
            None,
        )

    derived = {
        "a `#[cfg(test)]`-module helper": fixture(cfg_free - tests_free, resolves=True),
        "a `tests/`-file helper": fixture(tests_free - cfg_free, resolves=True),
        "a one-word `#[cfg(test)]` helper": fixture(cfg_short, resolves=False),
        "a one-word `tests/`-file helper": fixture(tests_short, resolves=False),
        "a method in a `#[cfg(test)]` impl": fixture(
            cfg_method - cfg_free - tests_free, resolves=False
        ),
        "a test-side name production also defines": fixture(
            (cfg_free | tests_free) & production_item_names(), resolves=False
        ),
    }
    if any(n is None for n in derived.values()):
        print("self-test: could not derive " + "; ".join(
            what for what, n in derived.items() if n is None
        ))
        return 1
    helper = derived["a `#[cfg(test)]`-module helper"]
    tests_helper = derived["a `tests/`-file helper"]
    short_cfg = derived["a one-word `#[cfg(test)]` helper"]
    short_tests = derived["a one-word `tests/`-file helper"]
    method = derived["a method in a `#[cfg(test)]` impl"]
    shadowed = derived["a test-side name production also defines"]

    cases += [
        (f"[`{camel}`] is the only thing that makes one; `{fake_test_name}` holds that",
         False, f"a name spelled like a test does not resolve unless it IS one "
                 f"({fake_test_name})", ('"only"', f"`{camel}`")),
        (f"[`{camel}`] is the only thing that makes one; the `{fake_test_name}` test "
         f"holds that", False,
         "a bare word `test` beside the SAME broken name does not excuse it — "
         "this is the exact shape that shipped false", ('"only"', f"`{camel}`")),
        (f"[`{camel}`] is the only thing that enumerates kinds; the "
         f"`{fake_plain_name}` test holds that", False,
         f"a backticked snake_case name that resolves to nothing is a citation "
         f"whatever its OWN spelling, and the bare word `test` beside it does "
         f"not excuse it ({fake_plain_name})", ('"only"', f"`{camel}`")),
        (f"[`{camel}`] is the only thing that makes one; `{helper}` holds that", True,
         f"STAYS GREEN: a `#[cfg(test)]`-module helper resolves even with no "
         f"`#[test]` attribute of its own ({helper})"),
        (f"[`{camel}`] is the only thing that makes one; `{tests_helper}` holds that",
         True, f"STAYS GREEN: a free function in a `crates/*/tests/*.rs` file "
               f"resolves the same way ({tests_helper})"),
        (f"[`{camel}`] is the only thing that makes one; held by {helper}", False,
         f"the broadened enumeration needs the BACKTICK — a bare mention of an "
         f"ordinary helper name is indistinguishable from prose ({helper})",
         ('"only"', f"`{camel}`")),
        (f"[`{camel}`] is the only thing that makes one; the user presses "
         f"`{short_cfg}`", False,
         f"a ONE-WORD `#[cfg(test)]` helper is not a citation, it is the English "
         f"word it collides with ({short_cfg})", ('"only"', f"`{camel}`")),
        (f"[`{camel}`] is the only thing that makes one; see the `{short_tests}` "
         f"above", False,
         f"and a one-word free function in a `tests/` file is not one either "
         f"({short_tests})", ('"only"', f"`{camel}`")),
        (f"[`{camel}`] is the only thing that makes one; `{method}` holds that", False,
         f"a METHOD is not a test, whatever module it sits in ({method})",
         ('"only"', f"`{camel}`")),
        (f"[`{camel}`] is the only thing that makes one; `{shadowed}` holds that",
         False, f"a name PRODUCTION defines is not a test citation, however a test "
                f"file shadows it ({shadowed})", ('"only"', f"`{camel}`")),
        (f"[`{camel}`] is the only thing that makes one; `{ack_symbol}` holds that",
         False, f"a name registered as an external SYMBOL is not a registered "
                f"external TEST, and discharges nothing ({ack_symbol})",
         ('"only"', f"`{camel}`")),
    ]

    # --- the BLOCK fixtures -----------------------------------------------
    # A sentence wraps. Reading a line as if the wrap ended it was rule D's
    # largest measured miss, so these carry the claim ACROSS lines — and the
    # controls are the two shapes where joining would be wrong.
    cases += [
        (["nothing else in this module writes", f"[`{camel}`] after the first frame"],
         False, "a claim wrapped across two lines is one claim",
         ('"nothing"', f"`{camel}`")),
        ([f"[`{camel}`] is rebuilt at boot and reads", "nothing else after that"],
         False, "the finding names the line the QUANTIFIER is on, not the block's first",
         ('"nothing"', f"`{camel}`"), 1),
        (["* nothing else writes here", f"* [`{camel}`] is rebuilt at boot"],
         True, "STAYS GREEN: two list items are two claims, not one wrapped sentence"),
        (["nothing else writes here", "", f"[`{camel}`] is rebuilt at boot"],
         True, "STAYS GREEN: a blank comment line ends the paragraph"),
        ([f"[`{camel}`] is the only thing that makes one.",
          f"The `{named_test}` test holds that."],
         False, f"a citation on the next LINE of the same paragraph is still in "
                f"another sentence, and discharges nothing ({named_test})",
         ('"only"', f"`{camel}`")),
        ([f"[`{camel}`] is the only thing that makes one and",
          f"`{named_test}` holds that"],
         True, f"STAYS GREEN: the citation wraps onto the next line but stays "
               f"inside the claim\'s sentence ({named_test})"),
        ([f"nothing else writes [`{camel}`].", "and that is where it stops"],
         True, "STAYS GREEN: the claim's sentence touches no line this run checks",
         (), 0, (1,)),
        # The pair that pins WHICH line admits a wrapped claim. The control
        # above cannot: its first line ends in a full stop, so the sentence
        # never reaches the line in scope and both readings agree. Here the
        # sentence wraps and the two disagree — reading the whole span reports a
        # claim at line 0 on a run checking only line 1, which is the gate
        # blocking on a quantifier the diff never added.
        (["nothing else in this module writes", f"[`{camel}`] after the first frame"],
         True, "STAYS GREEN: the wrap reaches this line, but the QUANTIFIER is "
               "on one this run does not check",
         (), 0, (1,)),
        (["nothing else in this module writes", f"[`{camel}`] after the first frame"],
         False, "the quantifier's own line is checked, so the claim is reported "
                "even though its symbol sits on a line that is not",
         ('"nothing"', f"`{camel}`"), 0, (0,)),
        # The same pair mirrored, quantifier on the wrap's SECOND line. Admitting
        # on the sentence's first line reads identically to the quantifier's own
        # on the pair above, and inverts on this one.
        ([f"[`{camel}`] is rebuilt at boot and reads", "nothing else after that"],
         False, "the quantifier is checked wherever in the wrap it falls, not "
                "only where the sentence starts",
         ('"nothing"', f"`{camel}`"), 1, (1,)),
        ([f"[`{camel}`] is rebuilt at boot and reads", "nothing else after that"],
         True, "STAYS GREEN: the wrap starts on this line, but the QUANTIFIER is "
               "on one this run does not check",
         (), 0, (0,)),
    ]

    # --- the PATTERN guards -----------------------------------------------
    # A property of a pattern, asserted against text written out here rather
    # than derived from the tree. The fixtures above reach the resolvers only
    # through check(), which reads the real crates, so they cannot put a
    # near-miss in front of one. These are English words and travel to a
    # sibling repo as they stand, like the QUANTIFIER list above.
    guards = [
        (not re.search(DEFN.format(sym="only"), "allows a retype only WITHIN a class"),
         "DEFN does not read `type` inside `retype` as a definition of `only`"),
        (bool(re.search(DEFN.format(sym="only"), "pub type only = u8;")),
         "DEFN still reads a real `type only` as a definition"),
    ] + revision_guards() + resolver_stub_guard() + acknowledged_kind_guard()

    # A finding has to say WHERE, or a reader cannot act on it. Asserted on
    # every case that is meant to be reported, not on a sample of them.
    probe_line = 17

    bad = 0
    for held, label in guards:
        print(f"  {'ok  ' if held else 'MISS'} {label}")
        if not held:
            bad += 1
    for case in cases:
        body, should_pass, label = case[0], case[1], case[2]
        want = case[3] if len(case) > 3 else ()
        # `at` is which fixture line the finding must name; `only` is which
        # fixture lines this run is checking, defaulting to all of them.
        at = case[4] if len(case) > 4 else 0
        only = case[5] if len(case) > 5 else None
        bodies = [body] if isinstance(body, str) else list(body)
        lines = [(probe_line + i, b) for i, b in enumerate(bodies)]
        scope = frozenset(
            probe_line + i
            for i in (range(len(bodies)) if only is None else only)
        )
        where = f"{probe.relative_to(ROOT)}:{probe_line + at} "
        in_scope = {f"{probe.relative_to(ROOT)}:{n} " for n in scope}
        found = check([Unit(probe, lines, scope)])
        ok, note = (not found) == should_pass, ""
        if ok and found:
            # A finding names a line this run is checking — asserted on every
            # reported case, because a rule that reads an unchanged neighbour
            # for context can report at it by accident, and that is the gate
            # blocking on a line the diff never added.
            astray = [m for m in found if not any(m.startswith(p) for p in in_scope)]
            if astray:
                ok, note = False, "  [message names a line outside this run's scope]"
            elif not all(m.startswith(where) for m in found):
                ok, note = False, f"  [message does not open with {where.strip()}]"
            else:
                absent = [w for w in want if not any(w in m for m in found)]
                if absent:
                    ok, note = False, f"  [message omits {', '.join(absent)}]"
        print(f"  {'ok  ' if ok else 'MISS'} {label}{note}")
        if not ok:
            bad += 1
    if bad:
        print(f"\nself-test FAILED: {bad} of {len(cases) + len(guards)} cases wrong")
        return 1
    print(f"\nself-test passed: {len(cases)} derived cases and {len(guards)} written "
          f"guards, detection and control both correct")
    return 0


def main() -> int:
    args = sys.argv[1:]
    if "--self-test" in args:
        return self_test()
    if "--all" in args:
        work = [u for f in sorted(ROOT.glob("crates/**/*.rs")) for u in units(f, WORKTREE)]
    else:
        work = diff_units(ROOT)

    failures = check(work)
    if failures:
        print("Comment claims that do not check out:\n")
        for f in failures:
            print(f"  {f}")
        print(
            "\nA citation names a symbol, path or package that is not where the comment\n"
            "says it is: fix the comment, or — if it is about code outside this repo —\n"
            "register it in ACKNOWLEDGED with the reason. A completeness claim asserts\n"
            "something about all of a thing with no test named: cite the test, or say\n"
            "the narrower thing you can hold."
        )
        return 1
    checked = len({(u.path, n) for u in work for n in u.scope})
    print(f"Comment claims check out ({checked} comment lines checked).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
