"""Offline Unix Codex helper contract; all writes stay under repository tmp/."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent


class CodexHooksTest(unittest.TestCase):
    def setUp(self):
        (ROOT / 'tmp').mkdir(exist_ok=True)
        self.scratch = Path(tempfile.mkdtemp(dir=str(ROOT / 'tmp')))
        self.target = self.scratch / 'hooks.json'
        self.config = self.scratch / 'config.toml'
        self.managed = self.scratch / 'requirements.toml'

    def tearDown(self):
        shutil.rmtree(str(self.scratch))

    def call(self, action, binary='/opt/bin dir/mmhmem', extra=()):
        env = dict(os.environ, CODEX_HOME=str(self.scratch))
        args = [sys.executable, str(ROOT / 'deploy/manage_hooks.py'), action,
                '--harness', 'codex', '--managed-settings-path', str(self.managed)]
        if action in ('show', 'install'):
            args += ['--binary', binary]
        p = subprocess.run(args + list(extra), env=env, capture_output=True)
        return p.returncode, (p.stdout + p.stderr).decode('utf-8', 'replace')

    def test_roundtrip_trust_and_idempotence(self):
        original = b'{"note":1.50,"hooks":{"Stop":[],"PreToolUse":[{"hooks":[{"command":"other"}]}]}}\n'
        self.target.write_bytes(original)
        config = b'[features]\nhooks = true\n[hooks.state."other"]\nenabled = true\n'
        self.config.write_bytes(config)
        code, output = self.call('install')
        self.assertEqual(code, 0, output)
        self.assertIn('/hooks', output)
        first = self.target.read_bytes()
        h = json.loads(first)['hooks']['PreToolUse'][-1]['hooks'][0]
        self.assertEqual(h['command'], "'/opt/bin dir/mmhmem' hook codex")
        self.assertNotIn('args', h)
        self.assertEqual(self.call('install')[0], 0)
        self.assertEqual(self.target.read_bytes(), first)
        self.assertEqual(self.config.read_bytes(), config)
        code, output = self.call('install', '/new/mmhmem')
        self.assertEqual(code, 0, output)
        self.assertIn('re-approval', output)
        self.assertEqual(self.call('remove')[0], 0)
        self.assertEqual(self.target.read_bytes(), original)
        self.assertEqual(self.config.read_bytes(), config)

    def test_policies_dry_run_and_show(self):
        for config, managed in [('[features]\nhooks = false', ''),
                                ('["features"]\nhooks = false', ''),
                                ('[features]\n"hooks" = false', ''),
                                ('features."hooks" = false', ''),
                                ('"features" = {"hooks" = false}', ''),
                                ('features = {"h\\u006foks" = false}', ''),
                                ("['features']\n'hooks' = false", ''),
                                ('', '["features"]\n"hooks" = false'),
                                ('', '"hooks" = false'),
                                ('', '"allow_managed_hooks_only" = true'),
                                ('', "'allow_managed_hooks_only' = true"),
                                ('features.hooks = false', ''),
                                ('', '[features]\nhooks = false'),
                                ('', 'hooks = false'),
                                ('', 'allow_managed_hooks_only = true')]:
            self.config.write_text(config)
            self.managed.write_text(managed)
            code, output = self.call('install')
            self.assertNotEqual(code, 0, output)
            self.assertIn('policy', output)
            self.assertFalse(self.target.exists())
            previous = b'{"note":"preserve me","hooks":{"Stop":[]}}\n'
            self.target.write_bytes(previous)
            code, output = self.call('install')
            self.assertNotEqual(code, 0, output)
            self.assertEqual(self.target.read_bytes(), previous)
            self.target.unlink()
        self.config.write_text('')
        self.managed.write_text('')
        self.assertEqual(self.call('install', extra=['--dry-run'])[0], 0)
        self.assertFalse(self.target.exists())
        code, output = self.call('show')
        self.assertEqual(code, 0, output)
        self.assertIn(str(self.target), output)
        self.assertIn('shell command', output)


if __name__ == '__main__':
    unittest.main()
