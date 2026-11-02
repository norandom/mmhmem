"""Synthetic bundle acceptance and provenance/evidence rejection checks."""

import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from check_bundle import check_bundle
from release_fixtures import write_bundle_fixture
from release_support import ARTIFACT_NAME, TARGET


class BundleTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(dir="/probe")
        self.root = Path(self.temporary.name)
        self.manifest = write_bundle_fixture(self.root)
        self.revision = self.manifest["revision"]

    def tearDown(self):
        self.temporary.cleanup()

    def save(self):
        (self.root / "verification.json").write_text(json.dumps(self.manifest))

    def report(self, name, change):
        path = self.root / name
        value = json.loads(path.read_text())
        change(value)
        path.write_text(json.dumps(value))

    def test_complete_bundle_and_dirty_nonpublishable_result(self):
        result = check_bundle(self.root, self.revision, require_publishable=True)
        self.assertEqual(result["artifact"], self.manifest["artifact"])
        self.manifest.update(source_clean=False, publishable=False)
        self.save()
        self.assertFalse(check_bundle(self.root, self.revision)["publishable"])
        with self.assertRaisesRegex(ValueError, "publishable"):
            check_bundle(self.root, self.revision, require_publishable=True)

    def test_artifact_and_checksum_file_tampering(self):
        binary = self.root / ARTIFACT_NAME
        original = binary.read_bytes()
        binary.write_bytes(original + b"tampered")
        with self.assertRaisesRegex(ValueError, "artifact"):
            check_bundle(self.root, self.revision)
        binary.write_bytes(original)
        for checksum in ["f" * 64 + "  " + ARTIFACT_NAME + "\n", "", "bad\n",
                         (self.root / "SHA256SUMS").read_text() * 2]:
            with self.subTest(checksum=checksum):
                (self.root / "SHA256SUMS").write_text(checksum)
                with self.assertRaisesRegex(ValueError, "SHA256SUMS"):
                    check_bundle(self.root, self.revision)

    def test_schema_provenance_types_and_identity_are_strict(self):
        original = copy.deepcopy(self.manifest)
        mutations = [
            ("schema_version", 2), ("schema_version", True), ("revision", "main"),
            ("target", "x86_64-unknown-linux-gnu"), ("source_clean", "true"),
            ("publishable", 1), ("source_clean", False), ("validation_id", "../escape"),
            ("lockfile_sha256", "F" * 64), ("build_image", "rust:latest"),
            ("rustc_version", "rustc 1.99.0"), ("cargo_version", "cargo 1.99.0"),
            ("package_version", ""), ("native_tools", {}),
        ]
        for name, value in mutations:
            with self.subTest(field=name, value=value):
                self.manifest = copy.deepcopy(original)
                self.manifest[name] = value
                self.save()
                with self.assertRaises(ValueError):
                    check_bundle(self.root, self.revision)
        self.manifest = original
        self.save()
        with self.assertRaisesRegex(ValueError, "revision"):
            check_bundle(self.root, "e" * 40)

    def test_missing_and_duplicate_runtime_or_case_records(self):
        original = copy.deepcopy(self.manifest)
        changes = [lambda m: m["runtimes"].pop(),
                   lambda m: m["runtimes"].append(m["runtimes"][0]),
                   lambda m: m["runtimes"][0].update(architecture="aarch64"),
                   lambda m: m["runtimes"][0].update(artifact_sha256="e" * 64),
            lambda m: m["runtimes"][0]["cases"].pop("server_auth"),
            lambda m: m["runtimes"][0]["cases"]["server_health"].update(status="failed"),
                   lambda m: m["runtimes"][0]["cases"]["persistence"].update(error="lost state")]
        for change in changes:
            with self.subTest(change=change):
                self.manifest = copy.deepcopy(original)
                change(self.manifest)
                self.save()
                with self.assertRaises(ValueError):
                    check_bundle(self.root, self.revision)

    def test_missing_required_fields_and_duplicate_json_keys(self):
        original = copy.deepcopy(self.manifest)
        for name in original:
            with self.subTest(missing=name):
                self.manifest = copy.deepcopy(original)
                self.manifest.pop(name)
                self.save()
                with self.assertRaises(ValueError):
                    check_bundle(self.root, self.revision)
        text = json.dumps(original)
        (self.root / "verification.json").write_text('{"schema_version":1,' + text[1:])
        with self.assertRaisesRegex(ValueError, "duplicate"):
            check_bundle(self.root, self.revision)

    def test_report_content_must_substantiate_manifest(self):
        reports = ["reports/linkage.json", "reports/target-suite.json", "reports/debian-bookworm.json"]
        mutations = [lambda r: r.update(status="failed"),
                     lambda r: r.update(target="wrong-target"),
                     lambda r: r.update(revision="e" * 40),
                     lambda r: r.update(error="contradiction")]
        for name in reports:
            original = (self.root / name).read_text()
            for change in mutations:
                with self.subTest(report=name, change=change):
                    self.report(name, change)
                    with self.assertRaises(ValueError):
                        check_bundle(self.root, self.revision)
                    (self.root / name).write_text(original)
        self.report("reports/target-suite.json", lambda r: r.update(validation_id="stale"))
        with self.assertRaisesRegex(ValueError, "validation_id"):
            check_bundle(self.root, self.revision)

    def test_linkage_regression_and_runtime_details_cannot_contradict_success(self):
        changes = [
            ("reports/linkage.json", lambda r: r["checks"]["interpreter"].update(status="failed")),
            ("reports/linkage.json", lambda r: r["readelf"].update(program="  INTERP 0x00\n  LOAD 0x00")),
            ("reports/target-suite.json", lambda r: r.update(optional_telemetry=True)),
            ("reports/target-suite.json", lambda r: r["checks"]["tests"].update(status="failed")),
            ("reports/target-suite.json", lambda r: r["artifact"].update(sha256="f" * 64)),
            ("reports/alpine-3.22.json", lambda r: r["cases"]["server_auth"].update(status="failed")),
        ]
        for name, change in changes:
            original = (self.root / name).read_text()
            with self.subTest(report=name):
                self.report(name, change)
                with self.assertRaises(ValueError):
                    check_bundle(self.root, self.revision)
                (self.root / name).write_text(original)

    def test_report_paths_and_symlinks_cannot_escape_bundle(self):
        self.manifest["linkage"]["report"] = "../linkage.json"
        self.save()
        with self.assertRaisesRegex(ValueError, "report"):
            check_bundle(self.root, self.revision)
        self.manifest["linkage"]["report"] = "reports/linkage.json"
        self.save()
        report = self.root / "reports/linkage.json"
        report.unlink()
        with tempfile.TemporaryDirectory(dir="/probe") as outside:
            external = Path(outside) / "linkage.json"
            external.write_text("{}")
            report.symlink_to(external)
            with self.assertRaisesRegex(ValueError, "symlink"):
                check_bundle(self.root, self.revision)

    def test_missing_report_and_cli_failure_exit(self):
        (self.root / "reports/alpine-3.22.json").unlink()
        result = subprocess.run([sys.executable, "ci/scripts/check_bundle.py", "--bundle", str(self.root),
                                 "--expected-revision", self.revision, "--require-publishable"],
                                capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 1)
        self.assertIn("alpine-3.22", result.stderr)
        self.assertNotIn('"status": "passed"', result.stdout)

    def test_wrong_userland_and_explicit_failed_states_are_rejected(self):
        runtime = self.manifest["runtimes"][0]
        runtime["image"] = "ubuntu:latest@sha256:" + "d" * 64
        self.report(runtime["report"], lambda r: r.update(image=runtime["image"]))
        self.save()
        with self.assertRaisesRegex(ValueError, "image"):
            check_bundle(self.root, self.revision)
        self.manifest = write_bundle_fixture(self.root)
        self.manifest["runtimes"][0]["status"] = "failed"
        self.save()
        with self.assertRaisesRegex(ValueError, "runtime"):
            check_bundle(self.root, self.revision)


if __name__ == "__main__":
    unittest.main()
