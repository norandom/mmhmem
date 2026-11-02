"""Static builder report and dependency rejection regression tests."""

import hashlib
from pathlib import Path
import tempfile
import unittest

from build_candidate import candidate_identity, validate_dependencies


class StaticBuilderTests(unittest.TestCase):
    def test_preserves_plain_rustls_crypto_graph(self):
        graph = "mmhmem v0.2.2\nreqwest v0.13.5\nrustls v0.23.1\naws-lc-rs v1.19.0\n"
        self.assertIn("aws-lc-rs", validate_dependencies(graph))

    def test_rejects_telemetry_and_native_dynamic_tls(self):
        for package in ("opentelemetry", "opentelemetry_sdk", "native-tls", "openssl-sys"):
            with self.subTest(package=package), self.assertRaisesRegex(ValueError, package):
                validate_dependencies(f"mmhmem v0.2.2\n{package} v1.0.0\n")

    def test_candidate_identity_binds_exact_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "mmhmem-linux-x86_64"
            binary.write_bytes(b"candidate bytes")
            identity = candidate_identity(binary)
            self.assertEqual(identity["sha256"], hashlib.sha256(b"candidate bytes").hexdigest())
            self.assertEqual(identity["size_bytes"], 15)
            binary.write_bytes(b"changed bytes")
            self.assertNotEqual(identity["sha256"], candidate_identity(binary)["sha256"])


if __name__ == "__main__":
    unittest.main()
