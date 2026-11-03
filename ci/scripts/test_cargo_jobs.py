"""Cargo job-count plumbing through the Dagger module (task 2.1; Req 2.5, 3.1-3.3).

The Dagger module needs Python 3.14 and the dagger SDK, so it cannot be
imported here. These checks parse main.py and static_release.py with `ast`
and evaluate the extracted pieces (the CARGO_BUILD_JOBS value expression and
the validation helper) in isolation.
"""

import ast
from pathlib import Path
import unittest

MODULE_DIR = Path(__file__).resolve().parents[1] / "src" / "mmhmem_ci"
SOURCES = {name: (MODULE_DIR / name).read_text(encoding="utf-8") for name in ("main.py", "static_release.py")}
TREES = {name: ast.parse(text) for name, text in SOURCES.items()}


def functions(tree: ast.AST) -> dict[str, ast.AST]:
    found: dict[str, ast.AST] = {}
    for node in ast.walk(tree):
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            found.setdefault(node.name, node)
    return found


FUNCS = {name: functions(tree) for name, tree in TREES.items()}


def param_default(func: ast.AST, name: str):
    args = func.args
    positional = args.posonlyargs + args.args
    defaults = [None] * (len(positional) - len(args.defaults)) + list(args.defaults)
    for arg, default in zip(positional + args.kwonlyargs, defaults + list(args.kw_defaults)):
        if arg.arg == name:
            if default is None:
                return "<required>"
            return ast.literal_eval(default)
    return "<missing>"


def call_name(call: ast.Call) -> str:
    target = call.func
    if isinstance(target, ast.Attribute):
        return target.attr
    if isinstance(target, ast.Name):
        return target.id
    return ""


def calls_to(func: ast.AST, name: str) -> list[ast.Call]:
    return [n for n in ast.walk(func) if isinstance(n, ast.Call) and call_name(n) == name]


def forwards_cargo_jobs(call: ast.Call) -> bool:
    return any(kw.arg == "cargo_jobs" and isinstance(kw.value, ast.Name) and kw.value.id == "cargo_jobs"
               for kw in call.keywords)


def jobs_env_values(func: ast.AST) -> list[ast.expr]:
    values = []
    for call in calls_to(func, "with_env_variable"):
        if call.args and isinstance(call.args[0], ast.Constant) and call.args[0].value == "CARGO_BUILD_JOBS":
            values.append(call.args[1])
    return values


def load_validator():
    node = FUNCS["static_release.py"].get("validate_cargo_jobs")
    if node is None:
        raise AssertionError("static_release.validate_cargo_jobs is missing")
    namespace: dict = {}
    exec(compile(ast.Module(body=[node], type_ignores=[]), "validate_cargo_jobs", "exec"), namespace)
    return namespace["validate_cargo_jobs"]


# (file, container factory)
FACTORIES = [("main.py", "rust_container"), ("static_release.py", "musl_container")]
# (file, function, callee that must receive cargo_jobs=cargo_jobs)
FORWARDING = [
    ("main.py", "static_installer", "installer_check"),
    ("main.py", "static_release", "verified_bundle"),
    ("main.py", "release", "static_release"),
    ("main.py", "deploy_artifacts", "release"),
    ("main.py", "all", "static_release"),
    ("main.py", "all", "static_installer"),
    ("main.py", "all", "deploy_artifacts"),
    ("static_release.py", "installer_check", "verified_bundle"),
    ("static_release.py", "verified_bundle", "build_candidate"),
    ("static_release.py", "build_candidate", "musl_container"),
]


class CargoJobsPlumbingTest(unittest.TestCase):
    def test_factories_default_to_two_and_use_the_option(self):
        for file, name in FACTORIES:
            with self.subTest(factory=name):
                func = FUNCS[file][name]
                self.assertEqual(param_default(func, "cargo_jobs"), 2)
                values = jobs_env_values(func)
                self.assertEqual(len(values), 1, "exactly one CARGO_BUILD_JOBS setting")
                self.assertNotIsInstance(values[0], ast.Constant, "CARGO_BUILD_JOBS must not be a literal")
                expr = compile(ast.Expression(values[0]), name, "eval")
                self.assertEqual(eval(expr, {"str": str, "cargo_jobs": 2}), "2")
                self.assertEqual(eval(expr, {"str": str, "cargo_jobs": 4}), "4")
                self.assertTrue(calls_to(func, "validate_cargo_jobs"), f"{name} must validate cargo_jobs")

    def test_no_hard_coded_job_count_remains(self):
        for file, text in SOURCES.items():
            with self.subTest(file=file):
                self.assertFalse('"CARGO_BUILD_JOBS", "2"' in text, f"{file} still hard-codes CARGO_BUILD_JOBS=2")

    def test_every_bundle_path_accepts_and_forwards_cargo_jobs(self):
        for file, name, callee in FORWARDING:
            with self.subTest(function=name, callee=callee):
                func = FUNCS[file][name]
                self.assertEqual(param_default(func, "cargo_jobs"), 2)
                calls = calls_to(func, callee)
                self.assertTrue(calls, f"{name} does not call {callee}")
                self.assertTrue(all(forwards_cargo_jobs(c) for c in calls),
                                f"{name} calls {callee} without cargo_jobs=cargo_jobs")

    def test_static_setup_keeps_the_default(self):
        # check_static_setup.py asserts CARGO_BUILD_JOBS == "2".
        for call in calls_to(FUNCS["static_release.py"]["setup_check"], "musl_container"):
            self.assertFalse(any(kw.arg == "cargo_jobs" for kw in call.keywords))
        self.assertEqual(param_default(FUNCS["main.py"]["static_setup"], "cargo_jobs"), "<missing>")

    def test_bundle_entrypoints_validate_before_any_build(self):
        for name, first_build in (("verified_bundle", "gate_tests"), ("installer_check", "instrumented_runtime")):
            with self.subTest(function=name):
                func = FUNCS["static_release.py"][name]
                validate = calls_to(func, "validate_cargo_jobs")
                self.assertTrue(validate, f"{name} must validate cargo_jobs")
                first = min(c.lineno for c in calls_to(func, first_build))
                self.assertLess(validate[0].lineno, first)

    def test_validator_rejects_values_below_one(self):
        validate = load_validator()
        for good in (1, 2, 4, 16):
            validate(good)
        for bad in (0, -1, True, 2.0, "4", None):
            with self.subTest(value=bad):
                with self.assertRaises(ValueError) as caught:
                    validate(bad)
                self.assertIn("cargo_jobs", str(caught.exception))


if __name__ == "__main__":
    unittest.main()
