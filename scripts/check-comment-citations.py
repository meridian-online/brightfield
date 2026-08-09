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

WHAT IS CHECKED (changed comment lines only, `origin/main...HEAD`)
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
       sentence as a named symbol, where the comment names no test. A, B and C
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

WHAT IS *NOT* CHECKED (scope, stated so nobody reads this as more than it is)
    - Only ADDED comment lines in the diff. Pre-existing debt is not blocking;
      this stops new claims, it does not audit old ones. `--all` audits the tree.
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
    - Rule D reads one comment LINE at a time, like the rest of this script, so
      a claim whose quantifier and symbol land on either side of a wrap is out
      of reach. Widening to the comment block would raise recall and would also
      pair words that are sentences apart; the narrow half is the one AC-checked
      here, and a missed claim costs a reviewer while a reported honest sentence
      costs the gate.
    - Rule D does not read the test it asks for. A comment that names one is
      past it, whatever the test asserts. It buys a reader a place to look and
      an author a moment of doubt, not a proof.
    - Rule D's RECALL is PARTIAL, and was measured that way against the comment
      lines a review wave in this repo made an author delete — the count is in
      the commit that landed this paragraph. Two shapes account for most of the
      residue (the line wrap above is a third), and both are refusals rather
      than oversights:
        * the subject is a bare lowercase word the file does not define as an
          item — a protocol status value quoted in prose, say. There is no
          route from that word to anything a gate can resolve, and treating
          every backticked word as a symbol was measured on this tree: it adds
          sentences about parameters and literal values, which is the shape
          that gets a gate switched off.
        * the quantifier and the subject sit in different SENTENCES of one
          line. Pairing across a sentence boundary is the false positive this
          rule can least afford, so that shape stays a reviewer's.
      A sentence rule D misses costs a reviewer a read. A sentence it reports
      wrongly costs the gate itself.

ESCAPE HATCH
    A claim about code outside this repo (upstream libraries, a sibling repo)
    cannot resolve here and must be registered in ACKNOWLEDGED with a reason,
    not silently skipped. Writing the reason is the point. The registry keys on
    the name as the comment writes it — a symbol for A, a package for C.

USAGE
    scripts/check-comment-citations.py             # gate the diff vs origin/main
    scripts/check-comment-citations.py --all       # audit every comment in tree
    scripts/check-comment-citations.py --self-test # prove the gate can fail
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Symbols and packages named in comments that this repo does not contain. Each
# needs a reason: the point of the registry is that "it's external" gets
# written down rather than assumed.
ACKNOWLEDGED: dict[str, str] = {
    "markPlotSpec": "mosaic/vgplot source; this tree vendors only the YAML spec corpus",
    "channelOption": "mosaic/vgplot source, same reason",
    "queryFieldInfo": "mosaic/vgplot source, same reason",
    "isColorChannel": "mosaic/vgplot source, same reason",
    "literalToSQL": "mosaic sql package, not vendored",
    "egui_code_editor": "evaluated against the spec editor and rejected, so it is "
                        "deliberately absent from Cargo.lock; the comparison is in "
                        "crates/brightfield-shell/src/editor.rs",
}

DEFN = r"(?:const|static|fn|struct|enum|trait|type|union|mod)\s+{sym}\b|macro_rules!\s+{sym}\b"

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
# the other is the false positive this rule can least afford.
SENTENCE_END = re.compile(r"(?<=[.!?;])\s+")

# Does the comment name a test? Deliberately generous — `tests`, `#[test]`,
# `self-test`, `tested`, `foo_test`, `test_foo` all count, and the test is never
# opened. The gate asks for a place to look; a reviewer decides whether what is
# there holds the claim. Generous in this direction only reduces what D reports,
# which is the direction a gate survives being wrong in.
TEST_CITE = re.compile(r"(?:\b|_)test", re.I)


def crates() -> set[str]:
    d = ROOT / "crates"
    return {p.name for p in d.iterdir() if p.is_dir()} if d.is_dir() else set()


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
        file on purpose: workspace-wide, `only`, `all`, `row`, `value` and
        `name` are each some crate's function, and the mark would degenerate
        into "any backticked word".

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


def sentences(body: str) -> list[str]:
    """A comment line's sentences. A line that ends mid-sentence is one of them."""
    return [s for s in SENTENCE_END.split(body) if s.strip()]


def completeness_claims(body: str, file_text: str = "") -> list[tuple[str, str]]:
    """(quantifier, symbol) for each sentence that asserts one over the other.

    `file_text` is the source the comment lives in, for the resolving mark in
    symbol_citations(). Empty when the comment names a test: the claim then has
    a place a reader can go, which is the whole of what this rule asks for.
    """
    if TEST_CITE.search(body):
        return []
    claims: list[tuple[str, str]] = []
    for sentence in sentences(body):
        # The quantifier has to be PROSE. A backticked `all` is a column, a
        # field or a variant this repo happens to have named that, and it
        # asserts nothing — so the backticked spans come out before the search,
        # and go back in for the symbol.
        q = QUANTIFIER.search(TICKED_SPAN.sub(" ", sentence))
        if not q:
            continue
        syms = symbol_citations(sentence, file_text)
        if syms:
            claims.append((q.group(1), syms[0]))
    return claims


def path_exists(ref: str) -> bool:
    """Repo-relative, or crate-relative — comments write both and mean the same.

    `vendor/mosaic-specs/yaml/` is real, at `crates/brightfield-spec/vendor/…`.
    Flagging that as missing is noise; a path that resolves nowhere is not.
    """
    if (ROOT / ref).exists():
        return True
    return any((c / ref).exists() for c in (ROOT / "crates").iterdir() if c.is_dir())


def changed_files() -> list[Path]:
    """Files this branch touched, or a LOUD failure.

    The one thing this gate must never do is pass vacuously. A shallow CI
    checkout has no merge base with origin/main, `git diff` then yields nothing,
    and "citations resolve" would be printed over an unread diff — which is
    precisely how a checker becomes decorative. So an unresolvable base is an
    error, not an empty list.
    """
    subprocess.run(["git", "-C", str(ROOT), "fetch", "-q", "--no-tags", "origin", "main"],
                   capture_output=True)
    base = subprocess.run(
        ["git", "-C", str(ROOT), "merge-base", "origin/main", "HEAD"],
        capture_output=True, text=True,
    )
    if base.returncode != 0 or not (base.stdout or "").strip():
        sys.exit(
            "check-comment-citations: no merge base with origin/main — refusing to\n"
            "report success over a diff it could not read. In CI this means the\n"
            "checkout is too shallow; set `fetch-depth: 0`."
        )
    out = subprocess.run(
        ["git", "-C", str(ROOT), "diff", "--name-only", "--diff-filter=d", "origin/main...HEAD"],
        capture_output=True,
        text=True,
    )
    return [ROOT / f for f in (out.stdout or "").split() if f.endswith(".rs")]


def added_comment_lines(path: Path) -> list[tuple[int, str]]:
    """(line-number, comment-body) for lines this branch ADDED."""
    rel = path.relative_to(ROOT)
    out = subprocess.run(
        ["git", "-C", str(ROOT), "diff", "-U0", "origin/main...HEAD", "--", str(rel)],
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


def all_comment_lines(path: Path) -> list[tuple[int, str]]:
    hits = []
    for i, line in enumerate(path.read_text(errors="replace").splitlines(), 1):
        m = COMMENT.match(line)
        if m:
            hits.append((i, m.group(1)))
    return hits


_FILE_TEXT: dict[Path, str] = {}


def file_text(path: Path) -> str:
    """The source a comment lives in, read once. Absent file reads as empty."""
    if path not in _FILE_TEXT:
        try:
            _FILE_TEXT[path] = path.read_text(errors="replace")
        except OSError:
            _FILE_TEXT[path] = ""
    return _FILE_TEXT[path]


def check(pairs: list[tuple[Path, list[tuple[int, str]]]]) -> list[str]:
    known = crates()
    failures: list[str] = []
    for path, lines in pairs:
        rel = path.relative_to(ROOT)
        source = file_text(path)
        for lineno, body in lines:
            for crate, symbol in (
                [(c, s) for c, s in ATTR_POSSESSIVE.findall(body)]
                + [(c, s) for s, c in ATTR_IN.findall(body)]
            ):
                if crate not in known or symbol in ACKNOWLEDGED:
                    continue
                if not defines(crate, symbol):
                    where = [k for k in known if defines(k, symbol)]
                    hint = f" — it is defined in {', '.join(where)}" if where else " — no crate defines it"
                    failures.append(f"{rel}:{lineno} attributes `{symbol}` to `{crate}`{hint}")
            for ref in PATH_REF.findall(body):
                if not path_exists(ref):
                    failures.append(f"{rel}:{lineno} cites `{ref}`, which does not exist")
            for name in package_citations(body):
                if name in ACKNOWLEDGED or package_resolves(name):
                    continue
                failures.append(
                    f"{rel}:{lineno} cites the `{name}` package, which is not a crate "
                    f"here and not in Cargo.lock"
                )
            for word, symbol in completeness_claims(body, source):
                failures.append(
                    f"{rel}:{lineno} says \"{word}\" of `{symbol}` and names no test — "
                    f"cite the test that holds it, or drop the quantifier"
                )
    return failures


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
    if not sym:
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

    for name, why in list(ACKNOWLEDGED.items())[:1]:
        cases.append((f"`{name}` is called here", True,
                      f"STAYS GREEN: ACKNOWLEDGED as external ({why})"))

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
            for n in ACKNOWLEDGED
            if re.fullmatch(PKG_NAME, n) and not package_resolves(n)
        ),
        None,
    )
    if ack_pkg is None:
        print("self-test: no ACKNOWLEDGED package-shaped name to derive the escape-hatch case from")
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
         f"STAYS GREEN: ACKNOWLEDGED as external ({ACKNOWLEDGED[ack_pkg]})"),
        (f"one `{absent}` sample costs ~2.5 ms", True,
         "STAYS GREEN: a duration is not a version"),
        (f"`{absent}` clamps to [0.0, 1.0]", True,
         "STAYS GREEN: a range is not a version"),
        (f"`{absent}` binds 127.0.0.1:1 to refuse connections", True,
         "STAYS GREEN: an address is not a version"),
    ]

    # --- the COMPLETENESS fixtures ----------------------------------------
    # Rule D judges no claim, so its fixtures are about where it stops. The
    # rejections are shapes a review wave in this repo corrected by hand; the
    # controls are sentences from beside them that were correct.
    #
    # The material below is derived so that each MARK in symbol_citations()
    # gets a token qualifying through that mark ALONE. Deleting a mark then
    # reddens one named case instead of none — which is what a gate whose marks
    # were unpinned looks like from the outside: green, and blind.
    def usable(name: str) -> bool:
        return not TEST_CITE.search(name) and not QUANTIFIER.fullmatch(name)

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
    probe_text = file_text(probe)

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
        (f"[`{camel}`] is the only thing that makes one; the `{test_stem}` tests hold that", True,
         f"STAYS GREEN: a test is named ({test_stem})"),
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
         "STAYS GREEN: a hyphenated quantifier is an adjective, not a claim"),
    ]

    # A finding has to say WHERE, or a reader cannot act on it. Asserted on
    # every case that is meant to be reported, not on a sample of them.
    probe_line = 17
    where = f"{probe.relative_to(ROOT)}:{probe_line} "

    bad = 0
    for case in cases:
        body, should_pass, label = case[0], case[1], case[2]
        want = case[3] if len(case) > 3 else ()
        found = check([(probe, [(probe_line, body)])])
        ok, note = (not found) == should_pass, ""
        if ok and found:
            if not all(m.startswith(where) for m in found):
                ok, note = False, f"  [message does not open with {where.strip()}]"
            else:
                absent = [w for w in want if not any(w in m for m in found)]
                if absent:
                    ok, note = False, f"  [message omits {', '.join(absent)}]"
        print(f"  {'ok  ' if ok else 'MISS'} {label}{note}")
        if not ok:
            bad += 1
    if bad:
        print(f"\nself-test FAILED: {bad} of {len(cases)} cases wrong")
        return 1
    print(f"\nself-test passed: {len(cases)} derived cases, detection and control both correct")
    return 0


def main() -> int:
    args = sys.argv[1:]
    if "--self-test" in args:
        return self_test()
    if "--all" in args:
        files = sorted(ROOT.glob("crates/**/*.rs"))
        pairs = [(f, all_comment_lines(f)) for f in files]
    else:
        pairs = [(f, added_comment_lines(f)) for f in changed_files() if f.exists()]

    failures = check(pairs)
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
    checked = sum(len(v) for _, v in pairs)
    print(f"Comment claims check out ({checked} comment lines checked).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
