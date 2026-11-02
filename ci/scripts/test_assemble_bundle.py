"""Release assembly accepts coherent evidence and invalidates failed candidates."""

import json
import os
from pathlib import Path
import shutil
import tempfile
import unittest

from assemble_bundle import assemble
from check_bundle import check_bundle
from release_fixtures import write_bundle_fixture
from release_support import ARTIFACT_NAME


class AssemblyTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(dir="/probe")
        self.root = Path(self.temporary.name)
        self.manifest = write_bundle_fixture(self.root)
        self.startup_evidence()

    def startup_evidence(self):
        self.write("scratch.json", {"status": "passed", "validation_id": "synthetic-fixture",
                   "artifact_sha256": self.manifest["artifact"]["sha256"], "reported_version": "mmhmem 0.2.2"})
        for runtime in self.manifest["runtimes"]:
            path = self.root / runtime["report"]
            report = json.loads(path.read_text())
            report["cases"]["pristine_startup"].update(artifact_sha256=self.manifest["artifact"]["sha256"],
                                                       reported_version="mmhmem 0.2.2")
            path.write_text(json.dumps(report))

    def tearDown(self):
        self.temporary.cleanup()

    def write(self, name, value):
        (self.root / "reports" / name).write_text(json.dumps(value))

    def assemble(self, clean=False):
        return assemble(self.root, self.manifest["revision"], "synthetic-fixture", clean)

    def assert_invalidated(self):
        self.assertFalse((self.root / "verification.json").exists())
        self.assertFalse((self.root / "SHA256SUMS").exists())
        self.assertEqual(json.loads((self.root / "reports/assembly-failure.json").read_text())["status"], "failed")

    def test_exact_candidate_and_source_cleanliness(self):
        result = self.assemble()
        self.assertFalse(result["publishable"])
        self.assertEqual(check_bundle(self.root, self.manifest["revision"])["artifact"], self.manifest["artifact"])
        self.assertTrue(self.assemble(True)["publishable"])

    def test_failed_runtime_cannot_leave_previous_success_bundle(self):
        runtime = dict(self.manifest["runtimes"][0], status="failed", validation_id="synthetic-fixture")
        self.write("debian-bookworm.json", runtime)
        with self.assertRaisesRegex(ValueError, "pass"):
            self.assemble()
        self.assert_invalidated()

    def test_changed_bytes_and_missing_evidence_fail_closed(self):
        for failure in ("bytes", "scratch", "runtime"):
            with self.subTest(failure=failure):
                self.manifest = write_bundle_fixture(self.root)
                self.startup_evidence()
                if failure == "bytes":
                    (self.root / ARTIFACT_NAME).write_bytes(b"tampered")
                else:
                    (self.root / "reports" / ("scratch.json" if failure == "scratch" else "alpine-3.22.json")).unlink()
                with self.assertRaises((ValueError, OSError)):
                    self.assemble()
                self.assert_invalidated()

    def test_startup_identity_and_version_must_match_candidate(self):
        scratch = json.loads((self.root / "reports/scratch.json").read_text())
        for key, wrong in (("artifact_sha256", "f" * 64), ("validation_id", "other"),
                           ("reported_version", "mmhmem 9.9"), ("status", "failed")):
            with self.subTest(key=key):
                self.write("scratch.json", {**scratch, key: wrong})
                with self.assertRaises(ValueError):
                    self.assemble()
                self.assert_invalidated()

    def test_revision_and_bool_inputs_are_not_coerced(self):
        for revision, clean in (("main", False), (self.manifest["revision"], "true")):
            with self.subTest(revision=revision, clean=clean), self.assertRaises(ValueError):
                assemble(self.root, revision, "synthetic-fixture", clean)
            self.assert_invalidated()


@unittest.skipUnless(os.environ.get("ACTUAL_BUNDLE"), "actual candidate supplied by canonical static gate")
class ActualGateFailureTests(unittest.TestCase):
    def test_injected_runtime_failure_with_real_candidate_exports_no_success(self):
        source = Path(os.environ["ACTUAL_BUNDLE"])
        manifest = json.loads((source / "verification.json").read_text())
        check_bundle(source, manifest["revision"])
        with tempfile.TemporaryDirectory(dir="/probe") as temporary:
            target = Path(temporary) / "injected"
            shutil.copytree(source, target)
            path = target / "reports/alpine-3.22.json"
            runtime = json.loads(path.read_text())
            runtime["cases"]["server_auth"]["status"] = "failed"
            path.write_text(json.dumps(runtime))
            with self.assertRaisesRegex(ValueError, "pass"):
                assemble(target, manifest["revision"], manifest["validation_id"], manifest["source_clean"])
            self.assertFalse((target / "verification.json").exists())
            self.assertFalse((target / "SHA256SUMS").exists())
            self.assertEqual((target / ARTIFACT_NAME).read_bytes(), (source / ARTIFACT_NAME).read_bytes())
            self.assertTrue((target / "reports/assembly-failure.json").is_file())


if __name__ == "__main__":
    unittest.main()
