"""Write the per-stage timing table of the Dagger gate to the job summary.

Design "timing_summary (`ci/scripts/timing_summary.py`)"; Requirements 1.3, 5.1, 5.3.

Usage (GitHub Actions step on the runner host, python3 standard library only):

    GATE_OUTPUT="<gate step stdout>" python3 ci/scripts/timing_summary.py [BASELINE_JSON]

- GATE_OUTPUT: the gate's standard output. The last line starting with
  "== timings: " carries compact JSON from `timings_line` in
  ci/src/mmhmem_ci/stages.py: a list of {name, status, started_s, duration_s}.
- BASELINE_JSON: defaults to ci/timing-baseline.json next to this script's
  parent directory. Schema: {"runner": "ubuntu-latest-4vcpu",
  "stages": {"<name>": <seconds> | null}}; null or absent means unbaselined.
- The markdown is appended to $GITHUB_STEP_SUMMARY, or written to stdout
  when that variable is unset (local use).

A stage whose duration exceeds 1.5 x its baseline is marked `slow`. The
script never changes the job verdict: it always exits 0. A missing timings
line writes "no stage timings recorded"; a malformed one is reported in the
summary. A missing or invalid baseline file is reported in the summary and on
stderr, and every stage is shown as unbaselined: failing this `if: always()`
step would turn a reporting problem into a red job, and the shipped baseline
is already guarded by test_timing_summary.py.

Baseline seed (research.md "Stage timeline", run 41626826480). Each timeline
line maps to exactly one stage so that no measured second is counted twice:
- static_release 413 = musl candidate 365 s + the ~48 s "static gates,
  installer candidate, distribution" line, which the old pipeline did not
  split; distribution and static_installer therefore stay unbaselined.
- docker_build 162, loc_report 64 (tokei source install), fmt 2.
- compose_config 10 = the ~10 s "compose config + deploy artifacts" line;
  deploy_artifacts therefore stays unbaselined.
- clippy_otel (no standalone cold history) and stage_runner_tests (new) stay
  unbaselined until task 4.2 records their first hosted values.
"""

import json
import os
from pathlib import Path
import sys

TIMINGS_PREFIX = "== timings: "  # stages.TIMINGS_PREFIX
GATE_OUTPUT_ENV = "GATE_OUTPUT"
SUMMARY_ENV = "GITHUB_STEP_SUMMARY"
DEFAULT_BASELINE = Path(__file__).resolve().parents[1] / "timing-baseline.json"
SLOW_FACTOR = 1.5
NO_TIMINGS = "no stage timings recorded"
HEADING = "### Dagger gate stage timings"
REQUIRED_FIELDS = ("name", "status", "started_s", "duration_s")
CACHE_PREFIX = "== cache: "  # main.CACHE_PREFIX
CACHE_HEADING = "### Compile cache (sccache)"
NO_CACHE = "no compile-cache line recorded"


def _is_number(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool)


def parse_timings(output):
    """Return the records of the last timings line, None when there is none.

    Raise ValueError when the line is not a JSON list of complete records.
    """
    line = None
    for candidate in output.splitlines():
        candidate = candidate.strip()
        if candidate.startswith(TIMINGS_PREFIX):
            line = candidate
    if line is None:
        return None
    try:
        records = json.loads(line[len(TIMINGS_PREFIX):])
    except json.JSONDecodeError as error:
        raise ValueError(f"invalid JSON: {error}") from None
    if not isinstance(records, list):
        raise ValueError("expected a JSON list of stage records")
    for record in records:
        if not isinstance(record, dict) or any(field not in record for field in REQUIRED_FIELDS):
            raise ValueError(f"stage record lacks {', '.join(REQUIRED_FIELDS)}: {record!r}")
        if not isinstance(record["name"], str) or not all(
                _is_number(record[f]) for f in ("started_s", "duration_s")):
            raise ValueError(f"stage record has wrong field types: {record!r}")
    return records


def load_baseline(path):
    """Return {stage: seconds or None}; raise ValueError on a missing or invalid file."""
    try:
        data = json.loads(Path(path).read_text(encoding="utf-8"))
    except OSError as error:
        raise ValueError(f"cannot read {path}: {error.strerror or error}") from None
    except json.JSONDecodeError as error:
        raise ValueError(f"invalid JSON in {path}: {error}") from None
    stages = data.get("stages") if isinstance(data, dict) else None
    if not isinstance(stages, dict):
        raise ValueError(f"{path} has no \"stages\" object")
    for name, seconds in stages.items():
        if seconds is not None and not (_is_number(seconds) and seconds > 0):
            raise ValueError(f"{path}: baseline for {name!r} must be positive seconds or null")
    return dict(stages)


def _cell(value):
    return str(value).replace("|", "\\|").replace("\n", " ")


def render(records, baseline):
    """Return the markdown table for parsed records against {stage: seconds or None}."""
    lines = ["| stage | status | start | duration | baseline | marker |",
             "|---|---|---|---|---|---|"]
    for record in records:
        seconds = baseline.get(record["name"])
        duration = record["duration_s"]
        if seconds is None:
            baseline_cell, marker = "unbaselined", ""
        else:
            baseline_cell = f"{seconds:g} s"
            marker = "slow" if duration > SLOW_FACTOR * seconds else ""
        lines.append(f"| {_cell(record['name'])} | {_cell(record['status'])} | "
                     f"{record['started_s']:.1f} s | {duration:.1f} s | {baseline_cell} | {marker} |")
    return "\n".join(lines)


def parse_cache(output):
    """Return {stage: stats record} from the last cache line, None when there
    is none; raise ValueError when it is not a JSON object of records."""
    line = None
    for candidate in output.splitlines():
        candidate = candidate.strip()
        if candidate.startswith(CACHE_PREFIX):
            line = candidate
    if line is None:
        return None
    try:
        stats = json.loads(line[len(CACHE_PREFIX):])
    except json.JSONDecodeError as error:
        raise ValueError(f"invalid JSON: {error}") from None
    if not isinstance(stats, dict) or not all(isinstance(v, dict) for v in stats.values()):
        raise ValueError("expected a JSON object of per-stage records")
    return stats


def _count(record, key):
    value = record.get(key, 0)
    return value if isinstance(value, int) and not isinstance(value, bool) else 0


def render_cache(stats):
    """Markdown table of per-stage compile-cache records (Requirements 5.1, 5.2)."""
    lines = ["| stage | cache | hits | misses | C hits | C misses | writes | write errors | note |",
             "|---|---|---|---|---|---|---|---|---|"]
    for name, record in stats.items():
        state = "on" if record.get("enabled") is True else "off"
        counts = " | ".join(str(_count(record, k)) for k in
                            ("hits", "misses", "c_hits", "c_misses", "writes", "write_errors"))
        lines.append(f"| {_cell(name)} | {state} | {counts} | {_cell(record.get('reason', ''))} |")
    return "\n".join(lines)


def _timing_section(output, baseline_path):
    parts, warnings = [HEADING, ""], []
    try:
        records = parse_timings(output)
    except ValueError as error:
        parts.append(f"timings line malformed, no table rendered: {_cell(error)}")
        return parts, [f"timing_summary: malformed timings line: {error}"]
    if records is None:
        parts.append(NO_TIMINGS)
        return parts, warnings
    try:
        baseline = load_baseline(baseline_path)
    except ValueError as error:
        baseline = {}
        parts += [f"baseline unavailable, every stage is unbaselined: {_cell(error)}", ""]
        warnings.append(f"timing_summary: baseline unavailable: {error}")
    parts.append(render(records, baseline))
    parts.append("")
    parts.append(f"`slow` marks a stage over {SLOW_FACTOR:g} x its baseline; it never fails the job.")
    return parts, warnings


def _cache_section(output):
    parts = ["", CACHE_HEADING, ""]
    try:
        stats = parse_cache(output)
    except ValueError as error:
        return parts + [f"cache line malformed, no table rendered: {_cell(error)}"], [
            f"timing_summary: malformed cache line: {error}"]
    if stats is None:
        return parts + [NO_CACHE], []
    return parts + [render_cache(stats)], []


def build_summary(output, baseline_path):
    """Return (markdown, warnings) for the gate output and baseline file."""
    timing, warnings = _timing_section(output, baseline_path)
    cache, cache_warnings = _cache_section(output)
    return "\n".join(timing + cache) + "\n", warnings + cache_warnings


def main(argv=None, environ=None, stdout=None, stderr=None):
    """Write the summary and return 0; the job verdict is never changed."""
    argv = sys.argv[1:] if argv is None else argv
    environ = os.environ if environ is None else environ
    stdout = sys.stdout if stdout is None else stdout
    stderr = sys.stderr if stderr is None else stderr
    baseline_path = Path(argv[0]) if argv else DEFAULT_BASELINE
    markdown, warnings = build_summary(environ.get(GATE_OUTPUT_ENV, ""), baseline_path)
    for warning in warnings:
        print(warning, file=stderr)
    summary_file = environ.get(SUMMARY_ENV)
    if summary_file:
        try:
            with open(summary_file, "a", encoding="utf-8") as handle:
                handle.write(markdown)
            return 0
        except OSError as error:
            print(f"timing_summary: cannot append to {summary_file}: {error}", file=stderr)
    stdout.write(markdown)
    return 0


if __name__ == "__main__":
    sys.exit(main())
