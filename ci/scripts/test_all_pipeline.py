"""Contract tests for the `all` pipeline on the stage runner (tasks 2.2, 2.3).

Requirements 1.1, 2.1-2.4, 3.1, 3.4, 4.4, 5.1; design "`all` stage list",
"Memory guard", Key Decisions 2, 3 and 4.

main.py imports dagger and needs Python 3.14, so it cannot be imported here.
The stage plan is a module-level literal (ALL_STAGE_PLAN) read with `ast`;
the pure helpers (all_summary, parse_mem_total_kib, validate_concurrency)
are extracted and executed in isolation; the runner itself is imported from
stages.py and driven with stub async stages built from the plan.
"""

import ast
import asyncio
import json
from pathlib import Path
import sys
import unittest

MODULE_DIR = Path(__file__).resolve().parents[1] / "src" / "mmhmem_ci"
# Import the module directly: the mmhmem_ci package __init__ imports dagger.
sys.path.insert(0, str(MODULE_DIR))

from stages import TIMINGS_PREFIX, Stage, StageFailure, run_stages, timings_line  # noqa: E402

SOURCE = (MODULE_DIR / "main.py").read_text(encoding="utf-8")
TREE = ast.parse(SOURCE)

# Design "`all` stage list" table: (name, heavy, needs), in order, except
# that loc_report sits right after the three heavy stages (task 2.3, Decision 4
# fallback): bookworm has no tokei package, so its source build takes the
# first slot a heavy stage frees and overlaps the static chain.
EXPECTED_PLAN = (
    ("static_release", True, ()),
    ("docker_build", True, ()),
    ("clippy_otel", True, ()),
    ("loc_report", False, ()),
    ("stage_runner_tests", False, ()),
    ("distribution", False, ()),
    ("fmt", False, ()),
    ("compose_config", False, ()),
    ("static_installer", False, ("static_release",)),
    ("deploy_artifacts", False, ("static_release",)),
)
TOKEI_SOURCE_INSTALL = "tokei --version >/dev/null 2>&1 || cargo install tokei --locked"
# Relative stage durations in seconds from research.md (run 41626826480).
# clippy_otel is taken cold (no default-feature pass warms it any more).
RESEARCH_DURATIONS_S = {
    "static_release": 413,  # musl candidate 365 s + static gates ~48 s
    "docker_build": 162,
    "clippy_otel": 60,
    "loc_report": 64,  # tokei cargo install from source
    "stage_runner_tests": 10,
    "distribution": 48,
    "fmt": 2,
    "compose_config": 5,
    "static_installer": 10,
    "deploy_artifacts": 5,
}
TIME_SCALE = 1 / 400  # 413 s of static_release becomes about one second
STAGE_NAMES = [entry[0] for entry in EXPECTED_PLAN]
OTEL_CLIPPY = ["cargo", "clippy", "--all-targets", "--features", "otel", "--", "-D", "warnings"]
DEFAULT_CLIPPY = ["cargo", "clippy", "--all-targets", "--", "-D", "warnings"]


def module_assign(name: str) -> ast.expr:
    for node in TREE.body:
        if isinstance(node, ast.Assign):
            targets = node.targets
        elif isinstance(node, ast.AnnAssign):
            targets = [node.target]
        else:
            continue
        if any(isinstance(t, ast.Name) and t.id == name for t in targets):
            return node.value
    raise AssertionError(f"main.py has no module-level {name}")


def find_function(name: str):
    for node in ast.walk(TREE):
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name == name:
            return node
    raise AssertionError(f"main.py has no function {name}")


def module_function(name: str):
    for node in TREE.body:
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name == name:
            return node
    raise AssertionError(f"main.py has no module-level function {name}")


def load(name: str, namespace: dict):
    node = module_function(name)
    exec(compile(ast.Module(body=[node], type_ignores=[]), name, "exec"), namespace)
    return namespace[name]


def call_name(call: ast.Call) -> str:
    target = call.func
    if isinstance(target, ast.Attribute):
        return target.attr
    if isinstance(target, ast.Name):
        return target.id
    return ""


def calls_to(func: ast.AST, name: str) -> list:
    return [n for n in ast.walk(func) if isinstance(n, ast.Call) and call_name(n) == name]


def forwards(call: ast.Call, keyword: str) -> bool:
    return any(kw.arg == keyword and isinstance(kw.value, ast.Name) and kw.value.id == keyword
               for kw in call.keywords)


def param_default(func, name: str):
    args = func.args
    positional = args.posonlyargs + args.args
    defaults = [None] * (len(positional) - len(args.defaults)) + list(args.defaults)
    for arg, default in zip(positional + args.kwonlyargs, defaults + list(args.kw_defaults)):
        if arg.arg == name:
            return "<required>" if default is None else ast.literal_eval(default)
    return "<missing>"


def list_constants(func) -> list:
    found = []
    for node in ast.walk(func):
        if isinstance(node, ast.List) and node.elts and all(isinstance(e, ast.Constant) for e in node.elts):
            found.append([e.value for e in node.elts])
    return found


def plan():
    return ast.literal_eval(module_assign("ALL_STAGE_PLAN"))


def all_function():
    return find_function("all")


class StagePlanTest(unittest.TestCase):
    def test_plan_matches_the_design_table(self):
        self.assertEqual(tuple((n, h, tuple(d)) for n, h, d in plan()), EXPECTED_PLAN)

    def test_glibc_default_clippy_and_test_are_not_in_all(self):
        names = [entry[0] for entry in plan()]
        self.assertNotIn("clippy", names)
        self.assertNotIn("test", names)
        func = all_function()
        self.assertFalse(calls_to(func, "clippy"), "all must not call the two-pass clippy")
        self.assertFalse(calls_to(func, "test"), "all must not run the glibc cargo test")
        commands = list_constants(func)
        self.assertNotIn(["cargo", "test"], commands)
        self.assertNotIn(DEFAULT_CLIPPY, commands)

    def test_public_clippy_and_test_functions_stay(self):
        clippy = list_constants(find_function("clippy"))
        self.assertIn(DEFAULT_CLIPPY, clippy)
        self.assertIn(OTEL_CLIPPY, clippy)
        self.assertIn(["cargo", "test"], list_constants(find_function("test")))

    def test_clippy_otel_helper_runs_the_otel_pass_with_cargo_jobs(self):
        helper = module_function("_clippy_otel")
        self.assertEqual(param_default(helper, "cargo_jobs"), 2)
        self.assertEqual(list_constants(helper), [OTEL_CLIPPY])
        containers = calls_to(helper, "rust_container")
        self.assertTrue(containers and all(forwards(c, "cargo_jobs") for c in containers))

    def test_all_signature_defaults(self):
        func = all_function()
        self.assertEqual(param_default(func, "source_clean"), False)
        self.assertEqual(param_default(func, "cargo_jobs"), 2)
        self.assertEqual(param_default(func, "concurrency"), 1)

    def test_all_forwards_cargo_jobs_to_every_compiling_stage(self):
        for callee in ("static_release", "static_installer", "deploy_artifacts", "_clippy_otel"):
            with self.subTest(callee=callee):
                calls = calls_to(all_function(), callee)
                self.assertTrue(calls, f"all does not call {callee}")
                self.assertTrue(all(forwards(c, "cargo_jobs") for c in calls))

    def test_every_stage_run_is_a_nested_coroutine(self):
        func = all_function()
        runs = None
        for node in ast.walk(func):
            if isinstance(node, ast.Assign) and any(isinstance(t, ast.Name) and t.id == "runs" for t in node.targets):
                runs = node.value
        self.assertIsInstance(runs, ast.Dict, "all must map stage names to runs in a dict literal `runs`")
        keys = [ast.literal_eval(k) for k in runs.keys]
        self.assertEqual(keys, STAGE_NAMES)
        nested = {n.name for n in ast.walk(func) if isinstance(n, ast.AsyncFunctionDef) and n is not func}
        for key, value in zip(keys, runs.values):
            with self.subTest(stage=key):
                self.assertIsInstance(value, ast.Name)
                self.assertIn(value.id, nested, f"{key} must run an `async def` defined in all")

    def test_stages_are_built_from_the_plan(self):
        func = all_function()
        self.assertEqual(len(calls_to(func, "Stage")), 1, "build every Stage from ALL_STAGE_PLAN in one place")
        self.assertTrue(any(isinstance(n, ast.Name) and n.id == "ALL_STAGE_PLAN" for n in ast.walk(func)))

    def test_memory_is_checked_before_any_stage_runs(self):
        func = all_function()
        guard = calls_to(func, "check_memory")
        runner = calls_to(func, "run_stages")
        self.assertEqual(len(guard), 1)
        self.assertEqual(len(runner), 1)
        self.assertLess(guard[0].lineno, runner[0].lineno)
        self.assertTrue(forwards(runner[0], "concurrency") or any(
            isinstance(a, ast.Name) and a.id == "concurrency" for a in runner[0].args))
        # The probe reads MemTotal from an alpine:3.22 container before the guard.
        def image(arg):
            if isinstance(arg, ast.Name):  # a module-level constant
                arg = module_assign(arg.id)
            return arg.value if isinstance(arg, ast.Constant) else None

        alpine = [c.lineno for c in calls_to(func, "from_") if c.args and image(c.args[0]) == "alpine:3.22"]
        self.assertTrue(alpine, "all must probe memory in an alpine:3.22 container")
        self.assertLess(min(alpine), guard[0].lineno)
        self.assertTrue(any("/proc/meminfo" in cmd for cmd in list_constants(func)))

    def test_options_are_validated_before_the_memory_probe(self):
        func = all_function()
        probe = min(c.lineno for c in calls_to(func, "from_"))
        validators = calls_to(func, "validate_cargo_jobs") + calls_to(func, "validate_concurrency")
        self.assertEqual(len(validators), 2)
        self.assertTrue(all(v.lineno < probe for v in validators))
        validate = load("validate_concurrency", {})
        for good in (1, 3):
            validate(good)
        for bad in (0, -1, True, 1.0, "3", None):
            with self.subTest(value=bad):
                with self.assertRaises(ValueError) as caught:
                    validate(bad)
                self.assertIn("concurrency", str(caught.exception))

    def test_stage_runner_tests_runs_test_stages_in_debian_with_the_repo(self):
        helper = module_function("_stage_runner_tests")
        runtimes = calls_to(helper, "instrumented_runtime")
        self.assertTrue(runtimes and isinstance(runtimes[0].args[0], ast.Constant)
                        and runtimes[0].args[0].value == "debian-bookworm")
        self.assertIn(["python3", "-m", "unittest", "discover", "-s", "ci/scripts", "-p", "test_stages.py", "-v"],
                      list_constants(helper))
        self.assertTrue(calls_to(all_function(), "_stage_runner_tests"))


class MemTotalParseTest(unittest.TestCase):
    def test_reads_mem_total_kib(self):
        parse = load("parse_mem_total_kib", {})
        self.assertEqual(parse("MemTotal:        6094848 kB\nMemFree:  1000 kB\n"), 6094848)

    def test_missing_mem_total_is_an_error(self):
        parse = load("parse_mem_total_kib", {})
        with self.assertRaises(ValueError):
            parse("MemFree: 1000 kB\n")


class StubPipelineTest(unittest.TestCase):
    """Drive run_stages with stub async stages built from the real plan."""

    def build(self, fail=None):
        events: list = []
        stages = []
        for name, heavy, needs in plan():
            async def run(name=name):
                events.append(f"start:{name}")
                await asyncio.sleep(0.01)
                if name == fail:
                    raise RuntimeError(f"{name} stub failure")
                events.append(f"end:{name}")
                return {"static_release": "MANIFEST", "loc_report": "TOKEI"}.get(name, "ok")
            stages.append(Stage(name, run, tuple(needs), heavy))
        return stages, events

    def test_same_stage_set_and_dependencies_at_concurrency_one_and_three(self):
        seen = {}
        for concurrency in (1, 3):
            stages, events = self.build()
            results = asyncio.run(run_stages(stages, concurrency))
            self.assertEqual([r.status for r in results], ["passed"] * len(EXPECTED_PLAN))
            for dependent in ("static_installer", "deploy_artifacts"):
                self.assertLess(events.index("end:static_release"), events.index(f"start:{dependent}"))
            seen[concurrency] = ([r.name for r in results], [s.needs for s in stages])
        self.assertEqual(seen[1], seen[3])
        self.assertEqual(seen[1][0], STAGE_NAMES)

    def test_concurrency_one_runs_in_plan_order(self):
        stages, events = self.build()
        asyncio.run(run_stages(stages, 1))
        self.assertEqual([e[len("start:"):] for e in events if e.startswith("start:")], STAGE_NAMES)

    def test_static_release_failure_skips_installer_and_deploy(self):
        for concurrency in (1, 3):
            with self.subTest(concurrency=concurrency):
                stages, _ = self.build(fail="static_release")
                with self.assertRaises(StageFailure) as caught:
                    asyncio.run(run_stages(stages, concurrency))
                status = {r.name: r.status for r in caught.exception.results}
                self.assertEqual(status.pop("static_release"), "failed")
                self.assertEqual(status.pop("static_installer"), "skipped")
                self.assertEqual(status.pop("deploy_artifacts"), "skipped")
                self.assertEqual(set(status.values()), {"passed"})
                text = str(caught.exception)
                self.assertIn("== static_release: failed", text)
                self.assertIn("static_release stub failure", text)
                self.assertIn("== static_installer: skipped", text)
                self.assertIn("== deploy_artifacts: skipped", text)

    def labels(self) -> dict:
        return ast.literal_eval(module_assign("STAGE_LABELS"))

    def test_summary_keeps_ok_lines_manifest_tokei_and_ends_with_timings(self):
        summary = load("all_summary", {"timings_line": timings_line, "STAGE_LABELS": self.labels()})
        for concurrency in (1, 3):
            with self.subTest(concurrency=concurrency):
                stages, _ = self.build()
                results = asyncio.run(run_stages(stages, concurrency))
                text = summary(results)
                lines = text.splitlines()
                self.assertTrue(lines[-1].startswith(TIMINGS_PREFIX))
                records = json.loads(lines[-1][len(TIMINGS_PREFIX):])
                self.assertEqual([r["name"] for r in records], STAGE_NAMES)
                self.assertTrue(all(set(r) == {"name", "status", "started_s", "duration_s"} for r in records))
                ok = [line for line in lines if line.startswith("== ") and line.endswith(": ok")]
                expected = [name for name in STAGE_NAMES if name != "loc_report"]
                self.assertEqual([line[3:].split(" ")[0].rstrip(":") for line in ok], expected)
                self.assertIn("== verified Linux bundle:\nMANIFEST\n", text)
                self.assertIn("== tokei (informational):\nTOKEI\n", text)
                self.assertLess(text.index("== tokei (informational):"), text.index(TIMINGS_PREFIX))

    def test_labels_start_with_the_stage_name(self):
        labels = self.labels()
        self.assertEqual(set(labels), {name for name in STAGE_NAMES if name != "loc_report"})
        for name, label in labels.items():
            self.assertTrue(label == name or label.startswith(f"{name} ("), label)


class LocReportTokeiTest(unittest.TestCase):
    """Task 2.3: bookworm has no tokei package, so Decision 4's fallback applies."""

    def test_loc_report_keeps_the_cached_source_install(self):
        func = find_function("loc_report")
        commands = list_constants(func)
        self.assertIn(["sh", "-c", TOKEI_SOURCE_INSTALL], commands)
        self.assertIn(["tokei", "--files"], commands)
        self.assertNotIn("apt-get", ast.get_source_segment(SOURCE, func))

    def test_loc_report_is_light_has_no_dependents_and_follows_the_heavy_stages(self):
        entries = plan()
        names = [entry[0] for entry in entries]
        self.assertEqual(names[0], "static_release")
        self.assertEqual([entry[1] for entry in entries[:3]], [True, True, True])
        self.assertEqual(entries[3], ("loc_report", False, ()))
        self.assertFalse(any("loc_report" in needs for _, _, needs in entries))

    def test_loc_report_overlaps_the_static_chain_at_concurrency_three(self):
        def stub(name):
            async def run():
                await asyncio.sleep(RESEARCH_DURATIONS_S[name] * TIME_SCALE)
                return "ok"
            return run

        stages = [Stage(name, stub(name), tuple(needs), heavy) for name, heavy, needs in plan()]
        results = {r.name: r for r in asyncio.run(run_stages(stages, 3))}
        static = results["static_release"]
        static_end = static.started_s + static.duration_s
        loc = results["loc_report"]
        self.assertLess(loc.started_s, static_end, "loc_report must start before static_release finishes")
        self.assertLess(loc.started_s + loc.duration_s, static_end, "loc_report must finish inside the static chain")
        light = [r for name, heavy, _ in plan() if not heavy for r in [results[name]]]
        self.assertEqual(min(light, key=lambda r: r.started_s).name, "loc_report",
                         "loc_report must take the first slot a heavy stage frees")


if __name__ == "__main__":
    unittest.main()
