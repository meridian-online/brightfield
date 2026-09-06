#!/usr/bin/env python3
"""Gate the release path by what its steps DO, not by whether a script is named.

    scripts/check-release-readback.py
    scripts/check-release-readback.py --workflow <release.yml>
    scripts/check-release-readback.py --self-test

WHAT WENT WRONG BEFORE THIS FILE
    scripts/check-finetype-pin.sh establishes that the string
    `scripts/check-artifact-type-source.sh` occurs somewhere in release.yml.
    Four separate edits satisfy that and remove the protection, each measured
    against the commit this file was written on, each leaving every other check
    in the repository green:

      * `continue-on-error: true` on the read-back step -- the release ships an
        artefact whose bundle does not load, on a green run
      * deleting the `.dmg` argument -- one occurrence of the name remains, and
        the disk image is the download the release notes recommend
      * dropping `scripts/package.sh`'s app-bundle staging call, or reducing it
        to a `mkdir -p` -- caught at tag time by the read-back, and only if
        neither of the two above is also true
      * piping the invocation, or trailing it with `|| true` -- the default
        shell for a `run:` block with no `shell:` key is `bash -e`, WITHOUT
        pipefail, so a piped check reports the exit status of `sed`

    A substring count answers none of those questions. This file asks them.

THE CHAIN IT READS, AND WHERE EACH LINK USED TO BE UNGUARDED

      1. what packaging produces      -- a fixture here, pinned by (A) below
      2. what the build job uploads   -- `path:` globs, `if-no-files-found`
      3. what the release publishes   -- `files:` globs, fail_on_unmatched_files
      4. what the read-back opens     -- guarded by a substring count

    Ported from the `check_release_asset_contract.py` idea in the sibling
    type-source repository: read the declaration under test as structure,
    refuse every shape it cannot read exactly rather than guessing, and get
    every other fact by RUNNING the thing rather than by parsing it.

WHAT IS CHECKED

    A  The build job contains a step that fails when any single one of the
       packaged artefacts is missing. This is a contract item in its own right
       -- `if-no-files-found: error` and `fail_on_unmatched_files: true` both
       fire on an EMPTY glob, and the tarball alone satisfies both, so a disk
       image that failed to build would drop out of the release on a green run.
       It is also what pins the fixture below: if packaging grows an artefact
       and that step grows with it, the fixture stops satisfying it and this
       file refuses (exit 2) rather than checking a set it made up.
    B  Every artefact in that set is matched by the build job's upload globs,
       every upload glob matches something, and `if-no-files-found: error`.
    C  Every uploaded artefact is matched by the release job's `files:` globs,
       every such glob matches something, and `fail_on_unmatched_files: true`.
    D  Every published artefact that `scripts/check-artifact-type-source.sh`
       can open is handed to it, on the leg that built it, with that leg's
       target triple as the second argument. Which artefacts it can open is
       asked of that script, not written down here.
    E  A read-back that fails stops the job: no `continue-on-error` on the step
       or on the job, no `if:` on the step, and -- driven, once per artefact --
       a failing check makes the step exit non-zero.
    F  What the check says about an artefact reaches the run: its stdout and
       its `$GITHUB_STEP_SUMMARY` writes both survive the step.

HOW D, E AND F ARE DECIDED: BY RUNNING THE STEP
    The read-back steps' `run:` blocks are executed in a temporary directory
    holding a fake `dist/` and a recorder in place of
    scripts/check-artifact-type-source.sh, with the workflow's `${{ }}`
    expressions resolved from the matrix leg under test. What is recorded is
    the argv of every invocation. A guard the reader did not predict -- an
    `if` around one call, a `case` on the target, a variable that resolves to
    an empty string -- changes which invocations happen, and that is the thing
    being measured. The shell is the one GitHub would use for that step: `bash
    -e` with no `shell:` key, `bash --noprofile --norc -eo pipefail` for
    `shell: bash`, and an unrecognised `shell:` is a refusal.

EXIT CODES
    0  the release reads back every artefact it publishes, and a failure stops it
    1  the contract is broken -- an unread artefact, a swallowed failure
    2  the check could not run: a workflow shape this reader cannot read
       exactly, or a fixture the build job would not notice

Stdlib only.
"""

from __future__ import annotations

import argparse
import glob
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
WORKFLOW_REL = ".github/workflows/release.yml"
READBACK_REL = "scripts/check-artifact-type-source.sh"

BUILD_JOB = "build"
RELEASE_JOB = "release"

UPLOAD_ACTION = "actions/upload-artifact"
RELEASE_ACTION = "softprops/action-gh-release"

# A tag shaped like the ones this workflow runs on, and deliberately not a real
# one: every path below is built from it, so a fixture leaking into a message
# is obvious.
FIXTURE_REF = "vREADBACK"

# The suffixes scripts/package.sh writes per target. This list is a fixture and
# not a claim -- contract item A refuses the run unless the build job itself
# would notice each of these going missing.
ARTIFACT_SUFFIXES = (".tar.gz", ".tar.gz.sha256", ".dmg", ".dmg.sha256")

# `run:` with no `shell:` key is `bash -e {0}` -- no pipefail, no -u. `shell:
# bash` is the other one, and it IS pipefail. The difference decides whether a
# piped read-back can fail a job, so it is read rather than assumed.
SHELLS = {
    None: ["bash", "-e"],
    "bash": ["bash", "--noprofile", "--norc", "-eo", "pipefail"],
}

RECORDER = """#!/usr/bin/env bash
# Stands in for scripts/check-artifact-type-source.sh while the release step is
# driven. Records argv, speaks on both channels the real check speaks on, and
# fails for one chosen artefact so that a swallowed failure is visible.
printf '%s\\n' "$*" >> "$BF_RECORD"
echo "RECORDER-STDOUT $*"
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  echo "RECORDER-SUMMARY $*" >> "$GITHUB_STEP_SUMMARY"
fi
if [ -n "${BF_FAIL_FOR:-}" ] && [ "${1:-}" = "$BF_FAIL_FOR" ]; then
  echo "RECORDER-REFUSES $1" >&2
  exit 1
fi
exit 0
"""


class Unreadable(Exception):
    """A shape this reader cannot read exactly. Exit 2, never a verdict."""


# ── reading the declaration under test ──────────────────────────────────────


def _indent(line: str) -> int:
    body = line.rstrip("\n")
    return len(body) - len(body.lstrip(" "))


def _is_skippable(line: str) -> bool:
    stripped = line.strip()
    return stripped == "" or stripped.startswith("#")


def _scalar(raw: str, where: str) -> str | bool:
    """A YAML scalar, in the narrow set this reader accepts."""
    text = raw.strip()
    if text.startswith("#"):
        raise Unreadable(f"{where}: a comment where a value was expected")
    # Trailing comment, only after a quoted or bare scalar with a space before `#`.
    if " #" in text and not text.startswith(("'", '"')):
        text = text.split(" #", 1)[0].strip()
    if text in ("true", "false"):
        return text == "true"
    if len(text) >= 2 and text[0] == text[-1] and text[0] in "'\"":
        return text[1:-1]
    if text.startswith(("[", "{", "&", "*", "!")):
        raise Unreadable(f"{where}: flow collections, anchors and tags are not read here: {text}")
    return text


def _block_body(lines: list[str], start: int, indent: int) -> tuple[list[str], int]:
    """Lines under `start` indented deeper than `indent`, and the index after them."""
    body: list[str] = []
    i = start
    while i < len(lines):
        if _is_skippable(lines[i]):
            body.append(lines[i])
            i += 1
            continue
        if _indent(lines[i]) <= indent:
            break
        body.append(lines[i])
        i += 1
    while body and _is_skippable(body[-1]):
        body.pop()
    return body, i


def _mapping(lines: list[str], where: str) -> dict[str, object]:
    """A mapping of scalars, block scalars and one-level nested mappings."""
    out: dict[str, object] = {}
    keys_indent = None
    i = 0
    while i < len(lines):
        line = lines[i]
        if _is_skippable(line):
            i += 1
            continue
        if keys_indent is None:
            keys_indent = _indent(line)
        if _indent(line) != keys_indent:
            raise Unreadable(
                f"{where}: line {i + 1} of the block is indented {_indent(line)} where "
                f"{keys_indent} was expected; this reader does not read that shape"
            )
        match = re.match(r"^\s*([A-Za-z0-9_.-]+):(.*)$", line.rstrip("\n"))
        if not match:
            raise Unreadable(f"{where}: cannot read `{line.strip()}` as a key")
        key, rest = match.group(1), match.group(2)
        rest_stripped = rest.strip()
        if rest_stripped in ("|", "|-", "|+"):
            body, i = _block_body(lines, i + 1, keys_indent)
            text = [entry for entry in body if not _is_skippable(entry)]
            base = min((_indent(entry) for entry in text), default=0)
            out[key] = "".join(
                (entry[base:] if not _is_skippable(entry) else "\n") for entry in body
            )
            continue
        if rest_stripped.startswith(">"):
            raise Unreadable(
                f"{where}: `{key}: >` folds newlines away and this reader will not "
                "guess what the shell would then see"
            )
        if rest_stripped == "":
            body, i = _block_body(lines, i + 1, keys_indent)
            if any(entry.lstrip().startswith("- ") for entry in body if not _is_skippable(entry)):
                raise Unreadable(f"{where}: `{key}:` holds a list where a mapping was expected")
            out[key] = _mapping(body, f"{where} > {key}")
            continue
        out[key] = _scalar(rest, f"{where} > {key}")
        i += 1
    return out


def _list_items(lines: list[str], where: str) -> list[list[str]]:
    """Split a block sequence into one line-list per `- ` item, dash rewritten to space."""
    items: list[list[str]] = []
    dash_indent = None
    current: list[str] | None = None
    for line in lines:
        if _is_skippable(line):
            if current is not None:
                current.append(line)
            continue
        stripped = line.lstrip()
        if stripped.startswith("- ") and (dash_indent is None or _indent(line) == dash_indent):
            if dash_indent is None:
                dash_indent = _indent(line)
            current = []
            items.append(current)
            current.append(line.replace("- ", "  ", 1))
            continue
        if current is None:
            raise Unreadable(f"{where}: content before the first `- ` item: {line.strip()!r}")
        if _indent(line) <= (dash_indent or 0):
            raise Unreadable(f"{where}: `{line.strip()}` is outside the item it follows")
        current.append(line)
    return items


def job_body(lines: list[str], job_id: str) -> list[str]:
    starts = [
        i
        for i, line in enumerate(lines)
        if re.match(rf"^  {re.escape(job_id)}:\s*(#.*)?$", line.rstrip("\n"))
    ]
    if len(starts) != 1:
        raise Unreadable(
            f"{WORKFLOW_REL}: expected exactly one `{job_id}:` job at indent 2, found {len(starts)}"
        )
    body, _ = _block_body(lines, starts[0] + 1, 2)
    return body


def job_steps(body: list[str], job_id: str) -> list[dict[str, object]]:
    where = f"{WORKFLOW_REL} > {job_id}"
    top = _top_keys(body, where)
    if "steps" not in top:
        raise Unreadable(f"{where}: no `steps:` block")
    return [_mapping(item, f"{where} > step") for item in _list_items(top["steps"], where)]


def _top_keys(body: list[str], where: str) -> dict[str, list[str]]:
    """The job's own keys, each mapped to its raw body lines (empty for scalars)."""
    out: dict[str, list[str]] = {}
    i = 0
    base = None
    while i < len(body):
        line = body[i]
        if _is_skippable(line):
            i += 1
            continue
        if base is None:
            base = _indent(line)
        if _indent(line) != base:
            i += 1
            continue
        match = re.match(r"^\s*([A-Za-z0-9_.-]+):(.*)$", line.rstrip("\n"))
        if not match:
            raise Unreadable(f"{where}: cannot read `{line.strip()}` as a key")
        key, rest = match.group(1), match.group(2)
        if rest.strip() == "":
            nested, i = _block_body(body, i + 1, base)
            out[key] = nested
        else:
            out[key] = []
            out[f"={key}"] = [rest]  # scalar text, kept under a name a YAML key cannot take
            i += 1
    return out


def job_scalar(body: list[str], job_id: str, key: str) -> str | bool | None:
    top = _top_keys(body, f"{WORKFLOW_REL} > {job_id}")
    if f"={key}" not in top:
        return None
    return _scalar(top[f"={key}"][0], f"{WORKFLOW_REL} > {job_id} > {key}")


def matrix_legs(body: list[str], job_id: str) -> list[dict[str, object]]:
    where = f"{WORKFLOW_REL} > {job_id} > strategy.matrix.include"
    top = _top_keys(body, f"{WORKFLOW_REL} > {job_id}")
    if "strategy" not in top:
        raise Unreadable(f"{WORKFLOW_REL} > {job_id}: no `strategy:` block, so there are no legs to read")
    strategy = _top_keys(top["strategy"], where)
    if "matrix" not in strategy:
        raise Unreadable(f"{where}: no `matrix:`")
    matrix = _top_keys(strategy["matrix"], where)
    if "include" not in matrix:
        raise Unreadable(f"{where}: this reader reads only the `include:` form")
    if len(matrix) != 1:
        raise Unreadable(
            f"{where}: the matrix has keys other than `include:` "
            f"({sorted(k for k in matrix if not k.startswith('='))}), and a leg set this reader "
            "cannot enumerate is not one it may report on"
        )
    legs = [_mapping(item, where) for item in _list_items(matrix["include"], where)]
    if not legs:
        raise Unreadable(f"{where}: no legs")
    for leg in legs:
        if "target" not in leg:
            raise Unreadable(f"{where}: a leg with no `target:`: {leg}")
    return legs


# ── resolving what the runner would have substituted ────────────────────────

EXPRESSION = re.compile(r"\$\{\{([^}]*)\}\}")


def resolve(text: str, leg: dict[str, object], where: str, temp: str) -> str:
    """Substitute the `${{ }}` expressions this reader knows; refuse the rest."""

    def one(match: re.Match[str]) -> str:
        body = match.group(1).strip()
        if body.startswith("matrix."):
            key = body[len("matrix.") :]
            if key not in leg:
                raise Unreadable(f"{where}: `${{{{ {body} }}}}` names no key in the matrix leg {leg}")
            value = leg[key]
            return "true" if value is True else "false" if value is False else str(value)
        if body in ("github.ref_name", "env.GITHUB_REF_NAME"):
            return FIXTURE_REF
        if body in ("runner.temp", "env.RUNNER_TEMP"):
            return temp
        raise Unreadable(
            f"{where}: `${{{{ {body} }}}}` is an expression this reader cannot resolve, so what "
            "the step would actually run is unknown to it"
        )

    return EXPRESSION.sub(one, text)


# ── driving the steps ───────────────────────────────────────────────────────


def _step_name(step: dict[str, object], index: int) -> str:
    return str(step.get("name") or step.get("uses") or f"step {index + 1}")


def _shell_for(step: dict[str, object], where: str) -> list[str]:
    shell = step.get("shell")
    if shell is not None and not isinstance(shell, str):
        raise Unreadable(f"{where}: `shell:` is not a string")
    if shell not in SHELLS:
        raise Unreadable(
            f"{where}: `shell: {shell}` — this reader knows the default (`bash -e`) and "
            "`bash` (`-eo pipefail`); which one runs decides whether a piped check can fail "
            "a job, so it is not guessed"
        )
    return SHELLS[shell]


def _artifact_names(target: str) -> list[str]:
    stem = f"brightfield-{FIXTURE_REF}-{target}"
    return [f"{stem}{suffix}" for suffix in ARTIFACT_SUFFIXES]


class Sandbox:
    """A temporary checkout-shaped tree: `dist/` with fixture artefacts, a stub `scripts/`."""

    def __init__(self, root: Path, target: str) -> None:
        self.root = root
        self.target = target
        self.record = root / "record.txt"
        self.summary = root / "step-summary.md"
        self.temp = str(root / "runner-temp")
        (root / "dist").mkdir(parents=True, exist_ok=True)
        (root / "scripts").mkdir(parents=True, exist_ok=True)
        Path(self.temp).mkdir(parents=True, exist_ok=True)
        recorder = root / READBACK_REL
        recorder.write_text(RECORDER, encoding="utf-8")
        recorder.chmod(0o755)
        self.populate()

    def populate(self) -> None:
        for name in _artifact_names(self.target):
            path = self.root / "dist" / name
            path.write_bytes(f"fixture bytes for {name}\n".encode())

    def clear_dist(self) -> None:
        shutil.rmtree(self.root / "dist")
        (self.root / "dist").mkdir()

    def run_step(
        self,
        step: dict[str, object],
        leg: dict[str, object],
        where: str,
        fail_for: str | None = None,
    ) -> tuple[int, str, list[str], str]:
        """Execute a step's `run:` block. Returns (code, output, recorded argv, summary)."""
        script = step.get("run")
        if not isinstance(script, str):
            raise Unreadable(f"{where}: `run:` is not a block this reader can execute")
        self.record.write_text("", encoding="utf-8")
        self.summary.write_text("", encoding="utf-8")
        env = dict(os.environ)
        env.update(
            {
                "GITHUB_REF_NAME": FIXTURE_REF,
                "RUNNER_TEMP": self.temp,
                "GITHUB_STEP_SUMMARY": str(self.summary),
                "BF_RECORD": str(self.record),
                # A DELIBERATELY SHORT PATH. These are real `run:` blocks from a
                # real workflow, executed to find out what they do, and the build
                # job's steps include `rustup target add` — which on a developer's
                # machine and on a runner would install a toolchain as a side
                # effect of a check. Everything a step here is allowed to reach is
                # either in the sandbox (`scripts/`, `dist/`) or is a base system
                # tool; anything else is `command not found`, which is a step this
                # gate then draws no conclusion from.
                "PATH": "/usr/bin:/bin",
            }
        )
        env.pop("BF_FAIL_FOR", None)
        if fail_for is not None:
            env["BF_FAIL_FOR"] = fail_for
        declared = step.get("env")
        if declared is not None:
            if not isinstance(declared, dict):
                raise Unreadable(f"{where}: `env:` is not a mapping")
            for key, value in declared.items():
                env[key] = resolve(str(value), leg, f"{where} > env.{key}", self.temp)
        body = resolve(script, leg, where, self.temp)
        path = self.root / "step.sh"
        path.write_text(body, encoding="utf-8")
        proc = subprocess.run(
            [*_shell_for(step, where), str(path)],
            cwd=self.root,
            env=env,
            capture_output=True,
            text=True,
            check=False,
        )
        recorded = [
            line for line in self.record.read_text(encoding="utf-8").splitlines() if line.strip()
        ]
        return proc.returncode, proc.stdout + proc.stderr, recorded, self.summary.read_text(
            encoding="utf-8"
        )


# ── asking the read-back which artefacts it can open ────────────────────────

UNKNOWN_SHAPE = "not an artifact this script knows"


def loadable(readback: Path, artifact: Path, target: str) -> bool:
    """True when the read-back would try to open this file rather than reject its shape.

    Asked of the script rather than written down here: the set of artefact
    kinds a release must load-verify is whatever that script can open, so a
    kind it learns to open is one this gate starts requiring.

    The answer is cached across a run keyed by the artefact's NAME and target,
    not its path. Every fixture of a given name holds the same bytes, and the
    self-test drives this check twenty-odd times over workflow mutations that
    change nothing about either the read-back script or those bytes — on macOS
    each uncached `.dmg` answer costs an `hdiutil attach` that has to fail.
    """
    key = (str(readback), artifact.name, target)
    if key not in _LOADABLE:
        proc = subprocess.run(
            ["bash", str(readback), str(artifact), target],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
        _LOADABLE[key] = UNKNOWN_SHAPE not in (proc.stdout + proc.stderr)
    return _LOADABLE[key]


_LOADABLE: dict[tuple[str, str, str], bool] = {}


# ── the contract ────────────────────────────────────────────────────────────


def check(workflow: Path, readback: Path, report: list[str] | None = None) -> list[str]:
    """Return failure strings (empty = the release path holds). Raises Unreadable for exit 2.

    `report` collects what was actually read and driven. A gate that prints only
    a verdict cannot be told apart from one that checked nothing, and this one
    decides which artefacts it requires by asking another script — so the set it
    settled on belongs in the log beside the verdict.
    """
    if report is None:
        report = []
    if not workflow.is_file():
        raise Unreadable(f"no workflow at {workflow}")
    if not readback.is_file():
        raise Unreadable(f"no read-back script at {readback}")
    lines = workflow.read_text(encoding="utf-8").splitlines(keepends=True)

    build = job_body(lines, BUILD_JOB)
    release = job_body(lines, RELEASE_JOB)
    legs = matrix_legs(build, BUILD_JOB)
    build_steps = job_steps(build, BUILD_JOB)
    release_steps = job_steps(release, RELEASE_JOB)
    failures: list[str] = []

    if job_scalar(build, BUILD_JOB, "continue-on-error") is True:
        failures.append(
            f"the `{BUILD_JOB}` job carries `continue-on-error: true`, so nothing inside it — "
            "including the read-back — can fail the release"
        )

    upload_globs, upload_missing = _upload_declaration(release_steps, build_steps)
    release_globs, unmatched_setting = _release_declaration(release_steps)

    if upload_missing != "error":
        failures.append(
            f"the build job's upload step has `if-no-files-found: {upload_missing}`, not `error`, "
            "so a leg that packaged nothing uploads nothing and the release is short an artefact"
        )
    if unmatched_setting is not True:
        failures.append(
            f"the release step's `fail_on_unmatched_files` is `{unmatched_setting}`, not `true`, "
            "so a glob that stops matching is a warning at release time rather than a failure"
        )

    with tempfile.TemporaryDirectory() as tmp:
        for leg in legs:
            target = str(leg["target"])
            leg_root = Path(tmp) / target
            leg_root.mkdir(parents=True)
            box = Sandbox(leg_root, target)
            failures += _leg(box, leg, target, build_steps, release_steps, readback,
                             upload_globs, release_globs, report)
    return failures


def _upload_declaration(
    release_steps: list[dict[str, object]], build_steps: list[dict[str, object]]
) -> tuple[list[str], object]:
    hits = [
        step
        for step in build_steps
        if isinstance(step.get("uses"), str) and UPLOAD_ACTION in str(step["uses"])
    ]
    if len(hits) != 1:
        raise Unreadable(
            f"{WORKFLOW_REL} > {BUILD_JOB}: expected exactly one `{UPLOAD_ACTION}` step, "
            f"found {len(hits)}"
        )
    with_block = hits[0].get("with")
    if not isinstance(with_block, dict):
        raise Unreadable(f"{WORKFLOW_REL} > {BUILD_JOB}: the upload step has no `with:` mapping")
    path = with_block.get("path")
    if not isinstance(path, str):
        raise Unreadable(
            f"{WORKFLOW_REL} > {BUILD_JOB}: the upload step's `path:` is not a block scalar"
        )
    globs = [line.strip() for line in path.splitlines() if line.strip()]
    if not globs:
        raise Unreadable(f"{WORKFLOW_REL} > {BUILD_JOB}: the upload step's `path:` is empty")
    return globs, with_block.get("if-no-files-found", "<absent>")


def _release_declaration(release_steps: list[dict[str, object]]) -> tuple[list[str], object]:
    hits = [
        step
        for step in release_steps
        if isinstance(step.get("uses"), str) and RELEASE_ACTION in str(step["uses"])
    ]
    if len(hits) != 1:
        raise Unreadable(
            f"{WORKFLOW_REL} > {RELEASE_JOB}: expected exactly one `{RELEASE_ACTION}` step, "
            f"found {len(hits)}"
        )
    with_block = hits[0].get("with")
    if not isinstance(with_block, dict):
        raise Unreadable(f"{WORKFLOW_REL} > {RELEASE_JOB}: the release step has no `with:` mapping")
    files = with_block.get("files")
    if not isinstance(files, str):
        raise Unreadable(
            f"{WORKFLOW_REL} > {RELEASE_JOB}: the release step's `files:` is not a block scalar"
        )
    globs = [line.strip() for line in files.splitlines() if line.strip()]
    if not globs:
        raise Unreadable(f"{WORKFLOW_REL} > {RELEASE_JOB}: the release step's `files:` is empty")
    return globs, with_block.get("fail_on_unmatched_files", "<absent>")


def _leg(
    box: Sandbox,
    leg: dict[str, object],
    target: str,
    build_steps: list[dict[str, object]],
    release_steps: list[dict[str, object]],
    readback: Path,
    upload_globs: list[str],
    release_globs: list[str],
    report: list[str],
) -> list[str]:
    failures: list[str] = []
    fixture = _artifact_names(target)

    # ── A. the fixture, pinned against a step that would notice it shrinking ──
    sentinel: str | None = None
    for index, step in enumerate(build_steps):
        if not isinstance(step.get("run"), str):
            continue
        # A conditional step establishes nothing for a leg it does not run on.
        if "if" in step:
            continue
        where = f"{WORKFLOW_REL} > {BUILD_JOB} > {_step_name(step, index)}"
        code, _, _, _ = box.run_step(step, leg, where)
        if code != 0:
            continue
        noticed = True
        for name in fixture:
            (box.root / "dist" / name).unlink()
            gone, _, _, _ = box.run_step(step, leg, where)
            box.populate()
            if gone == 0:
                noticed = False
                break
        if noticed:
            sentinel = _step_name(step, index)
            report.append(
                f"[{target}] `{sentinel}` fails when any one of "
                f"{', '.join(fixture)} is missing"
            )
            break
    if sentinel is None:
        raise Unreadable(
            f"{WORKFLOW_REL} > {BUILD_JOB}: no step passes over the {len(fixture)} artefacts this "
            f"gate expects for {target} and fails when any one of them is missing. Either the job "
            "would not notice a missing artefact, or packaging now produces a different set and "
            "this gate is checking one it made up. Both are refusals, not verdicts."
        )

    # ── B and C. the two glob links, run against the fixture ──
    dist = box.root / "dist"
    uploaded: set[str] = set()
    for pattern in upload_globs:
        hits = {
            hit
            for hit in glob.glob(pattern, root_dir=box.root)
            if (box.root / hit).is_file()
        }
        if not hits:
            failures.append(
                f"[{target}] the build job uploads `{pattern}`, which matches nothing packaging "
                "produces — with `if-no-files-found: error` that is a failed release"
            )
        uploaded |= {Path(hit).name for hit in hits}
    for name in fixture:
        if name not in uploaded:
            failures.append(
                f"[{target}] packaging produces `{name}` and no upload glob matches it, so it "
                "never leaves the runner"
            )

    # `merge-multiple: true` puts every leg's upload flat under artifacts/.
    flat = box.root / "artifacts"
    flat.mkdir(exist_ok=True)
    for name in sorted(uploaded):
        (flat / name).write_bytes((dist / name).read_bytes())
    published: set[str] = set()
    for pattern in release_globs:
        hits = {hit for hit in glob.glob(pattern, root_dir=box.root) if (box.root / hit).is_file()}
        if not hits:
            failures.append(
                f"[{target}] the release step publishes `{pattern}`, which matches nothing the "
                "build job uploads — with `fail_on_unmatched_files: true` that is a failed release"
            )
        published |= {Path(hit).name for hit in hits}
    for name in sorted(uploaded):
        if name not in published:
            failures.append(
                f"[{target}] the build job uploads `{name}` and no glob in the release step's "
                "`files:` list matches it, so the tag would not carry it"
            )

    # ── D, E, F. the read-back, driven ──
    want = {
        name
        for name in sorted(published)
        if loadable(readback, dist / name, target)
    }
    if not want:
        raise Unreadable(
            f"[{target}] {READBACK_REL} rejects the shape of every artefact this release "
            f"publishes ({sorted(published)}), so there is nothing it could be required to read "
            "and a pass here would mean nothing"
        )

    report.append(
        f"[{target}] published: {', '.join(sorted(published))}"
    )
    report.append(
        f"[{target}] {READBACK_REL} opens {', '.join(sorted(want))} and rejects the shape of "
        f"{', '.join(sorted(published - want)) or 'nothing else'}"
    )
    seen: dict[str, list[str]] = {}
    stdout_seen: set[str] = set()
    summary_seen: set[str] = set()
    readback_steps: list[tuple[int, dict[str, object]]] = []
    for index, step in enumerate(build_steps):
        if not isinstance(step.get("run"), str):
            continue
        where = f"{WORKFLOW_REL} > {BUILD_JOB} > {_step_name(step, index)}"
        code, output, recorded, summary = box.run_step(step, leg, where)
        if not recorded:
            continue
        readback_steps.append((index, step))
        if code != 0:
            failures.append(
                f"[{target}] the step `{_step_name(step, index)}` exits {code} over artefacts that "
                f"are all present and a read-back that passes: {output.strip()[-400:]}"
            )
        for entry in recorded:
            argv = entry.split()
            seen.setdefault(argv[0], []).append(entry)
            # PER ARTEFACT, NOT PER STEP. Redirecting one call to /dev/null and
            # leaving the other alone is a step that still prints something, and
            # an "anything reached the log" test reads that as fine.
            if f"RECORDER-STDOUT {entry}" in output:
                stdout_seen.add(argv[0])
            if f"RECORDER-SUMMARY {entry}" in summary:
                summary_seen.add(argv[0])

    if not readback_steps:
        failures.append(
            f"[{target}] no step in the `{BUILD_JOB}` job runs {READBACK_REL}, so a release that "
            "packaged an artefact with no type source in it would publish green"
        )
        return failures

    for name in sorted(want):
        argv_for = seen.get(f"dist/{name}")
        if not argv_for:
            failures.append(
                f"[{target}] the release publishes `{name}` and no step hands it to "
                f"{READBACK_REL}, so nothing opens it before it ships. Read back "
                f"dist/{name} in the same step as the others."
            )
            continue
        for entry in argv_for:
            argv = entry.split()
            if len(argv) != 2 or argv[1] != target:
                failures.append(
                    f"[{target}] {READBACK_REL} is run as `{entry}`; it takes the artefact and "
                    f"the triple it was packaged for, so this leg must pass `{target}`"
                )

    for index, step in readback_steps:
        name = _step_name(step, index)
        where = f"{WORKFLOW_REL} > {BUILD_JOB} > {name}"
        if step.get("continue-on-error") is True:
            failures.append(
                f"[{target}] the step `{name}` carries `continue-on-error: true`, so it reports the "
                "artefact does not load and the release publishes it anyway"
            )
        if "if" in step:
            failures.append(
                f"[{target}] the step `{name}` carries `if: {step['if']}`, so whether the packaged "
                "artefact is opened at all depends on a condition rather than on the release "
                "happening"
            )

    for name in sorted(want):
        if f"dist/{name}" not in seen:
            continue
        for index, step in readback_steps:
            where = f"{WORKFLOW_REL} > {BUILD_JOB} > {_step_name(step, index)}"
            code, output, recorded, _ = box.run_step(step, leg, where, fail_for=f"dist/{name}")
            if not any(entry.split()[0] == f"dist/{name}" for entry in recorded):
                continue
            if code == 0:
                failures.append(
                    f"[{target}] {READBACK_REL} refuses `dist/{name}` and the step "
                    f"`{_step_name(step, index)}` still exits 0 — the failure is swallowed. Under "
                    "this step's shell a pipe or a `|| true` does exactly that."
                )

    for name in sorted(want):
        for entry in seen.get(f"dist/{name}", []):
            report.append(f"[{target}] read back as `{READBACK_REL} {entry}`")
    for name in sorted(want):
        if f"dist/{name}" not in seen:
            continue
        if f"dist/{name}" not in stdout_seen:
            failures.append(
                f"[{target}] what {READBACK_REL} prints about `{name}` does not reach the step's "
                "log, so a reader of the run cannot see what was checked"
            )
        if f"dist/{name}" not in summary_seen:
            failures.append(
                f"[{target}] what {READBACK_REL} writes to $GITHUB_STEP_SUMMARY about `{name}` "
                "does not reach the run summary, so which targets were load-verified is not "
                "where a release reader meets it"
            )
    return failures


# ══════════════════════════════════════════════════════════════════════════════
# SELF-TEST — a gate that is only known to pass is not known to detect
# ══════════════════════════════════════════════════════════════════════════════


def _run_cli(argv: list[str]) -> tuple[int, str]:
    """This file as a real subprocess, so `main`'s exit codes are what is asserted.

    The verdict cases below prove `check` returns the right failure strings.
    They say nothing about whether the PROGRAM acts on them: turning `main`'s
    `return 1` into `return 0` leaves every one of them green while the gate
    prints its complaint and exits 0 over a release that reads nothing back.
    The `Unreadable -> 2` path is the likelier edit of the two, since this
    reader refuses a wide family of legitimate YAML on purpose and softening
    that into a warning is the change somebody would make.
    """
    proc = subprocess.run(
        [sys.executable, str(Path(__file__).resolve()), *argv],
        capture_output=True,
        text=True,
        check=False,
    )
    return proc.returncode, proc.stdout + proc.stderr


def self_test() -> int:
    workflow = REPO_ROOT / WORKFLOW_REL
    readback = REPO_ROOT / READBACK_REL
    original = workflow.read_text(encoding="utf-8")

    try:
        control = check(workflow, readback)
    except Unreadable as exc:
        print(f"  CONTROL FAILED — the real workflow could not be read: {exc}")
        return 1
    if control:
        print("  CONTROL FAILED — the real release path does not satisfy the contract:")
        for failure in control:
            print(f"      {failure}")
        return 1
    print("  ok   control: the real release reads back every artefact it publishes")

    readback_call = '          scripts/check-artifact-type-source.sh "dist/${NAME}.dmg" "${{ matrix.target }}"\n'
    step_head = "      - name: The packaged artifacts carry a type source"

    # (name, text replaced in release.yml, replacement, substring the failure must name)
    cases: list[tuple[str, str, str, str]] = [
        (
            "the read-back step is marked continue-on-error",
            step_head,
            "      - continue-on-error: true\n        name: The packaged artifacts carry a type source",
            "continue-on-error",
        ),
        (
            "the .dmg argument is deleted from the read-back step",
            readback_call,
            "",
            ".dmg` and no step hands it to",
        ),
        (
            "the read-back of the disk image is made conditional on the leg being native",
            readback_call,
            '          if [ "${{ matrix.native }}" = "true" ]; then\n'
            '            scripts/check-artifact-type-source.sh "dist/${NAME}.dmg" "${{ matrix.target }}"\n'
            "          fi\n",
            ".dmg` and no step hands it to",
        ),
        (
            "the read-back of the disk image is trailed with || true",
            readback_call,
            '          scripts/check-artifact-type-source.sh "dist/${NAME}.dmg" "${{ matrix.target }}" || true\n',
            "swallowed",
        ),
        (
            "the read-back of the disk image is piped, so its exit status is the pipe's",
            readback_call,
            '          scripts/check-artifact-type-source.sh "dist/${NAME}.dmg" "${{ matrix.target }}" | sed \'s/^/   /\'\n',
            "swallowed",
        ),
        (
            "the read-back is given the wrong triple",
            readback_call,
            '          scripts/check-artifact-type-source.sh "dist/${NAME}.dmg" aarch64-apple-darwin\n',
            "must pass",
        ),
        (
            "the whole read-back is dropped from the step that carried it",
            '          scripts/check-artifact-type-source.sh "dist/${NAME}.tar.gz" "${{ matrix.target }}"\n'
            '          scripts/check-artifact-type-source.sh "dist/${NAME}.dmg" "${{ matrix.target }}"\n',
            '          echo "both artifacts are present"\n',
            "no step in the `build` job runs",
        ),
        (
            "the build job is marked continue-on-error",
            "    runs-on: macos-15\n    timeout-minutes: 90\n",
            "    runs-on: macos-15\n    continue-on-error: true\n    timeout-minutes: 90\n",
            "continue-on-error: true`, so nothing inside it",
        ),
        (
            "the read-back's output is redirected away from the log",
            readback_call,
            '          scripts/check-artifact-type-source.sh "dist/${NAME}.dmg" "${{ matrix.target }}" >/dev/null 2>&1\n',
            "does not reach the step's log",
        ),
        (
            "the run summary the check writes is discarded",
            step_head,
            "      - env:\n          GITHUB_STEP_SUMMARY: /dev/null\n"
            "        name: The packaged artifacts carry a type source",
            "not where a release reader meets it",
        ),
        (
            "the disk image is dropped from the upload globs",
            "            dist/brightfield-*.dmg\n            dist/brightfield-*.sha256\n",
            "            dist/brightfield-*.sha256\n",
            "no upload glob matches it",
        ),
        (
            "the disk image is dropped from the release step's files: list",
            "            artifacts/brightfield-*.dmg\n",
            "",
            "no glob in the release step's `files:` list matches it",
        ),
        (
            "fail_on_unmatched_files is turned off",
            "          fail_on_unmatched_files: true\n",
            "          fail_on_unmatched_files: false\n",
            "not `true`",
        ),
        (
            "if-no-files-found is softened to a warning",
            "          if-no-files-found: error\n",
            "          if-no-files-found: warn\n",
            "not `error`",
        ),
        (
            "a release glob is added that nothing the build uploads matches",
            "            artifacts/brightfield-*.dmg\n",
            "            artifacts/brightfield-*.dmg\n            artifacts/brightfield-*.msi\n",
            "matches nothing the build job uploads",
        ),
    ]

    # Ambiguity is exit 2, not a verdict.
    refusals: list[tuple[str, str, str, str]] = [
        (
            "the artefact set packaging produces is one no step would notice shrinking",
            '          test -f "dist/${NAME}.dmg"\n          test -f "dist/${NAME}.dmg.sha256"\n',
            "",
            "fails when any one of them is missing",
        ),
        (
            "the read-back step asks for a shell whose failure semantics this reader has not read",
            step_head,
            "      - shell: python\n        name: The packaged artifacts carry a type source",
            "this reader knows the default",
        ),
        (
            "the matrix gains an axis this reader cannot enumerate",
            "      matrix:\n        include:\n",
            "      matrix:\n        profile: [fast, slow]\n        include:\n",
            "keys other than `include:`",
        ),
        (
            "a second upload step appears, so which one carries the artefacts is ambiguous",
            "      - uses: actions/upload-artifact@v4\n",
            "      - uses: actions/upload-artifact@v4\n        with:\n          name: extra\n"
            "          path: dist/\n      - uses: actions/upload-artifact@v4\n",
            "expected exactly one `actions/upload-artifact` step",
        ),
        (
            "the release action is renamed, so no step publishes anything this reader can find",
            "        uses: softprops/action-gh-release@v2\n",
            "        uses: some-other-org/some-other-release@v9\n",
            "expected exactly one `softprops/action-gh-release` step",
        ),
        (
            "the read-back step's run block folds its newlines away",
            "        run: |\n          NAME=\"brightfield-${GITHUB_REF_NAME}-${{ matrix.target }}\"\n"
            '          scripts/check-artifact-type-source.sh "dist/${NAME}.tar.gz" "${{ matrix.target }}"\n',
            "        run: >\n          NAME=\"brightfield-${GITHUB_REF_NAME}-${{ matrix.target }}\"\n"
            '          scripts/check-artifact-type-source.sh "dist/${NAME}.tar.gz" "${{ matrix.target }}"\n',
            "folds newlines away",
        ),
        (
            "an expression this reader cannot resolve decides which artefact is read",
            '"dist/${NAME}.dmg" "${{ matrix.target }}"\n',
            '"dist/${NAME}${{ inputs.suffix }}" "${{ matrix.target }}"\n',
            "cannot resolve",
        ),
    ]

    failed = 0
    with tempfile.TemporaryDirectory() as tmpdir:
        mutated = Path(tmpdir) / "release.yml"

        for name, old, new, expected in cases:
            if original.count(old) != 1:
                print(f"  WRONG {name}: its anchor appears {original.count(old)} times, not once")
                failed += 1
                continue
            mutated.write_text(original.replace(old, new, 1), encoding="utf-8")
            try:
                found = check(mutated, readback)
            except Unreadable as exc:
                print(f"  WRONG {name}: refused as unreadable rather than reporting: {exc}")
                failed += 1
                continue
            text = "\n".join(found)
            if not found:
                print(f"  MISS {name}: mutation survived")
                failed += 1
            elif expected not in text:
                print(f"  WRONG {name}: caught, but not for the stated reason")
                print(f"      expected to see: {expected}")
                print(f"      got: {text}")
                failed += 1
            else:
                print(f"  ok   {name}")

        for name, old, new, expected in refusals:
            if original.count(old) != 1:
                print(f"  WRONG {name}: its anchor appears {original.count(old)} times, not once")
                failed += 1
                continue
            mutated.write_text(original.replace(old, new, 1), encoding="utf-8")
            try:
                found = check(mutated, readback)
            except Unreadable as exc:
                if expected in str(exc):
                    print(f"  ok   {name}: refused as unreadable rather than scored")
                else:
                    print(f"  WRONG {name}: refused, but not for the stated reason: {exc}")
                    failed += 1
                continue
            print(f"  MISS {name}: returned a verdict {found} instead of refusing")
            failed += 1

        failed += _exit_code_cases(original, Path(tmpdir))

    if failed:
        print(f"\nself-test FAILED: {failed} case(s) not detected correctly")
        return 1
    print(
        f"\nself-test passed: {len(cases)} release-path mutations detected, "
        f"{len(refusals)} shapes refused, exit codes pinned"
    )
    return 0


def _exit_code_cases(original: str, tmpdir: Path) -> int:
    """The same workflows through argv, `main` and `sys.exit`, asserting exact codes."""
    intact = tmpdir / "intact.yml"
    intact.write_text(original, encoding="utf-8")
    broken = tmpdir / "broken.yml"
    broken.write_text(
        original.replace(
            '          scripts/check-artifact-type-source.sh "dist/${NAME}.dmg" "${{ matrix.target }}"\n',
            "",
            1,
        ),
        encoding="utf-8",
    )
    not_a_workflow = tmpdir / "not-a-workflow.yml"
    not_a_workflow.write_text("name: something else\n", encoding="utf-8")

    cases: list[tuple[str, list[str], int]] = [
        ("the real release workflow exits 0", ["--workflow", str(intact)], 0),
        ("a release that publishes an unread artefact exits 1", ["--workflow", str(broken)], 1),
        ("a file that is not the release workflow exits 2, not 0", ["--workflow", str(not_a_workflow)], 2),
        ("a --workflow path that does not exist exits 2, not 0", ["--workflow", str(tmpdir / "absent.yml")], 2),
        (
            "a --readback path that does not exist exits 2, not 0",
            ["--workflow", str(intact), "--readback", str(tmpdir / "absent.sh")],
            2,
        ),
    ]
    failed = 0
    for name, argv, expected in cases:
        code, output = _run_cli(argv)
        if code != expected:
            print(f"  MISS {name}: exited {code}")
            tail = output.strip().splitlines()
            print(f"      last line of output: {tail[-1] if tail else '<none>'}")
            failed += 1
        else:
            print(f"  ok   {name}")
    return failed


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, add_help=True)
    parser.add_argument("--workflow", default=str(REPO_ROOT / WORKFLOW_REL))
    parser.add_argument("--readback", default=str(REPO_ROOT / READBACK_REL))
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()

    if args.self_test:
        return self_test()

    report: list[str] = []
    try:
        failures = check(Path(args.workflow), Path(args.readback), report)
    except Unreadable as exc:
        for line in report:
            print(f"   {line}")
        print(f"check-release-readback: REFUSED — {exc}", file=sys.stderr)
        return 2
    for line in report:
        print(f"   {line}")
    if failures:
        print("check-release-readback: the release path does not hold:", file=sys.stderr)
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        return 1
    print(
        "check-release-readback: every artefact this release publishes is opened by "
        f"{READBACK_REL} on the leg that built it, and a refusal stops the job."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
