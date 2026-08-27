#!/usr/bin/env python3
"""Pin the latency figures in docs/interaction-speed.md to the record they came from.

WHY THIS EXISTS
    That document is what the README and the coordinator rustdoc send a reader
    to for the size of the gap between an aggregating mark and a row-level one.
    It carried a table of millisecond figures and nothing kept them in step with
    `benchmarks/results/`.

    Read on 2026-08-26 against `2026-07-27-apple-m1-pro.json`, the run it was
    quoting, that table disagreed with it: a cubed cell printing a 95th
    percentile no run measured, direct cells with no 95th percentile at all, and
    a raw-scatter cell holding that scenario's CUBED figure in the `without`
    column. It was also a whole run behind — quoting 2026-07-27 after 2026-08-07
    had shipped. Both of those documents are must-fail cases in `--self-test`, in
    the digits they shipped with, the second read against that very record.

    `scripts/check-borrowed-benchmarks.sh` already stops UPSTREAM figures being
    restated as ours. This is the other half: our own figures, restated in prose
    and free to rot. The repo has the same rule for constants in Rust sources
    (`crates/brightfield-render/tests/one_ceiling.rs` fails when a measured
    ceiling is copied a second time); this applies it to a copy made in
    markdown.

WHAT IS CHECKED
    Each item below has a case in `--self-test`, and every must-fail case there
    names the finding it expects — prefixed `COULD NOT RUN:` where it expects the
    exit 2 refusal rather than an exit 1 disagreement. A must-fail case that
    names no expected finding is itself a self-test failure. So a case cannot go
    on passing because some OTHER check caught its mutation, which is how the .md
    link cases at item 2 were found to be covering for each other.

    `--self-test` also runs this script as a process over a copy of this
    checkout and reads its exit code, once per code listed at the foot of this
    docstring. Every other case calls `check()`, so nothing else reaches `main`
    — and `main` is what turns findings into exit 1 and is what the CI step runs.

    1. The `## Measured` section names its source record as a repo-relative path
       to a `.json` under benchmarks/results/, exactly one such path, and that
       file exists. A renamed, deleted or duplicated citation fails here, before
       any digit is compared.
    2. Every `benchmarks/results/<name>.md` and `benchmarks/results/<name>.json`
       path written anywhere in the document — link target, link label, reference
       definition or plain prose — names a file that exists and shares its stem
       with that record. The prose summary and the JSON are two views of one run,
       and citing different runs from one page is the same drift.

       The scan reads the path text and not the markdown around it, because the
       link syntaxes GitHub renders identically are not one shape: an anchor on
       the end of the path, a `(path "title")`, a reference definition, a
       repo-absolute path, an angle-bracket target and a raw `<a href>` each have
       a case in `--self-test`. A path this gate cannot resolve to a file is a
       finding, not a silent skip.
    3. The section's opening sentence, `<N> rows, on an <cpu>,`, is READ OUT OF
       THE DOCUMENT and both halves compared to the record: the row count
       against `config.rows`, the machine against `machine.cpu`. Digits or
       number words ("Ten million", "10,000,000") both parse. This is the drift
       direction that matters for a public page — the page rewritten, the record
       left alone — so it is checked against the page, not against a constant in
       this file.
    4. Every latency cell in the table equals the corresponding figure in that
       record, to the one decimal place the table prints. Rounding is half-up on
       the raw millisecond value, so 5.065 reads 5.1.
    5. The cube columns are checked against the record's `preagg` counters, not
       taken on trust:
         - `*no cube possible*` in the `with a cube` column requires
           `cubes_built == 0` for that scenario. A cell that swaps a measured
           figure for that marker cannot quietly un-pin itself.
         - `*no cube possible*` in the `without` column is rejected outright:
           that column is measured with pre-aggregation disabled for every row,
           so the marker asserts nothing there and only hides a figure.
         - A figure in the `with a cube` column requires `cube_hits > 0` for
           that scenario, or the column header is describing a run that served
           nothing from a cube.
    6. The table carries exactly the (chart, gesture) rows PUBLISHED_ROWS lists:
       none missing, none added, none repeated. Missing is the direction that
       matters, and `raw scatter, two views | zoom` is the row it matters most
       for — that row is the row-level half of the comparison, and a page that
       loses it claims only the fast half.

       PUBLISHED_ROWS is not the cross product of SCENARIO and FIELD. Those two
       translate a row's words into the record's names; the record carries
       scenarios (`slider-drag`) and timed fields (`live_apply`) this page does
       not publish, and the cross product names a `raw scatter, two views |
       brush` row the page has never carried.

WHAT IS *NOT* CHECKED (stated so nobody reads this as more than it is)
    - It does not check that the named record is the NEWEST one. Figures stay
      true of the run they cite, and a record captured on another machine is
      not automatically the one this document should quote. Choosing which run
      to publish is a judgement; keeping the digits equal to it is not, which
      is why only the second half is a gate.
    - Of the prose it checks one sentence, the `<N> rows, on an <cpu>` opener at
      item 3. Every other sentence is unchecked: "the cube is usually thousands
      of rows" is a claim about typical data, and no gate can judge it.
    - It reads only this one document. A latency figure written into any other
      file is out of scope and unpinned — including README.md's `Query
      optimisation` bullet, which quotes a millisecond range of its own.
    - It does not decide WHICH rows the page should publish. PUBLISHED_ROWS is a
      judgement written down, and item 6 only stops the document and that list
      moving apart. A row taken out of both is taken out of both.
    - The stem comparison at item 2 asks whether two paths name the same run. It
      does not open the summary and compare what is inside it to the JSON.

Usage (no arguments, from anywhere inside the repo):

    ./scripts/check-measured-figures.py
    ./scripts/check-measured-figures.py --self-test

Exit codes:
    0  clean
    1  one or more figures disagree with the record
    2  the gate could not run (document or record unreadable, table missing,
       the opening sentence missing or unparseable)
"""

from __future__ import annotations

import collections
import decimal
import json
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

DOC = Path("docs/interaction-speed.md")

# Which record field each table row reads. The document's own columns are
# `chart | gesture | with a cube | without`; a row names a benchmark scenario
# and a gesture names the timed field the harness records for it.
#
#   with a cube -> the `engine` half   (the shipped configuration)
#   without     -> the `engine_direct` half (identical code, layer disabled)
#
# `zoom` is the settled navigation extent the harness applies once and
# re-queries under; `brush` is one drag step through the coordinator.
SCENARIO = {
    "binned density": "brush-binned-density",
    "density": "brush-density",
    "raw scatter, two views": "crossfilter-dots",
}
FIELD = {
    "zoom": "navigation_apply",
    "brush": "coordinator_apply",
}
COLUMN = {"engine": "with a cube", "engine_direct": "without"}

# The rows the document publishes, as the (chart, gesture) pairs its first two
# columns spell. Checked in both directions: a row here and not in the table is
# a finding, and so is a row in the table and not here. The direction that made
# this necessary is the first one — the `raw scatter, two views` row is the only
# row-level measurement on the page, and deleting it used to leave every gate at
# exit 0 over a document claiming only the fast half.
#
# Which rows to publish is a judgement, so this is a list and not a derivation:
# the record measures scenarios and fields the page deliberately leaves out.
PUBLISHED_ROWS: list[tuple[str, str]] = [
    ("binned density", "zoom"),
    ("density", "zoom"),
    ("density", "brush"),
    ("binned density", "brush"),
    ("raw scatter, two views", "zoom"),
]

# `**5.1 ms** (5.8)` or `82.0 ms (91.2)` — a median with its 95th percentile.
CELL = re.compile(r"\*{0,2}(\d+\.\d)\s*ms\*{0,2}\s*\((\d+\.\d)\)")
# `*no cube possible*` — a row-level mark has no cube, so there is no cubed
# figure to compare and the cell asserts that rather than a number. What it
# asserts is checked against the record's preagg counters; see `cube_claim`.
NO_CUBE = re.compile(r"\*no cube possible\*")
# The source record, cited as a repo-relative path inside a markdown link.
SOURCE = re.compile(r"\((\.\./benchmarks/results/[A-Za-z0-9._-]+\.json)\)")
# Any benchmarks/results file the document names, in prose or as a link target.
# Matching starts at the path itself rather than at a link delimiter, so the
# markdown around it does not decide whether the citation is read: an anchor, a
# link title, a reference definition, a repo-absolute path, an angle-bracket
# target and a raw `<a href>` all put this same substring on the page. The
# capture is resolved under benchmarks/results/ from the repo root, so the
# leading `../` a link needs from docs/ is not part of what is read.
RESULTS_REF = re.compile(r"benchmarks/results/([A-Za-z0-9._/-]+\.(?:md|json))")
# `Ten million rows, on an Apple M1 Pro,` — the sentence that says what the
# table below is a measurement OF.
HEADLINE = re.compile(
    r"(?P<rows>[A-Za-z0-9][A-Za-z0-9,\- ]*?)\s+rows,\s+on\s+an?\s+(?P<cpu>[^,.]+?)\s*,"
)

NUMBER_WORD = {
    "one": 1, "two": 2, "three": 3, "four": 4, "five": 5, "six": 6, "seven": 7,
    "eight": 8, "nine": 9, "ten": 10, "eleven": 11, "twelve": 12, "fifteen": 15,
    "twenty": 20, "thirty": 30, "forty": 40, "fifty": 50, "sixty": 60,
    "seventy": 70, "eighty": 80, "ninety": 90,
}
SCALE_WORD = {"hundred": 100, "thousand": 1_000, "million": 1_000_000, "billion": 1_000_000_000}


class Fail(Exception):
    """A finding. The message is what the developer reads."""


def repo_root() -> Path:
    out = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"],
        capture_output=True,
        text=True,
        check=True,
    )
    return Path(out.stdout.strip())


def one_dp(ms: float) -> str:
    """The millisecond figure as the table prints it: one decimal, half-up."""
    return str(
        decimal.Decimal(str(ms)).quantize(
            decimal.Decimal("0.1"), rounding=decimal.ROUND_HALF_UP
        )
    )


def parse_rows(phrase: str) -> int | None:
    """`Ten million` and `10,000,000` both read as 10000000. None if neither."""
    compact = phrase.replace(",", "").replace(" ", "")
    if compact.isdigit():
        return int(compact)
    total = current = 0
    seen = False
    for token in re.split(r"[\s\-]+", phrase.strip().lower()):
        if not token:
            continue
        seen = True
        if token.isdigit():
            current += int(token)
        elif token in NUMBER_WORD:
            current += NUMBER_WORD[token]
        elif token in SCALE_WORD:
            current = (current or 1) * SCALE_WORD[token]
            if SCALE_WORD[token] >= 1_000:
                total, current = total + current, 0
        else:
            return None
    return total + current if seen else None


def measured_section(text: str) -> str:
    section = text.split("## Measured", 1)
    if len(section) != 2:
        raise Fail(f"{DOC} has no `## Measured` section to check")
    return section[1].split("\n## ", 1)[0]


def measured_table(section: str) -> list[list[str]]:
    """The rows of the `## Measured` table, each split into its cells."""
    rows = []
    for line in section.splitlines():
        line = line.strip()
        if not line.startswith("|"):
            continue
        cells = [c.strip() for c in line.strip("|").split("|")]
        if not cells or set("".join(cells)) <= set("-: "):
            continue  # the header underline
        if cells[0] == "chart":
            continue  # the header itself
        rows.append(cells)
    if not rows:
        raise Fail(f"{DOC}'s `## Measured` section has no table rows")
    return rows


def headline(section: str) -> tuple[int, str]:
    """(row count, machine) as the DOCUMENT states them, not as this file assumes."""
    match = HEADLINE.search(section)
    if not match:
        raise Fail(
            f"{DOC}'s `## Measured` section has no sentence of the form "
            "'<N> rows, on an <cpu>,'. That sentence says what the table is a "
            "measurement of, and this gate compares both halves of it to the "
            "record; without it the row count and the machine go unchecked."
        )
    rows = parse_rows(match.group("rows"))
    if rows is None:
        raise Fail(
            f"{DOC} says {match.group('rows')!r} rows, which this gate cannot read "
            "as a number. Write it in digits (10,000,000) or in plain number words "
            "(Ten million), so it can be compared to the record's config.rows."
        )
    return rows, match.group("cpu").strip()


def source_record(section: str, text: str, root: Path) -> tuple[Path, list[str]]:
    """The cited .json record, plus findings about any .md citation beside it."""
    cited = list(dict.fromkeys(SOURCE.findall(section)))
    if not cited:
        raise Fail(
            f"{DOC} does not name a source record. Link the "
            "`benchmarks/results/<date>-<machine>.json` the figures were read from, "
            "so this gate knows what to compare them against."
        )
    if len(cited) > 1:
        raise Fail(
            f"{DOC}'s `## Measured` section cites more than one record "
            f"({', '.join(cited)}). One table is read out of one run; with two "
            "citations this gate cannot say which one the digits should equal."
        )
    path = (root / "docs" / cited[0]).resolve()
    if not path.is_file():
        raise Fail(f"{DOC} cites {cited[0]}, which is not a file")
    return path, referenced_records(text, path, root)


def referenced_records(text: str, record: Path, root: Path) -> list[str]:
    """Findings about every benchmarks/results path the document names.

    The record itself is one of them and passes trivially. Everything else has
    to exist and name the same run, whatever markdown syntax it is written in.
    """
    findings = []
    for name in dict.fromkeys(RESULTS_REF.findall(text)):
        named = (root / "benchmarks" / "results" / name).resolve()
        # Existence first, then stem. Staged the other way round, the case for
        # the stem comparison was caught by this branch instead and the stem
        # comparison went untested — a mutation sweep is how that surfaced.
        if not named.is_file():
            findings.append(f"{DOC} names benchmarks/results/{name}, which is not a file")
        elif named.stem != record.stem:
            findings.append(
                f"{DOC} names benchmarks/results/{name} and reads its figures from "
                f"{record.name}. Those are different runs; name the summary of the "
                "run the table is read from."
            )
    return findings


def row_set(seen: list[tuple[str, str]], required: list[tuple[str, str]]) -> list[str]:
    """Findings about the table's row set, in both directions."""
    want = collections.Counter(required)
    got = collections.Counter(seen)
    script = Path(__file__).name
    findings = []
    for chart, gesture in sorted(want - got):
        findings.append(
            f"{DOC}'s table has no '{chart} | {gesture}' row, which PUBLISHED_ROWS "
            f"in {script} says this page publishes. A row cannot leave the table "
            "without leaving that list too."
        )
    for chart, gesture in sorted(got - want):
        if want[(chart, gesture)]:
            findings.append(f"{DOC}'s table repeats the '{chart} | {gesture}' row")
        else:
            findings.append(
                f"{DOC}'s table has a '{chart} | {gesture}' row that PUBLISHED_ROWS "
                f"in {script} does not list. Add it there, with the names it needs "
                "in SCENARIO/FIELD, or take the row out."
            )
    return findings


def figures(record: dict, rows: int) -> dict[tuple[str, str, str], tuple[str, str]]:
    """(scenario, field, half) -> (p50, p95) as the table would print them."""
    out = {}
    for entry in record.get("scaling", []):
        if entry.get("rows") != rows:
            continue
        for half in ("engine", "engine_direct"):
            for field, timing in (entry.get(half) or {}).items():
                if not isinstance(timing, dict) or "p50_ms" not in timing:
                    continue
                out[(entry["scenario"], field, half)] = (
                    one_dp(timing["p50_ms"]),
                    one_dp(timing["p95_ms"]),
                )
    return out


def counters(record: dict, rows: int) -> dict[tuple[str, str], dict]:
    """(scenario, half) -> the run's own preagg counters for it."""
    out = {}
    for entry in record.get("scaling", []):
        if entry.get("rows") != rows:
            continue
        for half in ("engine", "engine_direct"):
            preagg = (entry.get(half) or {}).get("preagg")
            if isinstance(preagg, dict):
                out[(entry["scenario"], half)] = preagg
    return out


def cube_claim(
    row: str, cell: str, half: str, scenario: str, preagg: dict | None, record_name: str
) -> str | None:
    """What the cube columns assert, checked against the record. None if it holds."""
    marker = bool(NO_CUBE.search(cell))
    if marker and half == "engine_direct":
        return (
            f"{DOC}: '{row}' says `*no cube possible*` in the `without` column. "
            "That column is measured with pre-aggregation disabled for every row, "
            "so the marker asserts nothing there and leaves the figure unpinned — "
            "print the measured value."
        )
    if half != "engine":
        return None
    if preagg is None:
        return (
            f"{record_name} carries no preagg counters for {scenario}, which the "
            f"`with a cube` column of '{row}' is a claim about"
        )
    if marker:
        if preagg.get("cubes_built"):
            return (
                f"{DOC}: '{row}' says `*no cube possible*`; {record_name} built "
                f"{preagg['cubes_built']} cube(s) for {scenario} and served "
                f"{preagg.get('cube_hits', 0)} mark re-queries from them"
            )
        return None
    if not preagg.get("cube_hits"):
        return (
            f"{DOC}: '{row}' prints a figure under `with a cube`; {record_name} "
            f"served 0 mark re-queries from a cube for {scenario}, so that figure "
            "was not measured with one"
        )
    return None


def check(root: Path, required: list[tuple[str, str]] | None = None) -> list[str]:
    doc = root / DOC
    if not doc.is_file():
        raise Fail(f"{DOC} is missing")
    text = doc.read_text()
    section = measured_section(text)
    record_path, findings = source_record(section, text, root)
    try:
        raw = record_path.read_text()
    except OSError as exc:
        # Reachable only if the existence check above has been broken, which a
        # mutation sweep does. A refusal names the file; a traceback does not.
        raise Fail(f"{record_path.name} could not be read: {exc}") from exc
    try:
        record = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise Fail(f"{record_path.name} is not readable JSON: {exc}") from exc

    rows, machine = headline(section)
    cpu = (record.get("machine") or {}).get("cpu")
    if machine != cpu:
        findings.append(
            f"{DOC} says the figures are from an {machine}; "
            f"{record_path.name} was captured on {cpu!r}"
        )
    if rows not in (record.get("config") or {}).get("rows", []):
        findings.append(
            f"{record_path.name} has no {rows}-row suite, which is the row count "
            f"{DOC} quotes"
        )

    known = figures(record, rows)
    preaggs = counters(record, rows)
    seen: list[tuple[str, str]] = []
    for cells in measured_table(section):
        # The pair is recorded before the guards below, so a row that is
        # malformed or unresolvable still counts toward the row set and is not
        # reported twice as missing.
        if len(cells) >= 2:
            seen.append((cells[0], cells[1]))
        if len(cells) < 4:
            findings.append(f"{DOC}: table row {cells!r} has fewer than four columns")
            continue
        chart, gesture, cubed, direct = cells[0], cells[1], cells[2], cells[3]
        row = f"{chart} | {gesture}"
        if chart not in SCENARIO or gesture not in FIELD:
            findings.append(
                f"{DOC}: row '{row}' names no benchmark scenario this "
                f"gate can resolve. Add it to SCENARIO/FIELD in {Path(__file__).name} "
                "or the figures go unchecked."
            )
            continue
        scenario, field = SCENARIO[chart], FIELD[gesture]
        for cell, half in ((cubed, "engine"), (direct, "engine_direct")):
            claim = cube_claim(
                row, cell, half, scenario, preaggs.get((scenario, half)), record_path.name
            )
            if claim:
                findings.append(claim)
                continue
            if NO_CUBE.search(cell):
                continue
            match = CELL.search(cell)
            if not match:
                findings.append(
                    f"{DOC}: '{row}' cell {cell!r} carries no "
                    "`<p50> ms (<p95>)` figure this gate can read"
                )
                continue
            want = known.get((scenario, field, half))
            if want is None:
                findings.append(
                    f"{record_path.name} has no {field} for {scenario} at {rows} rows "
                    f"in its {half} half, which '{row}' quotes"
                )
                continue
            got = (match.group(1), match.group(2))
            if got != want:
                findings.append(
                    f"{DOC}: '{row}' ({COLUMN[half]}) says "
                    f"{got[0]} ms ({got[1]}); {record_path.name} measured "
                    f"{want[0]} ms ({want[1]})"
                )
    findings.extend(row_set(seen, PUBLISHED_ROWS if required is None else required))
    return findings


# --------------------------------------------------------------------------
# Self-test. It runs in CI beside the gate, because a checker that has quietly
# stopped comparing anything reports exactly what a clean tree reports.
#
# Every case that must fail also names the finding it expects. Without that a
# case can keep passing while the check it was written for has stopped running,
# because some unrelated check caught the same mutation — which is the failure
# this whole file is a correction for.
# --------------------------------------------------------------------------

FIXTURE_ROWS = 10_000_000
FIXTURE_CPU = "Apple M1 Pro"

# The fixture document and the shipped one publish different row sets, so each
# case says which one it is being read against. PUBLISHED_ROWS — the list the
# real document is held to — is exercised by the gate itself, not from here.
FIXTURE_PUBLISHED_ROWS = [
    ("density", "zoom"),
    ("density", "brush"),
    ("raw scatter, two views", "zoom"),
]
SHIPPED_PUBLISHED_ROWS = [
    ("binned density", "zoom"),
    ("density", "zoom"),
    ("density", "brush"),
    ("binned density", "brush"),
    ("raw scatter, two views", "zoom"),
]

# One citation of a run the table is not read from, written six ways that GitHub
# renders identically. Each is a case below: the finding has to be the stem
# comparison naming `other.md`, so a case cannot pass because the scan mangled
# the path into something that is merely not a file.
LINK_SYNTAXES = [
    ("an anchor on the end of the path", "[Arrow held](../benchmarks/results/other.md#arrow-held)"),
    ("a link title", '[Arrow held](../benchmarks/results/other.md "Arrow held")'),
    ("a reference definition", "[Arrow held][summary]\n\n[summary]: ../benchmarks/results/other.md"),
    ("a repo-absolute path", "[Arrow held](/benchmarks/results/other.md)"),
    ("an angle-bracket target", "[Arrow held](<../benchmarks/results/other.md>)"),
    ("a raw HTML anchor", '<a href="../benchmarks/results/other.md">Arrow held</a>'),
]

RECORD_FIXTURE = {
    "machine": {"cpu": FIXTURE_CPU},
    "config": {"rows": [10_000, FIXTURE_ROWS]},
    "scaling": [
        {
            "scenario": "brush-density",
            "rows": FIXTURE_ROWS,
            "engine": {
                "coordinator_apply": {"p50_ms": 5.065, "p95_ms": 5.769},
                "navigation_apply": {"p50_ms": 2.643, "p95_ms": 3.402},
                "preagg": {"enabled": True, "cubes_built": 2, "cube_hits": 40},
            },
            "engine_direct": {
                "coordinator_apply": {"p50_ms": 82.035, "p95_ms": 91.165},
                "navigation_apply": {"p50_ms": 80.582, "p95_ms": 88.787},
                "preagg": {"enabled": False, "cubes_built": 0, "cube_hits": 0},
            },
        },
        {
            "scenario": "crossfilter-dots",
            "rows": FIXTURE_ROWS,
            "engine": {
                "navigation_apply": {"p50_ms": 155.635, "p95_ms": 351.409},
                "preagg": {"enabled": True, "cubes_built": 0, "cube_hits": 0},
            },
            "engine_direct": {
                "navigation_apply": {"p50_ms": 166.985, "p95_ms": 238.873},
                "preagg": {"enabled": False, "cubes_built": 0, "cube_hits": 0},
            },
        },
    ],
}

DOC_FIXTURE = """# What makes an interaction fast

Prose above the table, citing
[`benchmarks/results/fixture.md`](../benchmarks/results/fixture.md) for a column
that lives in the prose summary.

## Measured

Ten million rows, on an Apple M1 Pro, median with the 95th percentile beside it.
Every cell below is read from
[`benchmarks/results/fixture.json`](../benchmarks/results/fixture.json).

| chart | gesture | with a cube | without |
|---|---|---|---|
| density | zoom | **2.6 ms** (3.4) | 80.6 ms (88.8) |
| density | brush | **5.1 ms** (5.8) | 82.0 ms (91.2) |
| raw scatter, two views | zoom | *no cube possible* | 167.0 ms (238.9) |

## After

Prose below the table.
"""

# docs/interaction-speed.md's `## Measured` section as it shipped before this
# gate existed, in its own digits. Two cases below run it: as written, and with
# its citation pointed at 2026-07-27-apple-m1-pro.json — the run it was quoting.
SHIPPED_TABLE = """| chart | gesture | with a cube | without |
|---|---|---|---|
| binned density | zoom | **0.5 ms** (0.6) | 64.5 ms |
| density | zoom | **2.4 ms** (3.6) | 75.7 ms |
| density | brush | **4.0 ms** (5.0) | 90.6 ms |
| binned density | brush | **0.7 ms** (1.2) | 76.5 ms |
| raw scatter, two views | zoom | *no cube possible* | 157.7 ms |
"""

SHIPPED_DOC = f"""# What makes an interaction fast

## Measured

Ten million rows, on an Apple M1 Pro, median with the 95th percentile beside it.
The full record with its methodology is in [`benchmarks/results/`](../benchmarks/results/).

{SHIPPED_TABLE}
## When you do not get one, and what to do

Prose below the table.
"""

SHIPPED_DOC_CITED = SHIPPED_DOC.replace(
    "The full record with its methodology is in [`benchmarks/results/`](../benchmarks/results/).",
    "Every cell below is read from\n"
    "[`benchmarks/results/2026-07-27-apple-m1-pro.json`]"
    "(../benchmarks/results/2026-07-27-apple-m1-pro.json).",
)


def _cited_through(markup: str) -> str:
    """The fixture document with one more citation above the `## Measured` heading."""
    return DOC_FIXTURE.replace("## Measured", f"{markup}\n\n## Measured", 1)


def _stage(tmp: Path, doc: str, record: dict | str, name: str = "fixture") -> Path:
    root = tmp / "repo"
    (root / "docs").mkdir(parents=True, exist_ok=True)
    (root / "benchmarks" / "results").mkdir(parents=True, exist_ok=True)
    (root / DOC).write_text(doc)
    results = root / "benchmarks" / "results"
    if isinstance(record, Path):
        shutil.copyfile(record, results / f"{name}.json")
    else:
        body = record if isinstance(record, str) else json.dumps(record)
        (results / f"{name}.json").write_text(body)
    (results / f"{name}.md").write_text("| Arrow held (MiB) |\n")
    # `other.md` has to EXIST, or the case for the stem comparison is caught by
    # the existence branch and the comparison itself goes untested.
    (results / "other.md").write_text("| Arrow held (MiB) |\n")
    return root


def self_test() -> int:
    # (name, document, record, should_pass[, substring the finding must contain]
    #  [, the record's stem][, the row set the document is read against])
    cases: list[tuple] = [
        ("the fixture as published", DOC_FIXTURE, RECORD_FIXTURE, True),
        (
            "a median wrong by one tenth",
            DOC_FIXTURE.replace("**5.1 ms** (5.8)", "**5.0 ms** (5.8)"),
            RECORD_FIXTURE,
            False,
            "says 5.0 ms (5.8); fixture.json measured 5.1 ms (5.8)",
        ),
        (
            "a 95th percentile wrong by one tenth",
            DOC_FIXTURE.replace("82.0 ms (91.2)", "82.0 ms (91.3)"),
            RECORD_FIXTURE,
            False,
            "says 82.0 ms (91.3); fixture.json measured 82.0 ms (91.2)",
        ),
        (
            "a figure taken from the wrong row of the record",
            DOC_FIXTURE.replace("82.0 ms (91.2)", "80.6 ms (88.8)"),
            RECORD_FIXTURE,
            False,
            "'density | brush' (without)",
        ),
        (
            "the record moved on and the document did not",
            DOC_FIXTURE,
            {
                **RECORD_FIXTURE,
                "scaling": [
                    {
                        **RECORD_FIXTURE["scaling"][0],
                        "engine": {
                            **RECORD_FIXTURE["scaling"][0]["engine"],
                            "coordinator_apply": {"p50_ms": 7.2, "p95_ms": 8.1},
                        },
                    },
                    RECORD_FIXTURE["scaling"][1],
                ],
            },
            False,
            "fixture.json measured 7.2 ms (8.1)",
        ),
        (
            "the cited record does not exist",
            DOC_FIXTURE.replace("fixture.json", "gone.json"),
            RECORD_FIXTURE,
            False,
            "COULD NOT RUN: docs/interaction-speed.md cites ../benchmarks/results/gone.json, which is not a file",
        ),
        (
            "no record is cited at all",
            DOC_FIXTURE.replace(
                "[`benchmarks/results/fixture.json`](../benchmarks/results/fixture.json)",
                "the benchmark record",
            ),
            RECORD_FIXTURE,
            False,
            "COULD NOT RUN: docs/interaction-speed.md does not name a source record",
        ),
        (
            "two records are cited and the gate cannot say which the digits equal",
            DOC_FIXTURE.replace(
                "| density | brush |",
                "and also [`x`](../benchmarks/results/other.json)\n\n| density | brush |",
            ),
            RECORD_FIXTURE,
            False,
            "COULD NOT RUN: docs/interaction-speed.md's `## Measured` section cites more than one record",
        ),
        (
            "the prose summary links a different run from the table",
            DOC_FIXTURE.replace(
                "(../benchmarks/results/fixture.md)", "(../benchmarks/results/other.md)"
            ),
            RECORD_FIXTURE,
            False,
            "names benchmarks/results/other.md and reads its figures from fixture.json",
        ),
        (
            "the prose summary link leads nowhere",
            DOC_FIXTURE.replace(
                "(../benchmarks/results/fixture.md)", "(../benchmarks/results/gone.md)"
            ),
            RECORD_FIXTURE,
            False,
            "names benchmarks/results/gone.md, which is not a file",
        ),
        # One case per link syntax. Each cites a run the table is not read from,
        # so a scan that stops at the shape of a plain `(path)` link goes silent
        # on it — which is how a deep link into the prose summary un-gated that
        # citation while the page said it was checked.
        *[
            (
                f"a different run cited through {syntax}",
                _cited_through(markup),
                RECORD_FIXTURE,
                False,
                "names benchmarks/results/other.md and reads its figures from fixture.json",
            )
            for syntax, markup in LINK_SYNTAXES
        ],
        # The row set, in both directions. Every digit in these tables is
        # correct, so only the row-set comparison can catch them.
        (
            "the row-level row deleted from the table",
            DOC_FIXTURE.replace(
                "| raw scatter, two views | zoom | *no cube possible* | 167.0 ms (238.9) |\n",
                "",
            ),
            RECORD_FIXTURE,
            False,
            "has no 'raw scatter, two views | zoom' row",
        ),
        (
            "a published row repeated",
            DOC_FIXTURE.replace(
                "| density | brush | **5.1 ms** (5.8) | 82.0 ms (91.2) |",
                "| density | brush | **5.1 ms** (5.8) | 82.0 ms (91.2) |\n"
                "| density | brush | **5.1 ms** (5.8) | 82.0 ms (91.2) |",
            ),
            RECORD_FIXTURE,
            False,
            "repeats the 'density | brush' row",
        ),
        (
            "a row the published list does not carry",
            DOC_FIXTURE.replace(
                "| raw scatter, two views | zoom | *no cube possible* | 167.0 ms (238.9) |",
                "| raw scatter, two views | zoom | *no cube possible* | 167.0 ms (238.9) |\n"
                "| raw scatter, two views | brush | *no cube possible* | 200.0 ms (300.0) |",
            ),
            RECORD_FIXTURE,
            False,
            "has a 'raw scatter, two views | brush' row that PUBLISHED_ROWS",
        ),
        # Removing the column-count guard makes the unpacking below it raise,
        # and a raise is not a report: without this case `if len(cells) < 4:`
        # could be deleted at self-test exit 0.
        (
            "a table row with a column missing",
            DOC_FIXTURE.replace(
                "| raw scatter, two views | zoom | *no cube possible* | 167.0 ms (238.9) |",
                "| raw scatter, two views | zoom | *no cube possible* |",
            ),
            RECORD_FIXTURE,
            False,
            "has fewer than four columns",
        ),
        (
            "the record was captured on another machine",
            DOC_FIXTURE,
            {**RECORD_FIXTURE, "machine": {"cpu": "Apple M4 Max"}},
            False,
            "says the figures are from an Apple M1 Pro",
        ),
        # The three that follow are the drift direction that matters for a
        # public page: the PAGE is rewritten and the record is left alone.
        (
            "the page's machine rewritten over an unchanged record",
            DOC_FIXTURE.replace("on an Apple M1 Pro", "on an Apple M4 Max"),
            RECORD_FIXTURE,
            False,
            "says the figures are from an Apple M4 Max",
        ),
        (
            "the page's row count rewritten over an unchanged record",
            DOC_FIXTURE.replace("Ten million rows", "One hundred rows"),
            RECORD_FIXTURE,
            False,
            "has no 100-row suite",
        ),
        (
            "both halves of the opening sentence rewritten over an unchanged record",
            DOC_FIXTURE.replace(
                "Ten million rows, on an Apple M1 Pro",
                "One hundred rows, on an Apple M4 Max",
            ),
            RECORD_FIXTURE,
            False,
            "has no 100-row suite",
        ),
        (
            "the same row count written in digits",
            DOC_FIXTURE.replace("Ten million rows", "10,000,000 rows"),
            RECORD_FIXTURE,
            True,
        ),
        (
            "the opening sentence is gone",
            DOC_FIXTURE.replace(
                "Ten million rows, on an Apple M1 Pro, median", "Medians"
            ),
            RECORD_FIXTURE,
            False,
            "COULD NOT RUN: docs/interaction-speed.md's `## Measured` section has no sentence of the form",
        ),
        (
            "a row count nothing can read as a number",
            DOC_FIXTURE.replace("Ten million rows", "Umpteen zillion rows"),
            RECORD_FIXTURE,
            False,
            "COULD NOT RUN: docs/interaction-speed.md says 'Umpteen zillion' rows, which this gate cannot read as a number",
        ),
        # The cube columns. Each of these leaves every digit in the table
        # correct, so only the preagg check can catch it.
        (
            "a measured cell swapped for `*no cube possible*` where a cube was built",
            DOC_FIXTURE.replace("**5.1 ms** (5.8)", "*no cube possible*"),
            RECORD_FIXTURE,
            False,
            "built 2 cube(s) for brush-density",
        ),
        (
            "`*no cube possible*` used in the `without` column",
            DOC_FIXTURE.replace("80.6 ms (88.8)", "*no cube possible*"),
            RECORD_FIXTURE,
            False,
            "in the `without` column",
        ),
        (
            "a cubed figure claimed for a run that served nothing from a cube",
            DOC_FIXTURE.replace("*no cube possible*", "**155.6 ms** (351.4)"),
            RECORD_FIXTURE,
            False,
            "served 0 mark re-queries from a cube for crossfilter-dots",
        ),
        (
            "the record dropped the preagg counters the cube column relies on",
            DOC_FIXTURE,
            {
                **RECORD_FIXTURE,
                "scaling": [
                    {
                        **RECORD_FIXTURE["scaling"][0],
                        "engine": {
                            k: v
                            for k, v in RECORD_FIXTURE["scaling"][0]["engine"].items()
                            if k != "preagg"
                        },
                    },
                    RECORD_FIXTURE["scaling"][1],
                ],
            },
            False,
            "carries no preagg counters for brush-density",
        ),
        (
            "a table row this gate cannot resolve to a scenario",
            DOC_FIXTURE.replace("| density | brush |", "| hexbin over a cube | brush |"),
            RECORD_FIXTURE,
            False,
            "names no benchmark scenario this",
        ),
        (
            "the Measured section lost its table",
            DOC_FIXTURE.split("| chart")[0] + "\n## After\n",
            RECORD_FIXTURE,
            False,
            "COULD NOT RUN: docs/interaction-speed.md's `## Measured` section has no table rows",
        ),
        (
            "the document lost its Measured section",
            DOC_FIXTURE.replace("## Measured", "## Timings"),
            RECORD_FIXTURE,
            False,
            "COULD NOT RUN: docs/interaction-speed.md has no `## Measured` section",
        ),
        (
            "the record stopped running the row count the document quotes",
            DOC_FIXTURE,
            {**RECORD_FIXTURE, "config": {"rows": [10_000]}},
            False,
            "has no 10000000-row suite",
        ),
        (
            "the record is not readable JSON",
            DOC_FIXTURE,
            "{ this is not json",
            False,
            "COULD NOT RUN: fixture.json is not readable JSON",
        ),
        (
            "a rounding boundary reads half-up, not half-even",
            DOC_FIXTURE.replace("**2.6 ms** (3.4)", "**2.7 ms** (3.4)"),
            {
                **RECORD_FIXTURE,
                "scaling": [
                    {
                        **RECORD_FIXTURE["scaling"][0],
                        "engine": {
                            **RECORD_FIXTURE["scaling"][0]["engine"],
                            "navigation_apply": {"p50_ms": 2.65, "p95_ms": 3.402},
                        },
                    },
                    RECORD_FIXTURE["scaling"][1],
                ],
            },
            True,
        ),
        # The state this gate was written for. It shipped citing a directory
        # rather than a run, so it stops at the citation...
        (
            "the table as it shipped, naming no run at all",
            SHIPPED_DOC,
            RECORD_FIXTURE,
            False,
            "COULD NOT RUN: docs/interaction-speed.md does not name a source record",
            "fixture",
            SHIPPED_PUBLISHED_ROWS,
        ),
        # ...and the same table read against the run it was quoting. The one
        # case built from a committed record; it sits in this list rather than
        # beside it so `len(cases)` counts what the loop runs.
        _shipped_against_the_run_it_quoted(),
    ]

    failures = 0
    for case in cases:
        name, doc, record, should_pass = case[:4]
        expect = case[4] if len(case) > 4 else None
        record_name = case[5] if len(case) > 5 else "fixture"
        required = case[6] if len(case) > 6 else FIXTURE_PUBLISHED_ROWS
        if not should_pass and not expect:
            # Without this a must-fail case can be added with no expectation and
            # go on passing while the check it was written for has stopped
            # running, because some other check caught the same mutation.
            failures += 1
            print(
                f"SELF-TEST FAILED: must-fail case {name!r} names no finding it "
                "expects, so nothing holds it to its own reason",
                file=sys.stderr,
            )
        with tempfile.TemporaryDirectory() as tmp:
            try:
                # Staging is inside the try because one case copies a committed
                # record rather than writing a fixture, and a missing file there
                # must be reported against that case rather than abort the run.
                root = _stage(Path(tmp), doc, record, name=record_name)
                findings = check(root, required)
                passed = not findings
                detail = "; ".join(findings)
            except Fail as exc:
                passed = False
                detail = f"COULD NOT RUN: {exc}"
            except Exception as exc:  # noqa: BLE001 - see below
                # One case that raises must not abort the ones after it.
                # Removing a guard clause makes the code it guards crash, and a
                # traceback out of here reports nothing about the rest.
                passed = False
                detail = f"CRASHED: {type(exc).__name__}: {exc}"
        if passed != should_pass:
            failures += 1
            if should_pass:
                print(f"SELF-TEST FAILED: cried wolf on {name}: {detail}", file=sys.stderr)
            else:
                print(f"SELF-TEST FAILED: stayed silent on {name}", file=sys.stderr)
        elif expect and expect not in detail:
            failures += 1
            print(
                f"SELF-TEST FAILED: caught {name}, but not for the reason the case "
                f"was written for.\n  expected a finding containing: {expect}\n"
                f"  got: {detail}",
                file=sys.stderr,
            )

    failures += _end_to_end_self_test()
    if failures:
        return 1
    print(f"measured-figures gate self-test: ok ({len(cases)} cases, and end to end)")
    return 0


def _end_to_end_self_test() -> int:
    """The exit codes `main` returns, over a copy of this checkout.

    Every case above calls `check()` directly. Nothing above reaches `main`, so
    its reporting branch could be forced to return 0 over a document full of
    wrong digits and the whole list would still pass — and `main` is what the CI
    step runs. The document is copied from this tree rather than synthesised,
    because the row set `main` holds it to is PUBLISHED_ROWS and this is the
    document that carries it.
    """
    script = Path(__file__).resolve()
    root = repo_root()
    doc = (root / DOC).read_text()
    cases = [
        ("this checkout, unmodified", doc, 0),
        ("one median off by a tenth", doc.replace("**5.1 ms**", "**5.2 ms**"), 1),
        ("the Measured heading renamed", doc.replace("## Measured", "## Timings"), 2),
    ]
    failures = 0
    for name, text, expected in cases:
        if expected and text == doc:
            failures += 1
            print(
                f"SELF-TEST FAILED: end to end, {name}: the mutation changed nothing, "
                "so this case is reading an unmodified document",
                file=sys.stderr,
            )
            continue
        with tempfile.TemporaryDirectory() as tmp:
            staged = Path(tmp) / "repo"
            (staged / "docs").mkdir(parents=True)
            shutil.copytree(
                root / "benchmarks" / "results", staged / "benchmarks" / "results"
            )
            (staged / DOC).write_text(text)
            subprocess.run(["git", "init", "-q", str(staged)], capture_output=True, check=True)
            got = subprocess.run(
                [sys.executable, str(script)], cwd=staged, capture_output=True
            ).returncode
        if got != expected:
            failures += 1
            print(
                f"SELF-TEST FAILED: end to end, {name}: expected exit {expected}, "
                f"got {got}",
                file=sys.stderr,
            )
    return failures


def _shipped_against_the_run_it_quoted() -> tuple:
    """...and read against 2026-07-27-apple-m1-pro.json, it is rejected on digits.

    This is the one case that reads a committed record rather than a fixture,
    because the claim in WHY THIS EXISTS is about that file and no other. It is
    returned as an ordinary case and sits in the list with the rest, so the loop
    runs it and `len(cases)` counts it. It used to run itself and be counted by a `+ 1`
    beside the list, which meant the banner named a case the run could no longer
    reach: dropping its result left the self-test at exit 0 still claiming it.

    If the record is missing, staging raises inside the loop, the case records
    CRASHED, and the expected finding below does not match it — so the case
    reports rather than passing on a technicality. On a tree that is not a git
    checkout, `repo_root()` below raises while this case list is being built
    and before the loop runs, so `--self-test` exits non-zero on that rather
    than on a case verdict.
    """
    record = "2026-07-27-apple-m1-pro"
    return (
        "the table as it shipped, read against the run it was quoting",
        SHIPPED_DOC_CITED,
        repo_root() / "benchmarks" / "results" / f"{record}.json",
        False,
        "'density | brush' (with a cube) says 4.0 ms (5.0)",
        record,
        SHIPPED_PUBLISHED_ROWS,
    )


def main(argv: list[str]) -> int:
    if "--self-test" in argv:
        return self_test()
    try:
        findings = check(repo_root())
    except Fail as exc:
        print(f"MEASURED-FIGURES GATE COULD NOT RUN: {exc}", file=sys.stderr)
        return 2
    if findings:
        print(
            "Measured figures disagree with the record they are read from.\n"
            "This document is where the README and the coordinator rustdoc send a\n"
            "reader for the size of the gap, so a stale digit here is a public\n"
            "claim nobody measured.\n",
            file=sys.stderr,
        )
        for finding in findings:
            print(f"  {finding}", file=sys.stderr)
        print(
            "\nFix: read the cells back out of the record, or cite the record the\n"
            "figures actually came from. Re-measure with ./scripts/bench-baseline.sh.",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
