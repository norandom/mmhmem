"""Unit tests for the in-container sccache wrapper (ci-dependency-cache task 1.2).

Requirements 1.2, 1.3, 5.1, 5.2. The stats sample follows real sccache
v0.18.0 `--show-stats --stats-format json` output.
"""

import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))

from sccache_stats import is_sccache, main, parse_stats, run_under_sccache  # noqa: E402

SAMPLE = {
    "stats": {
        "compile_requests": 410,
        "requests_executed": 380,
        "cache_hits": {"counts": {"Rust": 300, "C/C++": 61}, "adv_counts": {"rust": 300, "c": 61}},
        "cache_misses": {"counts": {"Rust": 12, "C/C++": 2}, "adv_counts": {"rust": 12, "c": 2}},
        "cache_errors": {"counts": {}, "adv_counts": {}},
        "non_cacheable_compilations": 7,
        "cache_writes": 14,
        "cache_write_errors": 3,
        "cache_read_errors": 1,
    },
    "cache_location": "GHA, name: mmhmem-1",
    "version": "0.18.0",
}
SECRET = "ghs_SECRET_TOKEN_VALUE"


class FakeRunner:
    """Answers sccache calls from a script; records every call."""

    def __init__(self, start_rc=0, build_rc=0, stats=SAMPLE):
        self.start_rc, self.build_rc, self.stats = start_rc, build_rc, stats
        self.calls = []

    def __call__(self, args, env):
        self.calls.append((list(args), dict(env)))
        if args[1:] == ["--start-server"]:
            return self.start_rc, f"failed to start; token={SECRET}" if self.start_rc else ""
        if args[1:3] == ["--show-stats", "--stats-format"]:
            return 0, json.dumps(self.stats)
        return self.build_rc, ""


class ParseTests(unittest.TestCase):
    def test_counts_are_split_by_language(self):
        record = parse_stats(SAMPLE)
        self.assertEqual(record["enabled"], True)
        self.assertEqual((record["hits"], record["misses"]), (361, 14))
        self.assertEqual((record["rust_hits"], record["c_hits"]), (300, 61))
        self.assertEqual((record["rust_misses"], record["c_misses"]), (12, 2))
        self.assertEqual(record["non_cacheable"], 7)
        self.assertEqual((record["writes"], record["write_errors"], record["read_errors"]), (14, 3, 1))
        self.assertEqual(record["location"], "GHA, name: mmhmem-1")

    def test_empty_or_odd_stats_count_as_zero(self):
        record = parse_stats({"stats": {"cache_hits": {"counts": {}}}})
        self.assertEqual((record["hits"], record["misses"], record["c_hits"]), (0, 0, 0))
        self.assertEqual(parse_stats({})["hits"], 0)


class RunTests(unittest.TestCase):
    env = {"RUSTC_WRAPPER": "/usr/local/bin/sccache", "ACTIONS_RUNTIME_TOKEN": SECRET}

    def test_not_configured_runs_plainly(self):
        runner = FakeRunner()
        rc, record = run_under_sccache(["cargo", "build"], {"PATH": "/bin"}, runner)
        self.assertEqual(rc, 0)
        self.assertEqual(record, {"enabled": False, "reason": "not configured"})
        self.assertEqual([c[0] for c in runner.calls], [["cargo", "build"]])

    def test_enabled_build_reports_stats_and_keeps_exit_code(self):
        runner = FakeRunner(build_rc=101)
        rc, record = run_under_sccache(["cargo", "build"], self.env, runner)
        self.assertEqual(rc, 101, "the build's exit code is preserved")
        self.assertEqual(record["hits"], 361)
        self.assertEqual(runner.calls[1][1]["RUSTC_WRAPPER"], "/usr/local/bin/sccache")

    def test_failed_server_start_builds_uncached_without_leaking(self):
        runner = FakeRunner(start_rc=2)
        rc, record = run_under_sccache(["cargo", "build"], self.env, runner)
        self.assertEqual(rc, 0)
        self.assertFalse(record["enabled"])
        self.assertIn("did not start", record["reason"])
        self.assertNotIn(SECRET, json.dumps(record))
        build_env = runner.calls[1][1]
        self.assertNotIn("RUSTC_WRAPPER", build_env, "the build runs without the wrapper")


class BinaryNameTests(unittest.TestCase):
    def test_sccache_is_recognised_with_or_without_exe(self):
        for path in ("/usr/local/bin/sccache", "C:\\tools\\sccache.exe", "SCCACHE.EXE"):
            self.assertTrue(is_sccache(path), path)
        for path in ("", "/usr/bin/cargo", "/opt/not-sccache"):
            self.assertFalse(is_sccache(path), path)


class MainTests(unittest.TestCase):
    def test_main_writes_the_record_and_returns_the_exit_code(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "stats.json"
            runner = FakeRunner(build_rc=3)
            rc = main(["--stats", str(out), "--", "cargo", "test"], RunTests.env, runner)
            self.assertEqual(rc, 3)
            record = json.loads(out.read_text())
            self.assertEqual(record["hits"], 361)
            self.assertNotIn(SECRET, out.read_text())

    def test_main_requires_a_command(self):
        with tempfile.TemporaryDirectory() as tmp, self.assertRaises(SystemExit):
            main(["--stats", str(Path(tmp) / "s.json"), "--"], {}, FakeRunner())


if __name__ == "__main__":
    unittest.main()
