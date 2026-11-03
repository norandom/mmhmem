"""Unit tests for the job-summary timing report (task 2.4).

Requirements 1.3, 5.1, 5.3; design "timing_summary (`ci/scripts/timing_summary.py`)",
"Error Handling" (timings line missing) and "Testing Strategy".
"""

import ast
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest

SCRIPTS_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPTS_DIR.parents[1]
sys.path.insert(0, str(SCRIPTS_DIR))

import timing_summary  # noqa: E402

BASELINE_PATH = REPO_ROOT / "ci" / "timing-baseline.json"
MAIN_PY = REPO_ROOT / "ci" / "src" / "mmhmem_ci" / "main.py"


def plan_stage_names():
    """Stage names from main.py's ALL_STAGE_PLAN literal (main.py imports dagger)."""
    tree = ast.parse(MAIN_PY.read_text(encoding="utf-8"))
    for node in tree.body:
        if isinstance(node, ast.Assign) and any(
                isinstance(t, ast.Name) and t.id == "ALL_STAGE_PLAN" for t in node.targets):
            return [entry[0] for entry in ast.literal_eval(node.value)]
    raise AssertionError("ALL_STAGE_PLAN not found in main.py")


def timings(*records):
    return "== timings: " + json.dumps(
        [{"name": n, "status": s, "started_s": st, "duration_s": d} for n, s, st, d in records],
        separators=(",", ":"))


def row_for(markdown, name):
    for line in markdown.splitlines():
        if line.startswith("| ") and line.split("|")[1].strip() == name:
            return [cell.strip() for cell in line.strip().strip("|").split("|")]
    raise AssertionError(f"no table row for {name!r} in:\n{markdown}")


class MarkerTests(unittest.TestCase):
    BASELINE = {"slowpoke": 100.0, "steady": 100.0, "fresh": None}

    def render(self, *records):
        return timing_summary.render(timing_summary.parse_timings(timings(*records)), self.BASELINE)

    def test_stage_at_1_6_times_baseline_is_marked_slow(self):
        row = row_for(self.render(("slowpoke", "passed", 0.0, 160.0)), "slowpoke")
        self.assertEqual(row[-1], "slow")

    def test_stage_at_1_4_times_baseline_is_not_marked(self):
        row = row_for(self.render(("steady", "passed", 0.0, 140.0)), "steady")
        self.assertEqual(row[-1], "")

    def test_exactly_1_5_times_baseline_is_not_marked(self):
        row = row_for(self.render(("steady", "passed", 0.0, 150.0)), "steady")
        self.assertEqual(row[-1], "")

    def test_unbaselined_stage_has_no_marker(self):
        md = self.render(("fresh", "passed", 0.0, 9999.0), ("unknown_stage", "passed", 1.0, 9999.0))
        for name in ("fresh", "unknown_stage"):
            row = row_for(md, name)
            self.assertEqual(row[4], "unbaselined")
            self.assertEqual(row[-1], "")

    def test_table_has_status_start_duration_and_baseline_columns(self):
        md = self.render(("steady", "failed", 12.5, 40.25))
        self.assertIn("| stage | status | start | duration | baseline | marker |", md)
        self.assertEqual(row_for(md, "steady"), ["steady", "failed", "12.5 s", "40.2 s", "100 s", ""])


class ParserTests(unittest.TestCase):
    def test_finds_timings_line_when_it_is_the_last_of_many_lines(self):
        output = "\n".join([f"== stage{i}: ok" for i in range(200)]
                           + ["tokei report", timings(("fmt", "passed", 0.0, 2.0))])
        records = timing_summary.parse_timings(output)
        self.assertEqual([r["name"] for r in records], ["fmt"])
        self.assertEqual(records[0]["duration_s"], 2.0)

    def test_missing_line_returns_none(self):
        self.assertIsNone(timing_summary.parse_timings("== fmt: ok\nno timings here\n"))
        self.assertIsNone(timing_summary.parse_timings(""))

    def test_malformed_json_raises_value_error(self):
        with self.assertRaises(ValueError):
            timing_summary.parse_timings("== timings: [{not json")
        with self.assertRaises(ValueError):
            timing_summary.parse_timings('== timings: [{"name":"fmt"}]')


class MainTests(unittest.TestCase):
    BASELINE = {"runner": "ubuntu-latest-4vcpu", "stages": {"fmt": 2}}

    def run_main(self, gate_output, baseline=None, use_summary_file=True):
        with tempfile.TemporaryDirectory() as tmp:
            baseline_path = Path(tmp) / "baseline.json"
            if baseline is not None:
                baseline_path.write_text(json.dumps(baseline), encoding="utf-8")
            summary_path = Path(tmp) / "summary.md"
            summary_path.write_text("existing\n", encoding="utf-8")
            env = {}
            if gate_output is not None:
                env["GATE_OUTPUT"] = gate_output
            if use_summary_file:
                env["GITHUB_STEP_SUMMARY"] = str(summary_path)
            stdout, stderr = io.StringIO(), io.StringIO()
            code = timing_summary.main([str(baseline_path)], env, stdout, stderr)
            text = summary_path.read_text(encoding="utf-8") if use_summary_file else stdout.getvalue()
            return code, text, stderr.getvalue()

    def test_missing_line_writes_message_and_exits_zero(self):
        code, text, _ = self.run_main("== fmt: ok\n", self.BASELINE)
        self.assertEqual(code, 0)
        self.assertIn("no stage timings recorded", text)

    def test_unset_gate_output_writes_message_and_exits_zero(self):
        code, text, _ = self.run_main(None, self.BASELINE)
        self.assertEqual(code, 0)
        self.assertIn("no stage timings recorded", text)

    def test_malformed_json_is_reported_and_exits_zero(self):
        code, text, _ = self.run_main("== timings: [oops", self.BASELINE)
        self.assertEqual(code, 0)
        self.assertIn("malformed", text)

    def test_summary_is_appended_not_overwritten(self):
        code, text, _ = self.run_main(timings(("fmt", "passed", 0.0, 4.0)), self.BASELINE)
        self.assertEqual(code, 0)
        self.assertTrue(text.startswith("existing\n"))
        self.assertEqual(row_for(text, "fmt")[-1], "slow")

    def test_writes_to_stdout_when_step_summary_is_unset(self):
        code, text, _ = self.run_main(timings(("fmt", "passed", 0.0, 1.0)), self.BASELINE,
                                      use_summary_file=False)
        self.assertEqual(code, 0)
        self.assertEqual(row_for(text, "fmt")[4], "2 s")

    def test_missing_baseline_file_still_renders_and_exits_zero(self):
        code, text, err = self.run_main(timings(("fmt", "passed", 0.0, 1.0)), None)
        self.assertEqual(code, 0)
        self.assertEqual(row_for(text, "fmt")[4], "unbaselined")
        self.assertIn("baseline unavailable", text)
        self.assertIn("baseline", err)

    def test_malformed_baseline_still_renders_and_exits_zero(self):
        code, text, _ = self.run_main(timings(("fmt", "passed", 0.0, 1.0)), {"stages": {"fmt": "x"}})
        self.assertEqual(code, 0)
        self.assertEqual(row_for(text, "fmt")[4], "unbaselined")
        self.assertIn("baseline unavailable", text)


class ShippedBaselineTests(unittest.TestCase):
    def test_shipped_baseline_parses_and_uses_only_plan_stage_names(self):
        data = json.loads(BASELINE_PATH.read_text(encoding="utf-8"))
        self.assertEqual(set(data), {"runner", "stages"})
        self.assertEqual(data["runner"], "ubuntu-latest-4vcpu")
        self.assertLessEqual(set(data["stages"]), set(plan_stage_names()))
        for name, seconds in data["stages"].items():
            self.assertTrue(seconds is None or (isinstance(seconds, (int, float))
                                                and not isinstance(seconds, bool) and seconds > 0),
                            f"{name}: {seconds!r}")
        timing_summary.load_baseline(BASELINE_PATH)

    def test_shipped_baseline_seeds_research_values_without_double_counting(self):
        stages = json.loads(BASELINE_PATH.read_text(encoding="utf-8"))["stages"]
        # One research.md timeline line maps to exactly one stage (see timing_summary docstring).
        self.assertEqual(stages, {
            "static_release": 413,  # musl candidate 365 s + static gates/installer/distribution ~48 s
            "docker_build": 162,
            "clippy_otel": None,  # no standalone history
            # loc_report left the gate (ci-dependency-cache 4.5).
            "stage_runner_tests": None,  # new stage
            "distribution": None,  # inside the ~48 s line already counted in static_release
            "fmt": 2,
            "compose_config": 10,  # compose config + deploy artifacts ~10 s
            "static_installer": None,  # inside the ~48 s line
            "deploy_artifacts": None,  # inside the ~10 s line already counted in compose_config
        })

    def test_default_baseline_path_is_the_shipped_file(self):
        self.assertEqual(Path(timing_summary.DEFAULT_BASELINE).resolve(), BASELINE_PATH.resolve())


def cache_line(stats):
    return "== cache: " + json.dumps(stats, separators=(",", ":"), sort_keys=True)


class CacheTableTests(unittest.TestCase):
    """ci-dependency-cache task 2.3 (Requirements 5.1, 5.2)."""

    STATS = {
        "static_release": {"enabled": True, "hits": 361, "misses": 14, "rust_hits": 300,
                           "rust_misses": 12, "c_hits": 61, "c_misses": 2, "non_cacheable": 7,
                           "writes": 14, "write_errors": 3, "read_errors": 0},
        "clippy_otel": {"enabled": True, "hits": 0, "misses": 0, "reason": "statistics unavailable"},
        "docker_build": {"enabled": False, "reason": "sccache server did not start; built without the cache"},
    }

    def summary(self, output):
        markdown, _ = timing_summary.build_summary(output, BASELINE_PATH)
        return markdown

    def test_prefix_matches_the_pipeline(self):
        tree = ast.parse(MAIN_PY.read_text(encoding="utf-8"))
        value = next(ast.literal_eval(n.value) for n in tree.body if isinstance(n, ast.Assign)
                     and any(isinstance(t, ast.Name) and t.id == "CACHE_PREFIX" for t in n.targets))
        self.assertEqual(timing_summary.CACHE_PREFIX, value)

    def test_enabled_and_disabled_stages_are_tabulated(self):
        output = "\n".join([cache_line(self.STATS), timings(("static_release", "passed", 0, 300))])
        markdown = self.summary(output)
        self.assertIn(timing_summary.CACHE_HEADING, markdown)
        cache = markdown[markdown.index(timing_summary.CACHE_HEADING):]
        self.assertEqual(row_for(cache, "static_release"),
                         ["static_release", "on", "361", "14", "61", "2", "14", "3", ""])
        self.assertEqual(row_for(cache, "clippy_otel")[1:4], ["on", "0", "0"])
        self.assertEqual(row_for(cache, "clippy_otel")[-1], "statistics unavailable")
        disabled = row_for(cache, "docker_build")
        self.assertEqual(disabled[1], "off")
        self.assertIn("did not start", disabled[-1])
        self.assertLess(markdown.index(timing_summary.HEADING), markdown.index(timing_summary.CACHE_HEADING))

    def test_missing_cache_line_is_reported(self):
        markdown = self.summary(timings(("fmt", "passed", 0, 1)))
        self.assertIn(timing_summary.NO_CACHE, markdown)

    def test_malformed_cache_line_is_reported_without_failing(self):
        markdown = self.summary("== cache: {not json\n" + timings(("fmt", "passed", 0, 1)))
        self.assertIn("cache line malformed", markdown)
        self.assertIn("| fmt |", markdown, "the timing table still renders")

    def test_cache_section_renders_even_without_timings(self):
        markdown = self.summary(cache_line(self.STATS))
        self.assertIn(timing_summary.NO_TIMINGS, markdown)
        self.assertIn("| static_release | on |", markdown)


if __name__ == "__main__":
    unittest.main()
