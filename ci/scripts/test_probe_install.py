"""Candidate installer checks must reject invalid bytes before executing them."""

import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
import urllib.error
import urllib.request

from probe_install import validate_candidate, worker_fixture


class CandidateInstallerTests(unittest.TestCase):
    def test_preserved_worker_fixture_accepts_real_installer_validation(self):
        checker_path = Path(__file__).resolve().parents[2] / "deploy/check_worker.py"
        spec = importlib.util.spec_from_file_location("installer_worker_checker", checker_path)
        checker = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(checker)
        with worker_fixture() as server:
            url = f"http://127.0.0.1:{server.server_port}/v1/chat/completions"
            payload = {"model": "preserved-model", "messages": [{"role": "user", "content": "Reply OK."}]}
            request = urllib.request.Request(url, data=json.dumps(payload).encode(),
                                             headers={"Content-Type": "application/json"})
            with urllib.request.urlopen(request, timeout=5) as response:
                self.assertEqual(json.load(response)["choices"][0]["message"]["content"], "OK")
            self.assertEqual(server.requests, 1)
            payload["model"] = "wrong-model"
            request = urllib.request.Request(url, data=json.dumps(payload).encode())
            with self.assertRaises(urllib.error.HTTPError) as error:
                urllib.request.urlopen(request, timeout=5)
            self.assertEqual(error.exception.code, 400)
            self.assertEqual(server.requests, 1)
            checker.check({"worker_model": "preserved-model", "worker_api_url": url, "worker_api_key": ""})
            self.assertEqual(server.requests, 2)

    def test_candidate_checksum_is_required_before_execution(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "candidate"
            binary.write_bytes(b"never execute this file")
            with self.assertRaisesRegex(ValueError, "checksum"):
                validate_candidate(binary, "0" * 64)
            digest = hashlib.sha256(binary.read_bytes()).hexdigest()
            self.assertEqual(validate_candidate(binary, digest), digest)

    def test_missing_and_malformed_candidate_identity_are_rejected(self):
        with self.assertRaisesRegex(ValueError, "checksum"):
            validate_candidate(Path("/missing"), "not-a-hash")
        with self.assertRaises(OSError):
            validate_candidate(Path("/missing"), "0" * 64)


if __name__ == "__main__":
    unittest.main()
