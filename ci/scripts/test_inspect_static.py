"""Static ELF acceptance, including real static PIE and incompatible fixtures."""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

from inspect_static import inspect, validate_header, validate_linkage
from release_support import run


class InspectorTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory(prefix="elf-inspection-")
        cls.root = Path(cls.temporary.name)
        source = cls.root / "fixture.rs"
        source.write_text('fn main() { println!("static inspection fixture"); }')
        cls.binary = cls.root / "static"
        flags = os.environ["CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS"].split()
        run(["rustc", "--target", "x86_64-unknown-linux-musl", "-C", "linker=musl-gcc",
             *flags, str(source), "-o", str(cls.binary)])
        cls.dynamic = cls.root / "dynamic"
        run(["rustc", str(source), "-o", str(cls.dynamic)])
        cls.digest = hashlib.sha256(cls.binary.read_bytes()).hexdigest()

    @classmethod
    def tearDownClass(cls):
        cls.temporary.cleanup()

    def test_real_static_pie_has_structured_evidence(self):
        report = inspect(self.binary, self.digest)
        self.assertEqual(report["status"], "passed")
        self.assertEqual(report["artifact"]["sha256"], self.digest)
        self.assertEqual(report["artifact"]["size_bytes"], self.binary.stat().st_size)
        self.assertEqual(report["elf"]["type"], "DYN")
        self.assertIn("Dynamic section", report["readelf"]["dynamic"])
        self.assertEqual(run([str(self.binary)]), "static inspection fixture")

    def test_rejects_real_dynamic_binary(self):
        digest = hashlib.sha256(self.dynamic.read_bytes()).hexdigest()
        with self.assertRaisesRegex(ValueError, "PT_INTERP"):
            inspect(self.dynamic, digest)

    def test_rejects_real_wrong_architecture(self):
        payload = bytearray(self.binary.read_bytes())
        payload[18:20] = (183).to_bytes(2, "little")
        binary = self.root / "aarch64"
        binary.write_bytes(payload)
        with self.assertRaisesRegex(ValueError, "x86-64"):
            inspect(binary, hashlib.sha256(payload).hexdigest())

    def test_rejects_malformed_and_checksum_mismatch(self):
        broken = self.root / "broken"
        broken.write_bytes(b"not an ELF executable")
        with self.assertRaisesRegex(RuntimeError, "readelf"):
            inspect(broken, hashlib.sha256(broken.read_bytes()).hexdigest())
        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            inspect(self.binary, "0" * 64)
        with self.assertRaisesRegex(ValueError, "64 lowercase"):
            inspect(self.binary, "invalid")

    def test_metadata_rules_distinguish_static_pie_from_dynamic_linkage(self):
        valid = "  Class: ELF64\n  Type: DYN (Position-Independent Executable file)\n  Machine: Advanced Micro Devices X86-64\n"
        self.assertEqual(validate_header(valid)["type"], "DYN")
        with self.assertRaisesRegex(ValueError, "ELF64"):
            validate_header(valid.replace("ELF64", "ELF32"))
        with self.assertRaisesRegex(ValueError, "executable"):
            validate_header(valid.replace("DYN", "REL"))
        validate_linkage("LOAD", "Dynamic section: (FLAGS_1) PIE", "No version information")
        for field, values in (
            ("PT_INTERP", (" INTERP 0x0", "", "")),
            ("DT_NEEDED", ("LOAD", "(NEEDED) Shared library: [libc.so.6]", "")),
            ("GLIBC", ("LOAD", "", "Name: GLIBC_2.34")),
        ):
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, field):
                validate_linkage(*values)

    def test_cli_success_and_failure_reports(self):
        report_path = self.root / "linkage.json"
        command = ["python3", "ci/scripts/inspect_static.py", "--binary", str(self.binary),
                   "--expected-sha256", self.digest, "--report", str(report_path)]
        run(command)
        self.assertEqual(json.loads(report_path.read_text())["status"], "passed")
        command[command.index(self.digest)] = "0" * 64
        result = subprocess.run(command, capture_output=True, text=True, timeout=30)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("checksum mismatch", result.stderr)
        self.assertEqual(json.loads(report_path.read_text())["status"], "failed")


if __name__ == "__main__":
    unittest.main()
