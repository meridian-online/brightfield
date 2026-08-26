#!/usr/bin/env python3
"""Pin the latency figures in docs/interaction-speed.md to the record they came from.

WHY THIS EXISTS
    That document is what the README and the coordinator rustdoc send a reader
    to for the size of the gap between an aggregating mark and a row-level one.
    It carried a five-row table of millisecond figures and nothing kept them in
    step with `benchmarks/results/`. Read against the records on 2026-08-26,
    four of its five rows disagreed with every committed run: two cells held a
    number from the wrong row of a summary table (a slider scenario's maximum,
    printed as a density scenario's median), and the table as a whole was a run
    behind — quoting 2026-07-27 while 2026-08-07 shipped.

    `scripts/check-borrowed-benchmarks.sh` already stops UPSTREAM figures being
    restated as ours. This is the other half: our own figures, restated in prose
    and free to rot. The repo has the same rule for constants in Rust sources
    (`crates/brightfield-render/tests/one_ceiling.rs` fails when a measured
    ceiling is copied a second time); this applies it to a copy made in
    markdown.

WHAT IS CHECKED
    1. The document names its source record as a repo-relative path, and that
       file exists. A renamed or deleted record fails here, before any digit is
       compared.
    2. Every latency cell in the `## Measured` table equals the corresponding
       figure in that record, to the one decimal place the table prints.
       Rounding is half-up on the raw millisecond value, so 5.065 reads 5.1.
    3. The table's row set is exactly the mapping below — a row added to the
       document without a mapping fails, rather than passing unchecked.

WHAT IS *NOT* CHECKED (stated so nobody reads this as more than it is)
    - It does not check that the named record is the NEWEST one. Figures stay
      true of the run they cite, and a record captured on another machine is
      not automatically the one this document should quote. Choosing which run
      to publish is a judgement; keeping the digits equal to it is not, which
      is why only the second half is a gate.
    - It does not check the prose around the table. "Ten million rows, on an
      Apple M1 Pro" is verified against the record's `rows` and `machine.cpu`
      because both are in the record; sentences like "the cube is usually
      thousands of rows" are not, and no gate can judge them.
    - It reads only this one document. A latency figure written into any other
      file is out of scope and unpinned.

Usage (no arguments, from anywhere inside the repo):

    ./scripts/check-measured-figures.py
    ./scripts/check-measured-figures.py --self-test

Exit codes:
    0  clean
    1  one or more figures disagree with the record
    2  the gate could not run (document or record unreadable, table missing)
"""

from __future__ import annotations

import decimal
import json
import re
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
ROWS = 10_000_000
MACHINE_CPU = "Apple M1 Pro"

# `**5.1 ms** (5.8)` or `82.0 ms (91.2)` — a median with its 95th percentile.
CELL = re.compile(r"\*{0,2}(\d+\.\d)\s*ms\*{0,2}\s*\((\d+\.\d)\)")
# `*no cube possible*` — a row-level mark has no cube, so there is no cubed
# figure to compare and the cell asserts that rather than a number.
NO_CUBE = re.compile(r"\*no cube possible\*")
# The source record, cited as a repo-relative path inside a markdown link.
SOURCE = re.compile(r"\((\.\./benchmarks/results/[A-Za-z0-9._-]+\.json)\)")


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


def measured_table(text: str) -> list[list[str]]:
    """The rows of the `## Measured` table, each split into its cells."""
    section = text.split("## Measured", 1)
    if len(section) != 2:
        raise Fail(f"{DOC} has no `## Measured` section to check")
    body = section[1].split("\n## ", 1)[0]
    rows = []
    for line in body.splitlines():
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


def source_record(text: str, root: Path) -> Path:
    match = SOURCE.search(text)
    if not match:
        raise Fail(
            f"{DOC} does not name a source record. Link the "
            "`benchmarks/results/<date>-<machine>.json` the figures were read from, "
            "so this gate knows what to compare them against."
        )
    path = (root / "docs" / match.group(1)).resolve()
    if not path.is_file():
        raise Fail(f"{DOC} cites {match.group(1)}, which is not a file")
    return path


def figures(record: dict) -> dict[tuple[str, str, str], tuple[str, str]]:
    """(scenario, field, half) -> (p50, p95) as the table would print them."""
    out = {}
    for entry in record.get("scaling", []):
        if entry.get("rows") != ROWS:
            continue
        for half in ("engine", "engine_direct"):
            for field, timing in (entry.get(half) or {}).items():
                if not isinstance(timing, dict) or "p50_ms" not in timing:
                    continue
                key = (entry["scenario"], field, half)
                out[key] = (one_dp(timing["p50_ms"]), one_dp(timing["p95_ms"]))
    return out


def check(root: Path) -> list[str]:
    doc = root / DOC
    if not doc.is_file():
        raise Fail(f"{DOC} is missing")
    text = doc.read_text()
    record_path = source_record(text, root)
    try:
        record = json.loads(record_path.read_text())
    except json.JSONDecodeError as exc:
        raise Fail(f"{record_path.name} is not readable JSON: {exc}") from exc

    findings = []
    cpu = (record.get("machine") or {}).get("cpu")
    if cpu != MACHINE_CPU:
        findings.append(
            f"{DOC} says the figures are from an {MACHINE_CPU}; "
            f"{record_path.name} was captured on {cpu!r}"
        )
    if ROWS not in (record.get("config") or {}).get("rows", []):
        findings.append(
            f"{record_path.name} has no {ROWS}-row suite, which is the row count "
            f"{DOC} quotes"
        )

    known = figures(record)
    for cells in measured_table(text):
        if len(cells) < 4:
            findings.append(f"{DOC}: table row {cells!r} has fewer than four columns")
            continue
        chart, gesture, cubed, direct = cells[0], cells[1], cells[2], cells[3]
        if chart not in SCENARIO or gesture not in FIELD:
            findings.append(
                f"{DOC}: row '{chart} | {gesture}' names no benchmark scenario this "
                f"gate can resolve. Add it to SCENARIO/FIELD in {Path(__file__).name} "
                "or the figures go unchecked."
            )
            continue
        scenario, field = SCENARIO[chart], FIELD[gesture]
        for cell, half in ((cubed, "engine"), (direct, "engine_direct")):
            if NO_CUBE.search(cell):
                continue
            match = CELL.search(cell)
            if not match:
                findings.append(
                    f"{DOC}: '{chart} | {gesture}' cell {cell!r} carries no "
                    "`<p50> ms (<p95>)` figure this gate can read"
                )
                continue
            want = known.get((scenario, field, half))
            if want is None:
                findings.append(
                    f"{record_path.name} has no {field} for {scenario} at {ROWS} rows "
                    f"in its {half} half, which '{chart} | {gesture}' quotes"
                )
                continue
            got = (match.group(1), match.group(2))
            if got != want:
                findings.append(
                    f"{DOC}: '{chart} | {gesture}' ({half}) says "
                    f"{got[0]} ms ({got[1]}); {record_path.name} measured "
                    f"{want[0]} ms ({want[1]})"
                )
    return findings


# --------------------------------------------------------------------------
# Self-test. It runs in CI beside the gate, because a checker that has quietly
# stopped comparing anything reports exactly what a clean tree reports.
# --------------------------------------------------------------------------

RECORD_FIXTURE = {
    "machine": {"cpu": MACHINE_CPU},
    "config": {"rows": [10_000, ROWS]},
    "scaling": [
        {
            "scenario": "brush-density",
            "rows": ROWS,
            "engine": {
                "coordinator_apply": {"p50_ms": 5.065, "p95_ms": 5.769},
                "navigation_apply": {"p50_ms": 2.643, "p95_ms": 3.402},
            },
            "engine_direct": {
                "coordinator_apply": {"p50_ms": 82.035, "p95_ms": 91.165},
                "navigation_apply": {"p50_ms": 80.582, "p95_ms": 88.787},
            },
        },
        {
            "scenario": "crossfilter-dots",
            "rows": ROWS,
            "engine": {"navigation_apply": {"p50_ms": 155.635, "p95_ms": 351.409}},
            "engine_direct": {"navigation_apply": {"p50_ms": 166.985, "p95_ms": 238.873}},
        },
    ],
}

DOC_FIXTURE = """# What makes an interaction fast

Prose above the table.

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


def _stage(tmp: Path, doc: str, record: dict | str) -> Path:
    root = tmp / "repo"
    (root / "docs").mkdir(parents=True, exist_ok=True)
    (root / "benchmarks" / "results").mkdir(parents=True, exist_ok=True)
    (root / DOC).write_text(doc)
    body = record if isinstance(record, str) else json.dumps(record)
    (root / "benchmarks" / "results" / "fixture.json").write_text(body)
    return root


def self_test() -> int:
    cases: list[tuple[str, str, dict | str, bool]] = [
        ("the fixture as published", DOC_FIXTURE, RECORD_FIXTURE, True),
        (
            "a median wrong by one tenth",
            DOC_FIXTURE.replace("**5.1 ms** (5.8)", "**5.0 ms** (5.8)"),
            RECORD_FIXTURE,
            False,
        ),
        (
            "a 95th percentile wrong by one tenth",
            DOC_FIXTURE.replace("82.0 ms (91.2)", "82.0 ms (91.3)"),
            RECORD_FIXTURE,
            False,
        ),
        (
            "a figure taken from the wrong row of the record",
            DOC_FIXTURE.replace("82.0 ms (91.2)", "80.6 ms (88.8)"),
            RECORD_FIXTURE,
            False,
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
                            "coordinator_apply": {"p50_ms": 7.2, "p95_ms": 8.1},
                            "navigation_apply": {"p50_ms": 2.643, "p95_ms": 3.402},
                        },
                    },
                    RECORD_FIXTURE["scaling"][1],
                ],
            },
            False,
        ),
        (
            "the cited record does not exist",
            DOC_FIXTURE.replace("fixture.json", "gone.json"),
            RECORD_FIXTURE,
            False,
        ),
        (
            "no record is cited at all",
            DOC_FIXTURE.replace(
                "[`benchmarks/results/fixture.json`](../benchmarks/results/fixture.json)",
                "the benchmark record",
            ),
            RECORD_FIXTURE,
            False,
        ),
        (
            "the record was captured on another machine",
            DOC_FIXTURE,
            {**RECORD_FIXTURE, "machine": {"cpu": "Apple M4 Max"}},
            False,
        ),
        (
            "a table row this gate cannot resolve to a scenario",
            DOC_FIXTURE.replace(
                "| density | brush |", "| hexbin over a cube | brush |"
            ),
            RECORD_FIXTURE,
            False,
        ),
        (
            "the Measured section lost its table",
            DOC_FIXTURE.split("| chart")[0] + "\n## After\n",
            RECORD_FIXTURE,
            False,
        ),
        (
            "the document lost its Measured section",
            DOC_FIXTURE.replace("## Measured", "## Timings"),
            RECORD_FIXTURE,
            False,
        ),
        (
            "the record stopped running the row count the document quotes",
            DOC_FIXTURE,
            {**RECORD_FIXTURE, "config": {"rows": [10_000]}},
            False,
        ),
        (
            "the record is not readable JSON",
            DOC_FIXTURE,
            "{ this is not json",
            False,
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
                            "coordinator_apply": {"p50_ms": 5.065, "p95_ms": 5.769},
                            "navigation_apply": {"p50_ms": 2.65, "p95_ms": 3.402},
                        },
                    },
                    RECORD_FIXTURE["scaling"][1],
                ],
            },
            True,
        ),
    ]

    failures = 0
    for name, doc, record, should_pass in cases:
        with tempfile.TemporaryDirectory() as tmp:
            root = _stage(Path(tmp), doc, record)
            try:
                findings = check(root)
                passed = not findings
                detail = "; ".join(findings)
            except Fail as exc:
                passed = False
                detail = str(exc)
        if passed != should_pass:
            failures += 1
            if should_pass:
                print(f"SELF-TEST FAILED: cried wolf on {name}: {detail}", file=sys.stderr)
            else:
                print(f"SELF-TEST FAILED: stayed silent on {name}", file=sys.stderr)

    if failures:
        return 1
    print(f"measured-figures gate self-test: ok ({len(cases)} cases)")
    return 0


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
