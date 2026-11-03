"""Stage runner scheduling, failure aggregation, timings and memory guard checks."""

import asyncio
import json
from pathlib import Path
import sys
import unittest

# Import the module directly: the mmhmem_ci package __init__ imports dagger.
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src" / "mmhmem_ci"))

from stages import (  # noqa: E402
    ENGINE_RESERVE_GIB,
    HEAVY_STAGE_BUDGET_GIB,
    Stage,
    StageFailure,
    check_memory,
    run_stages,
    timings_line,
)

GIB_KIB = 1024 * 1024


class Recorder:
    """Builds stage coroutines that log start/end order and track overlap."""

    def __init__(self):
        self.events: list[str] = []
        self.running = 0
        self.peak = 0

    def stage(self, name, delay=0.01, error=None):
        async def run():
            self.events.append(f"start:{name}")
            self.running += 1
            self.peak = max(self.peak, self.running)
            try:
                await asyncio.sleep(delay)
                if error is not None:
                    raise error
                return f"{name} ok"
            finally:
                self.running -= 1
                self.events.append(f"end:{name}")

        return run


def run(stages, concurrency):
    return asyncio.run(run_stages(stages, concurrency))


class RunnerTests(unittest.TestCase):
    def test_concurrency_limit_holds(self):
        rec = Recorder()
        stages = [Stage(f"s{i}", rec.stage(f"s{i}")) for i in range(5)]
        results = run(stages, 2)
        self.assertEqual(rec.peak, 2)
        self.assertEqual([r.name for r in results], [f"s{i}" for i in range(5)])
        self.assertTrue(all(r.status == "passed" for r in results))
        self.assertEqual(results[0].output, "s0 ok")

    def test_concurrency_one_runs_in_list_order(self):
        rec = Recorder()
        names = ["c", "a", "d", "b"]
        stages = [Stage(n, rec.stage(n, delay=0.002 * (4 - i))) for i, n in enumerate(names)]
        run(stages, 1)
        expected = [event for n in names for event in (f"start:{n}", f"end:{n}")]
        self.assertEqual(rec.events, expected)
        self.assertEqual(rec.peak, 1)

    def test_failure_does_not_cancel_sibling(self):
        rec = Recorder()
        stages = [Stage("bad", rec.stage("bad", delay=0.005, error=RuntimeError("boom"))),
                  Stage("slow", rec.stage("slow", delay=0.05))]
        with self.assertRaises(StageFailure) as caught:
            run(stages, 2)
        results = {r.name: r for r in caught.exception.results}
        self.assertEqual(results["bad"].status, "failed")
        self.assertIn("boom", results["bad"].output)
        self.assertEqual(results["slow"].status, "passed")
        self.assertEqual(results["slow"].output, "slow ok")
        self.assertIn("end:slow", rec.events)

    def test_every_failure_is_reported(self):
        rec = Recorder()
        stages = [Stage("one", rec.stage("one", error=RuntimeError("first error"))),
                  Stage("two", rec.stage("two", error=ValueError("second error"))),
                  Stage("three", rec.stage("three"))]
        with self.assertRaises(StageFailure) as caught:
            run(stages, 3)
        text = str(caught.exception)
        for fragment in ("one", "first error", "two", "second error"):
            self.assertIn(fragment, text)
        self.assertEqual([r.status for r in caught.exception.results], ["failed", "failed", "passed"])

    def test_dependents_of_failed_stage_are_skipped(self):
        rec = Recorder()
        stages = [Stage("build", rec.stage("build", error=RuntimeError("compile failed"))),
                  Stage("install", rec.stage("install"), needs=("build",)),
                  Stage("deploy", rec.stage("deploy"), needs=("install",)),
                  Stage("fmt", rec.stage("fmt"))]
        with self.assertRaises(StageFailure) as caught:
            run(stages, 1)
        results = {r.name: r for r in caught.exception.results}
        self.assertEqual(results["build"].status, "failed")
        self.assertEqual(results["install"].status, "skipped")
        self.assertEqual(results["deploy"].status, "skipped")
        self.assertEqual(results["install"].duration_s, 0)
        self.assertEqual(results["deploy"].duration_s, 0)
        self.assertEqual(results["fmt"].status, "passed")
        self.assertNotIn("start:install", rec.events)
        self.assertNotIn("start:deploy", rec.events)
        text = str(caught.exception)
        for fragment in ("build", "compile failed", "install", "deploy", "skipped"):
            self.assertIn(fragment, text)
        self.assertNotIn("fmt", text)

    def test_dependent_waits_for_prerequisite(self):
        rec = Recorder()
        stages = [Stage("release", rec.stage("release", delay=0.03)),
                  Stage("installer", rec.stage("installer"), needs=("release",)),
                  Stage("docker", rec.stage("docker"))]
        results = run(stages, 3)
        self.assertLess(rec.events.index("end:release"), rec.events.index("start:installer"))
        self.assertLess(rec.events.index("start:docker"), rec.events.index("end:release"))
        self.assertTrue(all(r.status == "passed" for r in results))

    def test_error_output_keeps_last_40_lines(self):
        message = "\n".join(f"line {i}" for i in range(100))
        stages = [Stage("noisy", Recorder().stage("noisy", error=RuntimeError(message)))]
        with self.assertRaises(StageFailure) as caught:
            run(stages, 1)
        output = caught.exception.results[0].output
        self.assertEqual(output.splitlines(), [f"line {i}" for i in range(60, 100)])

    def test_timing_fields_are_recorded(self):
        rec = Recorder()
        # Margins absorb the ~15 ms Windows clock granularity.
        results = run([Stage("a", rec.stage("a", delay=0.05)), Stage("b", rec.stage("b"))], 1)
        self.assertGreaterEqual(results[0].duration_s, 0.03)
        self.assertGreaterEqual(results[1].started_s, results[0].started_s + results[0].duration_s - 0.001)

    def test_invalid_graphs_raise_before_any_stage(self):
        cases = {
            "duplicate": lambda rec: [Stage("a", rec.stage("a")), Stage("a", rec.stage("a"))],
            "unknown need": lambda rec: [Stage("a", rec.stage("a"), needs=("missing",))],
            "later need": lambda rec: [Stage("a", rec.stage("a"), needs=("b",)), Stage("b", rec.stage("b"))],
        }
        for label, build in cases.items():
            with self.subTest(case=label):
                rec = Recorder()
                with self.assertRaises(ValueError):
                    run(build(rec), 1)
                self.assertEqual(rec.events, [])

    def test_concurrency_below_one_raises(self):
        for concurrency in (0, -1):
            with self.subTest(concurrency=concurrency):
                rec = Recorder()
                with self.assertRaises(ValueError):
                    run([Stage("a", rec.stage("a"))], concurrency)
                self.assertEqual(rec.events, [])


class TimingsLineTests(unittest.TestCase):
    def test_line_is_prefixed_compact_json(self):
        rec = Recorder()
        stages = [Stage("ok", rec.stage("ok")),
                  Stage("bad", rec.stage("bad", error=RuntimeError("x"))),
                  Stage("after", rec.stage("after"), needs=("bad",))]
        with self.assertRaises(StageFailure) as caught:
            run(stages, 1)
        line = timings_line(caught.exception.results)
        self.assertTrue(line.startswith("== timings: "))
        self.assertNotIn("\n", line)
        payload = line[len("== timings: "):]
        self.assertNotIn(", ", payload)
        records = json.loads(payload)
        self.assertEqual([r["name"] for r in records], ["ok", "bad", "after"])
        self.assertEqual([r["status"] for r in records], ["passed", "failed", "skipped"])
        for record in records:
            self.assertEqual(set(record), {"name", "status", "started_s", "duration_s"})
            self.assertIsInstance(record["started_s"], (int, float))
            self.assertIsInstance(record["duration_s"], (int, float))


class MemoryGuardTests(unittest.TestCase):
    def test_budget_constants(self):
        self.assertEqual(HEAVY_STAGE_BUDGET_GIB, 3.0)
        self.assertEqual(ENGINE_RESERVE_GIB, 2.0)

    def test_six_gib_fits_concurrency_one(self):
        check_memory(6 * GIB_KIB, 1, 3)

    def test_six_gib_refuses_concurrency_two(self):
        with self.assertRaises(ValueError) as caught:
            check_memory(6 * GIB_KIB, 2, 3)
        message = str(caught.exception)
        self.assertIn("--concurrency", message)
        self.assertIn("8.0 GiB", message)
        self.assertIn("6.0 GiB", message)

    def test_heavy_count_caps_required_memory(self):
        check_memory(6 * GIB_KIB, 3, 1)

    def test_ci_runner_fits_concurrency_three(self):
        check_memory(16 * 1000 * 1000 * 1000 // 1024, 3, 3)


if __name__ == "__main__":
    unittest.main()
