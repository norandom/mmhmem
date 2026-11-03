"""Failure-path checks for the explicit-artifact black-box probe."""

import hashlib
import json
import os
from pathlib import Path
import sys
import tempfile
import time
import unittest

from probe_release import McpProcess, validate_statistics, verify_binary
from release_support import run


class ProbeTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(dir="/probe")
        self.root = Path(self.temporary.name)

    def tearDown(self):
        self.temporary.cleanup()

    def script(self, body):
        path = self.root / "candidate"
        path.write_text("#!/usr/bin/env python3\n" + body)
        path.chmod(0o755)
        return path

    def test_checksum_rejection_precedes_execution(self):
        marker = self.root / "executed"
        binary = self.script(f"open({str(marker)!r}, 'w').write('ran')\n")
        with self.assertRaisesRegex(ValueError, "checksum"):
            verify_binary(binary, "0" * 64)
        self.assertFalse(marker.exists())
        digest = hashlib.sha256(binary.read_bytes()).hexdigest()
        self.assertEqual(verify_binary(binary, digest), digest)
        with self.assertRaisesRegex(ValueError, "SHA-256"):
            verify_binary(binary, "invalid")

    def test_malformed_protocol_includes_stderr(self):
        for response in ("broken JSON", '{"jsonrpc":"2.0","id":1,"result":[]}'):
            with self.subTest(response=response):
                binary = self.script(f"import sys,time\nprint('fixture diagnostic',file=sys.stderr,flush=True)\nprint({response!r},flush=True)\ntime.sleep(20)\n")
                with self.assertRaisesRegex(RuntimeError, "fixture diagnostic"):
                    with McpProcess(binary, os.environ.copy(), timeout=2) as client:
                        client.initialize("0.2.2")

    def test_timeout_kills_process_group_and_captures_diagnostics(self):
        pid = self.root / "child.pid"
        binary = self.script(f"import subprocess,sys,time\np=subprocess.Popen([sys.executable,'-c','import time;time.sleep(30)'])\nopen({str(pid)!r},'w').write(str(p.pid))\nprint('fixture waiting',file=sys.stderr,flush=True)\ntime.sleep(30)\n")
        began = time.monotonic()
        with self.assertRaisesRegex(RuntimeError, "fixture waiting"):
            with McpProcess(binary, os.environ.copy(), timeout=0.3) as client:
                client.request("initialize", {})
        self.assertLess(time.monotonic() - began, 3)
        child = Path("/proc") / pid.read_text() / "stat"
        for _ in range(50):
            if not child.exists() or child.read_text().split()[2] == "Z":
                break
            time.sleep(0.02)
        else:
            self.fail("timed-out candidate left a live descendant")

    def test_cli_failure_replaces_stale_passed_report(self):
        binary = self.script("raise RuntimeError('must never execute')\n")
        report = self.root / "report.json"
        report.write_text('{"status":"passed"}')
        with self.assertRaisesRegex(RuntimeError, "checksum"):
            run([sys.executable, "ci/scripts/probe_release.py", "--binary", str(binary),
                 "--expected-sha256", "0" * 64, "--expected-version", "0.2.2",
                 "--validation-id", "test-probe", "--report", str(report)])
        result = json.loads(report.read_text())
        self.assertEqual(result["status"], "failed")
        self.assertNotIn("cases", result)

    def test_statistics_require_recorded_row_and_consistent_typed_totals(self):
        values = {"avoided_input_tokens_est": 0, "frontier_prompt_tokens_client_reported": 1234,
                  "frontier_completion_tokens_client_reported": 56,
                  "worker_prompt_tokens": 0, "worker_completion_tokens": 0}
        report = {"days": [{"day": "2026-11-02", "project": "p", "harness": "opencode",
                            "client_reported_sources": ["static-probe"], **values}],
                  "totals": values.copy()}
        validate_statistics(report, "p")
        unreported = json.loads(json.dumps(report))
        unreported["days"][0]["frontier_prompt_tokens_client_reported"] = 0
        unreported["totals"]["frontier_prompt_tokens_client_reported"] = 0
        with self.assertRaisesRegex(ValueError, "client_usage payload"):
            validate_statistics(unreported, "p")
        with self.assertRaisesRegex(ValueError, "recorded session row"):
            validate_statistics({"days": [], "totals": values}, "p")
        report["totals"]["avoided_input_tokens_est"] = 1
        with self.assertRaisesRegex(ValueError, "inconsistent"):
            validate_statistics(report, "p")
        report["totals"]["avoided_input_tokens_est"] = False
        with self.assertRaisesRegex(ValueError, "invalid"):
            validate_statistics(report, "p")


if __name__ == "__main__":
    unittest.main()
