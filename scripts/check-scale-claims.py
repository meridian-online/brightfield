#!/usr/bin/env python3
"""A promise about performance at scale has to say which marks it holds for.

WHY THIS EXISTS
    Interaction cost stops tracking row count when a cube can be built, and a
    cube can be built for a mark that aggregates. A row-level mark — a raw
    scatter, one drawn mark per row — has nothing to pre-compute, so its cost
    tracks the rows. The difference between the two is measured in
    benchmarks/results/ and set out in docs/interaction-speed.md.

    Read on 2026-08-26, sentences in README.md, the coordinator rustdoc and
    docs/interaction-speed.md promised the property without naming that split,
    and the strongest of them was the repo's opening line: "GPU-native desktop
    application for interactive data visualisation at any scale". Somebody
    evaluating this reads the first paragraph and stops there if it does not
    match their case; if what they need is a scatter over their own ten million
    rows, the README told them yes and they found out by downloading it. Every
    one of those sentences is a must-fail case below, in the words it shipped
    in; MUST_FAIL is the enumeration, so there is no count of them here to rot.

    A sweep for row-count vocabulary missed the ones written in scale
    vocabulary, which make the same promise in different words, and a third
    dialect — memory vocabulary, "without loading ten million rows into memory"
    — was found later still. That is the reason this is a committed gate and
    not a one-off grep: the claim has more than one dialect, and the next one
    will be written by somebody who has not read this file.

WHAT IS CHECKED
    Every tracked `.md` file, and every `///` / `//!` doc comment in a tracked
    `.rs` file. A sentence that makes one of the promises in PROMISES must
    carry one of the qualifications in QUALIFIED — in that same sentence, not
    in a footnote further down, because the reader this exists for stops
    reading at the end of the paragraph.

    Sentence, not line. Prose wraps, so lines are joined into a block first,
    and a blank line, a bullet, a numbered item, a table row, a heading or a
    block quote breaks the join — so one bullet's qualification cannot excuse
    the next bullet's promise. Each of those breaks has a must-fail case of its
    own whose promise carries no sentence-ending punctuation, because a case
    written for a break but ending in a full stop is caught by the sentence
    split instead and says nothing about the break.

    That enumeration is itself exercised. `--self-test` has two halves: the
    sentence cases call `scan_text`, and the tree cases stage a real git
    checkout and call `check`, so `tracked()` is run rather than assumed. A
    `tracked()` that has stopped listing files reports what a clean tree
    reports, and until 2026-08-26 this self-test stayed green over exactly
    that.

    Every must-fail case also names the finding it expects, so a case cannot go
    on passing because some other mechanism caught its mutation. Without that
    field the bullet and paragraph cases below were both being caught by the
    sentence split, and disabling the breaks they were written for left this
    self-test green.

WHAT IS *NOT* CHECKED (stated so nobody reads this as more than it is)
    - It cannot judge a claim, and it does not try. It asks whether the
      sentence names the distinction, which is a question about words. A
      sentence that names an aggregating mark and then says something false
      about it passes here and needs a reviewer.
    - PROMISES is an ENUMERATION and a new phrasing escapes it. That is not a
      hypothetical: this repo has already written the same promise three ways.
      What the enumeration buys is that the phrasings which HAVE shipped cannot
      come back unqualified — every one of them is a case in --self-test, in the
      exact words it shipped in.
    - Plain `//` comments are out of scope. A promise made to a reader lives in
      a doc comment or in markdown; an implementation note beside a line of code
      is a different audience, and reading those in reddened on ordinary prose
      ("this rectangle cannot fit at any scale this spec would render at").
    - It says nothing about commit messages, PR text or the website.
    - The tree cases prove the enumeration reaches a tracked `.md`, a tracked
      `.rs`, and neither an untracked file nor a vendored one. They say nothing
      about how git behaves in a checkout with submodules, sparse checkout or a
      pathspec-altering config; the gate would report differently there and no
      case here would notice.

Usage (no arguments, from anywhere inside the repo):

    ./scripts/check-scale-claims.py
    ./scripts/check-scale-claims.py --self-test

Exit codes:
    0  clean
    1  one or more unqualified promises found
    2  the gate could not run (not a git checkout)
"""

from __future__ import annotations

import re
import subprocess
import sys
import tempfile
from pathlib import Path

# The promise, in every dialect this repo has written it in. Each entry is
# (label, pattern); the label is what a finding prints, so it names the shape
# rather than the regex.
PROMISES: list[tuple[str, str]] = [
    ("scale vocabulary", r"at any scale\b"),
    ("scale vocabulary", r"at any (data |table )?size\b"),
    ("scale vocabulary", r"(thousands|millions|hundreds) to billions of (records|rows)"),
    ("scale vocabulary", r"scales? to (any|billions)\b"),
    ("row-count vocabulary", r"at any row count"),
    ("row-count vocabulary", r"independent of (the )?row count"),
    ("row-count vocabulary", r"(regardless of|whatever) (the )?(row count|number of rows|table size)"),
    ("row-count vocabulary", r"no matter how many rows"),
    ("row-count vocabulary", r"stops? tracking row count"),
    ("row-count vocabulary", r"as (the table|the data|it) grows\b"),
    ("memory vocabulary", r"without loading .{0,40}(rows|records|the table) into memory"),
    ("memory vocabulary", r"never loads? the (table|data|rows) into memory"),
]

# What makes the promise honest: the sentence names the split, or points at the
# document that draws it. Naming a row-level mark counts, and so does naming the
# mechanism the property comes from — both tell a reader which side they are on.
QUALIFIED: list[str] = [
    r"aggregat",       # aggregating mark, pre-aggregation, aggregates
    r"cube",
    r"row-level",
    r"scatter",
    r"(row per|per row)\b",
    r"row[- ]per[- ]mark",
    r"rows? per mark",
    r"summar",         # summarises, a small summary table
    r"interaction-speed",
]

PROMISE_RE = [(label, re.compile(pat, re.IGNORECASE)) for label, pat in PROMISES]
QUALIFIED_RE = re.compile("|".join(QUALIFIED), re.IGNORECASE)

# A line that starts a new block rather than continuing the one above: a list
# item, a table row, a heading, a quote. Joining across one of these would let a
# neighbour's qualification excuse this line's promise.
BREAKS = re.compile(r"^\s*(\|.*\||[-*+]\s|\d+[.)]\s|#{1,6}\s|>\s)")
FENCE = re.compile(r"^\s*(```|~~~)")
DOC_COMMENT = re.compile(r"^\s*//[/!](.*)$")
SENTENCE_SPLIT = re.compile(r"(?<=[.!?])\s+")


class Finding:
    def __init__(self, path: str, line: int, label: str, sentence: str):
        self.path, self.line, self.label, self.sentence = path, line, label, sentence

    def __str__(self) -> str:
        text = self.sentence if len(self.sentence) <= 160 else self.sentence[:157] + "..."
        return f"{self.path}:{self.line}: [{self.label}] {text}"


def tracked(root: Path) -> list[str]:
    """The tracked files this gate reads: markdown, and Rust for its doc comments.

    This file spells every phrase it looks for and is not excluded by name; the
    pathspec is `*.md` and `*.rs`, so a `.py` is never listed. Vendored Mosaic
    specs are upstream text nobody here writes, and a promise in one is not this
    repo's to qualify.
    """
    out = subprocess.run(
        ["git", "-C", str(root), "ls-files", "-z", "*.md", "*.rs"],
        capture_output=True,
        text=True,
        check=True,
    )
    return [
        p
        for p in out.stdout.split("\0")
        if p and "crates/brightfield-spec/vendor/" not in p
    ]


def blocks(path: str, text: str) -> list[tuple[int, str]]:
    """(line number of the block's first line, the block joined into one string).

    A block is what a sentence may span. For markdown that is a paragraph; for
    Rust it is a run of contiguous doc-comment lines. Both break at blank lines,
    list markers, table rows and headings.
    """
    rust = path.endswith(".rs")
    out: list[tuple[int, str]] = []
    start: int | None = None
    buf: list[str] = []
    fenced = False

    def flush() -> None:
        nonlocal start, buf
        if start is not None and buf:
            out.append((start, " ".join(buf)))
        start, buf = None, []

    for number, raw in enumerate(text.splitlines(), start=1):
        if rust:
            match = DOC_COMMENT.match(raw)
            if not match:
                flush()
                continue
            line = match.group(1)
        else:
            if FENCE.match(raw):
                fenced = not fenced
                flush()
                continue
            if fenced:
                continue
            line = raw
        if not line.strip():
            flush()
            continue
        if BREAKS.match(line):
            flush()
            start, buf = number, [line.strip()]
            continue
        if start is None:
            start = number
        buf.append(line.strip())
    flush()
    return out


def scan_text(path: str, text: str) -> list[Finding]:
    findings: list[Finding] = []
    for line, block in blocks(path, text):
        for sentence in SENTENCE_SPLIT.split(block):
            for label, pattern in PROMISE_RE:
                if pattern.search(sentence) and not QUALIFIED_RE.search(sentence):
                    findings.append(Finding(path, line, label, sentence.strip()))
                    break
    return findings


def check(root: Path) -> list[Finding]:
    findings: list[Finding] = []
    for path in tracked(root):
        try:
            text = (root / path).read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        findings.extend(scan_text(path, text))
    return findings


# --------------------------------------------------------------------------
# Self-test, part one: the sentence cases, against `scan_text`.
#
# Every must_fail case is a sentence that HAS shipped in this repo, in the words
# it shipped in; every must_pass case is prose from this tree that a careless
# pattern would redden. The gate runs beside this in CI, because a checker that
# has quietly stopped matching reports what a clean tree reports.
#
# These cases reach the matching and nothing else. Part two is the other half.
# --------------------------------------------------------------------------

# (path, text, the finding this case expects). The third field is why a case
# cannot go on passing because some OTHER mechanism caught its mutation: the
# bullet and paragraph cases below both end in a full stop, so the sentence
# split reddens them even with the break they were written for disabled. The
# cases that isolate a break carry no sentence-ending punctuation at all.
MUST_FAIL: list[tuple[str, str, str]] = [
    (
        "README.md",
        "GPU-native desktop application for interactive data visualisation at any scale. "
        "Brightfield combines Mosaic's declarative specification grammar and coordinator "
        "architecture with a Vello GPU 2D scene renderer.",
        "README.md:1: [scale vocabulary] GPU-native desktop application for "
        "interactive data visualisation at any scale.",
    ),
    (
        "README.md",
        "The goal is a tool that can interactively visualise and explore datasets from "
        "thousands to billions of records with fluid, GPU-rendered interactions, without "
        "the performance ceiling of browser-based rendering.",
        "README.md:1: [scale vocabulary] The goal is a tool that can interactively "
        "visualise and explore datasets from thousands to billions of records",
    ),
    (
        "README.md",
        "- A Mosaic YAML spec defining a two-view cross-filtered dashboard over a large "
        "Parquet file stays fluid as the table grows: interaction latency roughly "
        "independent of row count.",
        "README.md:1: [row-count vocabulary] - A Mosaic YAML spec defining a "
        "two-view cross-filtered dashboard",
    ),
    (
        "crates/brightfield-engine/src/coordinator.rs",
        "//! resolves to a predicate the engine wraps into a SQL `WHERE`, and the\n"
        "//! affected marks re-execute. That is what makes interaction latency roughly\n"
        "//! independent of row count.\n",
        "crates/brightfield-engine/src/coordinator.rs:1: [row-count vocabulary] That "
        "is what makes interaction latency roughly independent of row count.",
    ),
    (
        "docs/interaction-speed.md",
        "Brightfield pushes interaction down into the database: dragging a brush becomes a\n"
        "predicate and a re-query. That is what lets it work at ten million rows without\n"
        "loading ten million rows into memory.\n",
        "docs/interaction-speed.md:1: [memory vocabulary] That is what lets it work "
        "at ten million rows without loading ten million rows into memory.",
    ),
    # The wrap is the point: the promise and its would-be qualification on
    # different lines of one sentence must still be read as one sentence. Stop
    # joining wrapped lines and the block is "Interaction latency is
    # independent", which matches no promise and reports nothing.
    (
        "docs/wrapped.md",
        "Interaction latency is independent\nof row count.\n",
        "docs/wrapped.md:1: [row-count vocabulary] Interaction latency is "
        "independent of row count.",
    ),
    # The next two are shapes that have shipped. Both end in a full stop, so the
    # sentence split catches them whichever way the block was cut; they pin the
    # shape, and the isolating cases below pin the break.
    (
        "docs/bullets.md",
        "- Interaction latency is independent of row count.\n"
        "- A raw scatter is a row-level mark and aggregates nothing.\n",
        "docs/bullets.md:1: [row-count vocabulary] - Interaction latency is "
        "independent of row count.",
    ),
    (
        "docs/paragraphs.md",
        "Interaction latency is independent of row count.\n"
        "\n"
        "A raw scatter is a row-level mark, and aggregating marks are the ones that "
        "get a cube.\n",
        "docs/paragraphs.md:1: [row-count vocabulary] Interaction latency is "
        "independent of row count.",
    ),
    # ----------------------------------------------------------------------
    # One case per break, each naming the break it is the only cover for. The
    # promise carries no full stop, so the sentence split cannot separate it
    # from the qualification beside it: disable the named break and the two
    # join into one sentence that reads as qualified, and the case goes silent.
    # ----------------------------------------------------------------------
    # BREAKS, bullet alternative `[-*+]\s`.
    (
        "docs/bullet-break.md",
        "- Interaction latency is independent of row count\n"
        "- for an aggregating mark, which is served from a cube\n",
        "docs/bullet-break.md:1: [row-count vocabulary] - Interaction latency is "
        "independent of row count",
    ),
    # BREAKS, numbered alternative `\d+[.)]\s`. Written `1)` rather than `1.`
    # so the marker itself carries no full stop for the sentence split to use.
    (
        "docs/numbered-break.md",
        "1) Interaction latency is independent of row count\n"
        "2) for an aggregating mark, which is served from a cube\n",
        "docs/numbered-break.md:1: [row-count vocabulary] 1) Interaction latency is "
        "independent of row count",
    ),
    # BREAKS, table-row alternative `\|.*\|`.
    (
        "docs/table-break.md",
        "| claim | status |\n"
        "|---|---|\n"
        "| Interaction latency is independent of row count | shipped |\n"
        "| an aggregating mark is served from a cube | shipped |\n",
        "docs/table-break.md:3: [row-count vocabulary] | Interaction latency is "
        "independent of row count | shipped |",
    ),
    # BREAKS, heading alternative `#{1,6}\s`.
    (
        "docs/heading-break.md",
        "## Interaction latency is independent of row count\n"
        "### When an aggregating mark is served from a cube\n",
        "docs/heading-break.md:1: [row-count vocabulary] ## Interaction latency is "
        "independent of row count",
    ),
    # BREAKS, block-quote alternative `>\s`.
    (
        "docs/quote-break.md",
        "> Interaction latency is independent of row count\n"
        "> for an aggregating mark, which is served from a cube\n",
        "docs/quote-break.md:1: [row-count vocabulary] > Interaction latency is "
        "independent of row count",
    ),
    # The blank-line flush in `blocks`, which is not part of BREAKS.
    (
        "docs/blank-line-break.md",
        "Interaction latency is independent of row count\n"
        "\n"
        "An aggregating mark is served from a cube\n",
        "docs/blank-line-break.md:1: [row-count vocabulary] Interaction latency is "
        "independent of row count",
    ),
    # SENTENCE_SPLIT, the other half: one block, two sentences, and the
    # qualification belongs to the second. Read the block whole and it reads as
    # qualified. Nothing here is a break, so only the split can catch it.
    (
        "docs/sentence-split.md",
        "Interaction latency is independent of row count. An aggregating mark is "
        "served from a cube.\n",
        "docs/sentence-split.md:1: [row-count vocabulary] Interaction latency is "
        "independent of row count.",
    ),
]

MUST_PASS = [
    # The sites this branch rewrote, in the words it rewrote them to.
    (
        "README.md",
        "GPU-native desktop application for interactive data visualisation over large "
        "tables. How large depends on the mark: an aggregating mark is served from a "
        "small pre-aggregated summary, while a row-level mark such as a raw scatter "
        "draws one mark per row and its cost tracks the rows.",
    ),
    (
        "README.md",
        "The goal is a tool that can interactively visualise and explore datasets from "
        "thousands to billions of records with fluid, GPU-rendered interactions, reached "
        "today for aggregating marks and not for row-level ones.",
    ),
    (
        "crates/brightfield-engine/src/coordinator.rs",
        "//! Whether that leaves interaction latency independent of row count depends on\n"
        "//! the mark: an aggregating mark can be served from a pre-aggregated summary.\n",
    ),
    # Honest prose from this tree that a looser pattern would redden.
    (
        "benchmarks/README.md",
        "- **brush-binned-density** — the same shape over a brushed column with exactly\n"
        "  forty distinct values: the derived cube stays O(bins x 40) at any row count.\n",
    ),
    (
        "crates/brightfield-render/src/scale.rs",
        "/// A stable sort keyed on first appearance, so `b` precedes `c2` regardless of\n"
        "/// data order.\n",
    ),
    # A plain `//` comment is out of scope, and this one is about geometry.
    (
        "crates/brightfield-shell/tests/protocol_frame_crop.rs",
        "    // The protocol window over this fixture is nowhere near 9000 logical\n"
        "    // points on either axis, so this rectangle cannot fit at any scale this\n"
        "    // spec would plausibly render at.\n",
    ),
    # A fenced code block is not prose.
    (
        "docs/fenced.md",
        "Example output:\n\n```\ninteraction latency independent of row count\n```\n",
    ),
]


# --------------------------------------------------------------------------
# Self-test, part two: the file enumeration, end to end.
#
# The cases above call `scan_text` with text handed to them, which leaves
# `tracked()` and `check()` — the path the gate actually runs — unexercised.
# Make `tracked()` return nothing and part one stays green while the gate goes
# silent over a tree carrying every sentence it exists to stop. So these cases
# stage a real git checkout, `git add` some of it, and call `check`.
# --------------------------------------------------------------------------

SHIPPED_MD = (
    "# brightfield\n\n"
    "GPU-native desktop application for interactive data visualisation at any\n"
    "scale.\n"
)
SHIPPED_RS = (
    "//! affected marks re-execute. That is what makes interaction latency\n"
    "//! roughly independent of row count.\n"
    "\npub struct Coordinator;\n"
)
# The same opening line as SHIPPED_MD with the split named in it.
QUALIFIED_MD = (
    "# brightfield\n\n"
    "GPU-native desktop application for interactive data visualisation at any\n"
    "scale an aggregating mark can be served from a pre-aggregated summary at.\n"
)

TREE_CASES: list[tuple[str, dict[str, str], list[str] | None, list[str]]] = [
    # (name, files to write, files to `git add` (None = all), paths expected in findings)
    (
        "a tracked markdown file is reached",
        {"README.md": SHIPPED_MD},
        None,
        ["README.md"],
    ),
    (
        "a tracked Rust doc comment is reached",
        {"crates/brightfield-engine/src/coordinator.rs": SHIPPED_RS},
        None,
        ["crates/brightfield-engine/src/coordinator.rs"],
    ),
    (
        "both kinds are reached in one pass",
        {
            "README.md": SHIPPED_MD,
            "crates/brightfield-engine/src/coordinator.rs": SHIPPED_RS,
        },
        None,
        ["README.md", "crates/brightfield-engine/src/coordinator.rs"],
    ),
    (
        "an untracked file is out of scope",
        {"README.md": SHIPPED_MD},
        [],
        [],
    ),
    (
        "a vendored Mosaic spec is out of scope",
        {"crates/brightfield-spec/vendor/mosaic-specs/README.md": SHIPPED_MD},
        None,
        [],
    ),
    (
        "a tree that names the split is clean",
        {"README.md": QUALIFIED_MD},
        None,
        [],
    ),
]


def _stage_repo(tmp: Path, files: dict[str, str], add: list[str] | None) -> Path:
    root = tmp / "repo"
    root.mkdir(parents=True, exist_ok=True)
    subprocess.run(["git", "init", "-q", str(root)], capture_output=True, check=True)
    for rel, body in files.items():
        path = root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(body, encoding="utf-8")
    staged = sorted(files) if add is None else sorted(add)
    if staged:
        # --force: a global core.excludesFile on the host must not decide what
        # this case stages, and an ignored path would otherwise abort it.
        subprocess.run(
            ["git", "-C", str(root), "add", "--force", "--", *staged],
            capture_output=True,
            check=True,
        )
    return root


def tree_self_test() -> int:
    failures = 0
    for name, files, add, expected in TREE_CASES:
        with tempfile.TemporaryDirectory() as tmp:
            root = _stage_repo(Path(tmp), files, add)
            got = sorted({finding.path for finding in check(root)})
        if got != sorted(expected):
            failures += 1
            print(
                f"SELF-TEST FAILED: {name}: expected findings in "
                f"{sorted(expected)}, got {got}",
                file=sys.stderr,
            )
    return failures


def self_test() -> int:
    failures = 0
    for path, text, expect in MUST_FAIL:
        found = scan_text(path, text)
        if not found:
            print(
                f"SELF-TEST FAILED: stayed silent on an unqualified promise in {path}:\n"
                f"  {text.strip()[:200]}",
                file=sys.stderr,
            )
            failures += 1
            continue
        detail = "; ".join(str(finding) for finding in found)
        if expect not in detail:
            failures += 1
            print(
                f"SELF-TEST FAILED: caught {path}, but not for the reason the case "
                f"was written for.\n  expected a finding containing: {expect}\n"
                f"  got: {detail}",
                file=sys.stderr,
            )
    for path, text in MUST_PASS:
        found = scan_text(path, text)
        if found:
            print(
                f"SELF-TEST FAILED: cried wolf on honest prose in {path}:\n"
                f"  {found[0]}",
                file=sys.stderr,
            )
            failures += 1
    failures += tree_self_test()
    if failures:
        return 1
    print(
        "scale-claims gate self-test: ok "
        f"({len(MUST_FAIL)} must-fail, {len(MUST_PASS)} must-pass, "
        f"{len(TREE_CASES)} over a staged checkout)"
    )
    return 0


def main(argv: list[str]) -> int:
    if "--self-test" in argv:
        return self_test()
    try:
        root = Path(
            subprocess.run(
                ["git", "rev-parse", "--show-toplevel"],
                capture_output=True,
                text=True,
                check=True,
            ).stdout.strip()
        )
    except subprocess.CalledProcessError:
        print("SCALE-CLAIMS GATE COULD NOT RUN: not a git checkout", file=sys.stderr)
        return 2
    findings = check(root)
    if findings:
        print(
            "A promise about performance at scale is made without saying which marks\n"
            "it holds for. Interaction cost stops tracking row count for a mark that\n"
            "aggregates; a row-level mark such as a raw scatter has no cube to build\n"
            "and its cost tracks the rows.\n",
            file=sys.stderr,
        )
        for finding in findings:
            print(f"  {finding}", file=sys.stderr)
        print(
            "\nFix: name the split in the SAME sentence — an aggregating mark, a\n"
            "row-level mark, the cube, or a pointer to docs/interaction-speed.md —\n"
            "or make a promise the measurement supports.",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
