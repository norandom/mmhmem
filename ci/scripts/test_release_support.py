"""Acceptance checks for disposable runtime infrastructure, without mmhmem internals."""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from release_support import (
    REQUIRED_CASES, RUNTIME_NAMES, ProbeWorkspace, create_certificates,
    new_validation_id, run, validate_identity,
)
from release_fixtures import bundle_fixture


class InfrastructureTests(unittest.TestCase):
    def test_validation_ids_are_fresh_and_labels_are_safe(self):
        validate_identity("a" * 40, os.environ["VALIDATION_ID"])
        first, second = new_validation_id(), new_validation_id()
        self.assertNotEqual(first, second)
        validate_identity("a" * 40, first)
        for revision, label in [("main", first), ("a" * 40, "../bad"), ("a" * 40, "")]:
            with self.assertRaises(ValueError):
                validate_identity(revision, label)

    def test_workspaces_isolate_stores_and_reports(self):
        with ProbeWorkspace() as first, ProbeWorkspace() as second:
            self.assertNotEqual(first.root, second.root)
            self.assertTrue(first.store.is_dir())
            self.assertTrue(first.reports.is_dir())
            first.write_report("fixture.json", {"status": "passed"})
            self.assertEqual(json.loads((first.reports / "fixture.json").read_text())["status"], "passed")
            with self.assertRaises(ValueError):
                first.write_report("../escape", {})
        self.assertFalse(first.root.exists())

    def test_child_errors_capture_diagnostics_and_timeout_kills_descendants(self):
        with self.assertRaisesRegex(RuntimeError, "fixture stderr"):
            run([sys.executable, "-c", "import sys; print('fixture stderr', file=sys.stderr); sys.exit(7)"])
        with tempfile.TemporaryDirectory(dir="/probe") as scratch:
            marker = Path(scratch) / "leaked"
            child = "import time,pathlib; time.sleep(1); pathlib.Path(" + repr(str(marker)) + ").touch()"
            parent = "import subprocess,sys,time; subprocess.Popen([sys.executable,'-c'," + repr(child) + "]); time.sleep(30)"
            with self.assertRaisesRegex(RuntimeError, "timed out"):
                run([sys.executable, "-c", parent], timeout=0.2)
            run([sys.executable, "-c", "import time; time.sleep(1.2)"])
            self.assertFalse(marker.exists(), "timed-out child's descendant survived")

    def test_schema_fixture_binds_exact_payload_and_required_cases(self):
        payload = b"synthetic bundle fixture, never an executable"
        report = bundle_fixture(payload)
        self.assertEqual(report["schema_version"], 1)
        self.assertEqual(report["artifact"]["sha256"], hashlib.sha256(payload).hexdigest())
        self.assertEqual(report["artifact"]["size_bytes"], len(payload))
        self.assertEqual(tuple(item["name"] for item in report["runtimes"]), RUNTIME_NAMES)
        for runtime in report["runtimes"]:
            self.assertEqual(set(runtime["cases"]), set(REQUIRED_CASES))
        self.assertEqual(report["target"], "x86_64-unknown-linux-musl")

    def test_disposable_roots_and_hostname_certificate(self):
        with ProbeWorkspace() as workspace:
            certs = create_certificates(workspace.root / "certificates", "worker-fixture", new_validation_id())
            run(["openssl", "verify", "-verify_hostname", "worker-fixture", "-CAfile", str(certs / "trusted-ca.crt"), str(certs / "server.crt")])
            for arguments in [
                ["-verify_hostname", "wrong-host", "-CAfile", str(certs / "trusted-ca.crt")],
                ["-CAfile", str(certs / "untrusted-ca.crt")],
            ]:
                with self.assertRaises(RuntimeError):
                    run(["openssl", "verify", *arguments, str(certs / "server.crt")])
            self.assertEqual((certs / "server.key").stat().st_mode & 0o777, 0o600)
            self.assertNotEqual((certs / "trusted-ca.crt").read_bytes(), (certs / "untrusted-ca.crt").read_bytes())


if __name__ == "__main__":
    unittest.main()
