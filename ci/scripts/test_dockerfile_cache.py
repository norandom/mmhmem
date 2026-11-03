"""Contract tests for the Dockerfile's optional sccache path (ci-dependency-cache task 2.2).

Requirements 1.1, 1.2, 2.3, 2.4, 5.1; design "Dockerfile builder path".
"""

import ast
from pathlib import Path
import re
import shlex
import sys
import unittest

ROOT = Path(__file__).resolve().parents[2]
DOCKERFILE = (ROOT / "Dockerfile").read_text(encoding="utf-8")
MODULE_DIR = ROOT / "ci" / "src" / "mmhmem_ci"
sys.path.insert(0, str(MODULE_DIR))

import sccache  # noqa: E402

MAIN = ast.parse((MODULE_DIR / "main.py").read_text(encoding="utf-8"))


def builder_stage() -> str:
    start = DOCKERFILE.index(" AS builder")
    return DOCKERFILE[start:DOCKERFILE.index(" AS runtime")]


def function(name: str):
    for node in ast.walk(MAIN):
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name == name:
            return node
    raise AssertionError(f"main.py has no function {name}")


def docker_build_calls(func):
    return [n for n in ast.walk(func) if isinstance(n, ast.Call)
            and isinstance(n.func, ast.Attribute) and n.func.attr == "docker_build"]


class DockerfileTests(unittest.TestCase):
    def test_sccache_build_args_default_to_empty(self):
        stage = builder_stage()
        self.assertRegex(stage, r"(?m)^ARG SCCACHE_URL=$")
        self.assertRegex(stage, r"(?m)^ARG SCCACHE_SHA256=$")

    def test_credentials_arrive_only_through_an_optional_secret_mount(self):
        stage = builder_stage()
        self.assertIn("--mount=type=secret,id=sccache_env", stage)
        self.assertNotIn("required=true", stage)
        self.assertNotRegex(stage, r"ARG\s+ACTIONS_|ENV\s+ACTIONS_", "no credential as ARG or ENV")

    def test_plain_build_keeps_todays_cargo_command(self):
        stage = builder_stage()
        self.assertIn("cargo build --release --locked --features otel", stage)
        self.assertRegex(stage, r"else\b[^;]*cargo build --release --locked --features otel",
                         "the branch without sccache runs the unchanged build")
        self.assertIn("cp target/release/mmhmem /build/mmhmem-out", stage)

    def test_cached_branch_verifies_the_download_and_records_stats(self):
        stage = builder_stage()
        self.assertIn('sha256sum -c', stage)
        self.assertIn("--show-stats --stats-format json", stage)
        self.assertIn("/build/sccache-stats.json", stage)
        self.assertIn("did not start", stage, "a failed server start falls back to an uncached build")


class SecretFileTests(unittest.TestCase):
    def test_secret_file_exports_credentials_and_cache_env_safely(self):
        text = sccache.docker_secret_env("read-only", "https://results.example/a'b", "tok$en")
        exported = {}
        for line in text.splitlines():
            self.assertTrue(line.startswith("export "), line)
            name, _, value = line[len("export "):].partition("=")
            exported[name] = shlex.split(value)[0]
        self.assertEqual(exported["ACTIONS_RESULTS_URL"], "https://results.example/a'b")
        self.assertEqual(exported["ACTIONS_RUNTIME_TOKEN"], "tok$en")
        for name, value in sccache.cache_env("read-only").items():
            self.assertEqual(exported[name], value)


class DaggerDockerBuildTests(unittest.TestCase):
    def test_disabled_cache_builds_exactly_as_before(self):
        helper = function("_docker_image")
        plain = [c for c in docker_build_calls(helper) if not c.args and not c.keywords]
        self.assertEqual(len(plain), 1, "the cache-off path calls src.docker_build() with no arguments")

    def test_enabled_cache_reads_builder_stats_then_builds_the_image(self):
        helper = function("_docker_image")
        configured = [c for c in docker_build_calls(helper) if c.keywords]
        names = [sorted(kw.arg for kw in c.keywords) for c in configured]
        self.assertIn(["build_args", "secrets", "target"], names, "builder-stage build for the stats file")
        self.assertIn(["build_args", "secrets"], names, "full image build with identical inputs")
        source = ast.unparse(helper)
        self.assertIn("/build/sccache-stats.json", source)
        self.assertTrue(re.search(r"set_secret\(\s*'sccache_env'", source), "secret named after the mount id")

    def test_docker_build_function_and_gate_pass_the_cache(self):
        self.assertIn("cache_mode", [a.arg for a in function("docker_build").args.args])
        calls = [n for n in ast.walk(function("all")) if isinstance(n, ast.Call)
                 and isinstance(n.func, ast.Name) and n.func.id == "_docker_image"]
        self.assertTrue(calls, "the gate's docker stage uses the cache-aware helper")


if __name__ == "__main__":
    unittest.main()
