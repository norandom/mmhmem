"""Candidate installer checks must reject invalid bytes before executing them."""

import hashlib
from pathlib import Path
import tempfile
import unittest

from probe_install import validate_candidate


class CandidateInstallerTests(unittest.TestCase):
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
