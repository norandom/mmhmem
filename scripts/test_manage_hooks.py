"""Offline checks for deploy/manage_hooks.py (Claude Code entry, Linux and macOS).

Every path is a temporary override: no real harness settings are read or written.
Run: python3 scripts/test_manage_hooks.py  (or unittest discover -p "test_manage_hooks.py").
"""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import plistlib
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parent.parent
HELPER = ROOT / 'deploy' / 'manage_hooks.py'
BIN = '/opt/mmhmem dir/bin/mmhmem'
BIN2 = '/usr/local/other/mmhmem'
POSIX = os.name == 'posix'

NEIGHBOUR = (
    '{\n'
    '  "$schema": "https://json.schemastore.org/claude-code-settings.json",\n'
    '  "model": "opus",\n'
    '  "permissions": {\n'
    '    "allow": [\n'
    '      "Bash(npm run lint)"\n'
    '    ],\n'
    '    "deny": []\n'
    '  },\n'
    '  "hooks": {\n'
    '    "PreToolUse": [\n'
    '      {\n'
    '        "matcher": "Bash",\n'
    '        "hooks": [\n'
    '          {\n'
    '            "type": "command",\n'
    '            "command": "other-memory-plugin pre",\n'
    '            "timeout": 5\n'
    '          }\n'
    '        ]\n'
    '      }\n'
    '    ],\n'
    '    "PostToolUse": [\n'
    '      {\n'
    '        "matcher": "Edit",\n'
    '        "hooks": [\n'
    '          {\n'
    '            "type": "command",\n'
    '            "command": "fmt"\n'
    '          }\n'
    '        ]\n'
    '      }\n'
    '    ]\n'
    '  },\n'
    '  "note": "it\\u0027s caf\u00e9 \\"quoted\\"",\n'
    '  "ratio": 1.50\n'
    '}\n'
)


def load_module():
    spec = importlib.util.spec_from_file_location('manage_hooks', str(HELPER))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def ours(data):
    found = []
    pre = (data.get('hooks') or {}).get('PreToolUse') or []
    for group in pre:
        for handler in group.get('hooks') or []:
            name = str(handler.get('command', '')).replace('\\', '/').rsplit('/', 1)[-1]
            if name in ('mmhmem', 'mmhmem.exe') and handler.get('args') == ['hook', 'claude']:
                found.append((group, handler))
    return found


class ManageHooksTest(unittest.TestCase):
    def setUp(self):
        self.scratch = Path(tempfile.mkdtemp(prefix='manage-hooks-tests-'))
        self.no_managed = str(self.scratch / 'no-managed' / 'managed-settings.json')
        self.env = {k: v for k, v in os.environ.items() if k != 'CLAUDE_CONFIG_DIR'}
        self.env['PYTHONIOENCODING'] = 'utf-8'
        self.env['HOME'] = str(self.scratch / 'home')

    def tearDown(self):
        shutil.rmtree(str(self.scratch), ignore_errors=True)

    # --- helpers ---------------------------------------------------------------
    def run_helper(self, *args, env=None, cwd=None, managed=True):
        argv = [sys.executable, str(HELPER)] + list(args)
        if managed and '--managed-settings-path' not in args:
            argv += ['--managed-settings-path', self.no_managed]
        proc = subprocess.run(argv, cwd=str(cwd or self.scratch), env=env or self.env,
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        return proc.returncode, proc.stdout.decode('utf-8', 'replace')

    def install(self, settings, binary=BIN, *extra):
        return self.run_helper('install', '--harness', 'claude', '--binary', binary,
                               '--settings-path', str(settings), *extra)

    def remove(self, settings, *extra):
        return self.run_helper('remove', '--harness', 'claude', '--settings-path', str(settings), *extra)

    def case(self, name):
        path = self.scratch / name
        path.mkdir(parents=True, exist_ok=True)
        return path

    @staticmethod
    def write(path, text):
        path = Path(path)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(text.encode('utf-8'))

    @staticmethod
    def read_json(path):
        return json.loads(Path(path).read_bytes().decode('utf-8'))

    def assert_ok(self, result):
        self.assertEqual(result[0], 0, result[1])

    def assert_refused(self, result):
        self.assertNotEqual(result[0], 0, result[1])

    # --- 1. missing file ---------------------------------------------------------
    def test_install_into_missing_file(self):
        settings = self.case('missing') / 'deep' / 'claude-home' / 'settings.json'
        self.assert_ok(self.install(settings))
        raw = settings.read_bytes()
        self.assertFalse(raw.startswith(b'\xef\xbb\xbf'), 'created file has no BOM')
        found = ours(self.read_json(settings))
        self.assertEqual(len(found), 1)
        group, handler = found[0]
        self.assertEqual(group['matcher'], 'Read|Bash|PowerShell')
        self.assertEqual(handler, {'type': 'command', 'command': BIN, 'args': ['hook', 'claude'], 'timeout': 10})
        self.assertNotIn('shell', handler)
        self.assertFalse(Path(str(settings) + '.mmhmem.bak').exists(), 'no backup for a new file')
        self.assertEqual([p.name for p in settings.parent.iterdir()], ['settings.json'], 'no staging leftovers')
        if POSIX:
            self.assertEqual(stat.S_IMODE(settings.stat().st_mode), 0o600, 'new settings file is private')

    # --- 2-6. neighbours, repeat, replacement, exact removal, no-op removal ----------
    def test_neighbours_repeat_replace_remove(self):
        settings = self.case('neighbours') / 'settings.json'
        self.write(settings, NEIGHBOUR)
        original = settings.read_bytes()
        code, out = self.install(settings)
        self.assertEqual(code, 0, out)
        self.assertIn('backup', out)
        self.assertIn('trust', out, 'install prints the approval step')
        self.assertIn('python3', out)
        self.assertIn(str(HELPER), out, 'removal command names the helper')
        self.assertEqual(Path(str(settings) + '.mmhmem.bak').read_bytes(), original)
        data = self.read_json(settings)
        before = json.loads(NEIGHBOUR)
        self.assertEqual(list(data), list(before), 'top-level key order preserved')
        self.assertEqual(list(data['hooks']), ['PreToolUse', 'PostToolUse'])
        self.assertEqual(len(data['hooks']['PreToolUse']), 2)
        self.assertEqual(data['hooks']['PreToolUse'][0]['hooks'][0]['command'], 'other-memory-plugin pre')
        self.assertEqual(data['hooks']['PreToolUse'][1]['hooks'][0]['command'], BIN)
        self.assertEqual(len(ours(data)), 1)
        self.assertEqual(data['permissions'], {'allow': ['Bash(npm run lint)'], 'deny': []})
        text = settings.read_bytes().decode('utf-8')
        self.assertIn('"note": "it\\u0027s caf\u00e9 \\"quoted\\""', text)
        self.assertIn('"ratio": 1.50', text)
        self.assertIn('\n      {\n        "matcher": "Read|Bash|PowerShell",\n        "hooks": [\n', text,
                      'new group follows the file indentation')

        # 3. repeated install keeps exactly one entry
        after_first = settings.read_bytes()
        code, out = self.install(settings)
        self.assertEqual(code, 0, out)
        self.assertIn('unchanged', out)
        self.assertEqual(settings.read_bytes(), after_first)

        # 4. a different binary path replaces the entry in place
        code, out = self.install(settings, BIN2)
        self.assertEqual(code, 0, out)
        self.assertIn('replac', out.lower())
        data = self.read_json(settings)
        self.assertEqual(len(ours(data)), 1)
        self.assertEqual(data['hooks']['PreToolUse'][1]['hooks'][0]['command'], BIN2)

        # 5. remove restores the original exactly
        self.assert_ok(self.remove(settings))
        self.assertEqual(settings.read_bytes(), original)

        # 6. remove when absent is a no-op
        Path(str(settings) + '.mmhmem.bak').unlink()
        stamp = settings.stat().st_mtime_ns
        code, out = self.remove(settings)
        self.assertEqual(code, 0, out)
        self.assertIn('unchanged', out)
        self.assertEqual(settings.read_bytes(), original)
        self.assertEqual(settings.stat().st_mtime_ns, stamp)
        self.assertFalse(Path(str(settings) + '.mmhmem.bak').exists())
        missing = settings.parent / 'none' / 'settings.json'
        self.assert_ok(self.remove(missing))
        self.assertFalse(missing.parent.exists())

    # --- 7. CRLF and four-space indentation -------------------------------------------
    def test_crlf_round_trip(self):
        settings = self.case('crlf') / 'settings.json'
        self.write(settings, '{\r\n    "theme": "dark",\r\n    "env": {\r\n        "A": "1"\r\n    }\r\n}\r\n')
        original = settings.read_bytes()
        self.assert_ok(self.install(settings))
        text = settings.read_bytes().decode('utf-8')
        self.assertEqual(len(ours(json.loads(text))), 1)
        self.assertNotIn('\n', text.replace('\r\n', ''), 'CRLF line endings kept')
        self.assertIn('\r\n    "hooks": {\r\n        "PreToolUse": [', text)
        self.assert_ok(self.remove(settings))
        self.assertEqual(settings.read_bytes(), original)

    def test_bom_round_trip(self):
        settings = self.case('bom') / 'settings.json'
        settings.write_bytes(b'\xef\xbb\xbf{\n  "model": "opus"\n}\n')
        original = settings.read_bytes()
        self.assert_ok(self.install(settings))
        self.assertTrue(settings.read_bytes().startswith(b'\xef\xbb\xbf'))
        self.assert_ok(self.remove(settings))
        self.assertEqual(settings.read_bytes(), original)

    # --- 8. shared group ----------------------------------------------------------------------
    def test_shared_group(self):
        settings = self.case('shared-group') / 'settings.json'
        self.write(settings, '{"hooks":{"PreToolUse":[{"matcher":"Read","hooks":[{"type":"command","command":"guard"},'
                             '{"type":"command","command":"/old/mmhmem","args":["hook","claude"],"timeout":10}]}]}}')
        self.assert_ok(self.install(settings))
        data = self.read_json(settings)
        self.assertEqual(len(data['hooks']['PreToolUse']), 1)
        self.assertEqual(data['hooks']['PreToolUse'][0]['hooks'][1]['command'], BIN)
        self.assert_ok(self.remove(settings))
        self.assertEqual(settings.read_text(encoding='utf-8'),
                         '{"hooks":{"PreToolUse":[{"matcher":"Read","hooks":[{"type":"command","command":"guard"}]}]}}')

    # --- 9. identity -------------------------------------------------------------------------
    def test_identity(self):
        settings = self.case('identity') / 'settings.json'
        foreign = ('{"hooks":{"PreToolUse":[{"hooks":['
                   '{"type":"command","command":"/x/mmhmem","args":["hook","codex"]},'
                   '{"type":"command","command":"/x/notmmhmem","args":["hook","claude"]},'
                   '{"type":"command","command":"/x/mmhmem","args":["hook","claude","x"]},'
                   '{"type":"command","command":"/x/mmhmem hook claude"}]}]}}')
        self.write(settings, foreign)
        self.assert_ok(self.remove(settings))
        self.assertEqual(settings.read_text(encoding='utf-8'), foreign)
        # A Windows-style entry (mmhmem.exe, backslashes) is still recognised.
        self.write(settings, '{"hooks":{"PreToolUse":[{"hooks":[{"type":"command","command":"C:\\\\a\\\\mmhmem.exe","args":["hook","claude"]}]}]}}')
        self.assert_ok(self.remove(settings))
        self.assertEqual(settings.read_text(encoding='utf-8'), '{}')

    # --- 9b. compact files, emptied containers, duplicates ---------------------------------
    def test_compact_and_emptied_containers(self):
        settings = self.case('compact') / 'settings.json'
        self.write(settings, '{"hooks":{}}')
        self.assert_ok(self.install(settings))
        text = settings.read_text(encoding='utf-8')
        self.assertNotIn('\n', text)
        self.assertEqual(len(ours(json.loads(text))), 1)
        self.assert_ok(self.remove(settings))
        self.assertEqual(settings.read_text(encoding='utf-8'), '{}')

        self.write(settings, '{"hooks":{"PreToolUse":[]}}')
        self.assert_ok(self.install(settings))
        self.assert_ok(self.remove(settings))
        self.assertEqual(settings.read_text(encoding='utf-8'), '{}', 'pre-existing empty containers dropped')

        handler = '{"type":"command","command":"/a/mmhmem","args":["hook","claude"],"timeout":10}'
        self.write(settings, '{"hooks":{"PreToolUse":[{"matcher":"Read","hooks":[' + handler + ']},'
                             '{"matcher":"Bash","hooks":[' + handler + ']}]}}')
        self.assert_ok(self.install(settings))
        found = ours(self.read_json(settings))
        self.assertEqual(len(found), 1)
        self.assertEqual(found[0][1]['command'], BIN)
        self.assert_ok(self.remove(settings))
        self.assertEqual(settings.read_text(encoding='utf-8'), '{}')

        # the hooks object keeps other events when PreToolUse empties
        self.write(settings, '{"model":"x","hooks":{"Stop":[]}}')
        self.assert_ok(self.install(settings))
        self.assert_ok(self.remove(settings))
        self.assertEqual(settings.read_text(encoding='utf-8'), '{"model":"x","hooks":{"Stop":[]}}')

    # --- 10. unparsable or uneditable settings are refused unchanged ---------------------
    def test_unparsable_refused(self):
        settings = self.case('broken') / 'settings.json'
        bad_texts = [
            '{"model": "opus",, }',
            '{"model": "opus",}',
            '{"model": "opus"} // comment',
            "{'model': 'opus'}",
            '{"ratio": 01}',
            '{"ratio": NaN}',
            '{"hooks": [1]}',
            '{"hooks": {}, "hooks": {}}',
            '{"hooks": {"PreToolUse": {}}}',
            '[1]',
            '{"a": "\x01"}',
        ]
        for text in bad_texts:
            self.write(settings, text)
            original = settings.read_bytes()
            for op in ('install', 'remove'):
                result = self.install(settings) if op == 'install' else self.remove(settings)
                self.assert_refused(result)
                self.assertIn('Nothing was changed', result[1])
                self.assertEqual(settings.read_bytes(), original, text)
                self.assertFalse(Path(str(settings) + '.mmhmem.bak').exists())
        settings.write_bytes(b'{"a": "\xff"}')
        self.assert_refused(self.install(settings))

    # --- 11. policy refusals --------------------------------------------------------------
    def test_user_policy(self):
        settings = self.case('policy-user') / 'settings.json'
        self.write(settings, '{\n  "disableAllHooks": true\n}\n')
        original = settings.read_bytes()
        code, out = self.install(settings)
        self.assertNotEqual(code, 0, out)
        self.assertIn('disableAllHooks', out)
        self.assertEqual(settings.read_bytes(), original)
        code, out = self.run_helper('status', '--harness', 'claude', '--settings-path', str(settings))
        self.assertIn('policy-blocked', out)
        self.assertIn('disableAllHooks', out)

    def test_managed_policy(self):
        cases = {
            'disableAllHooks': '{"disableAllHooks": true}',
            'allowManagedHooksOnly': '{"allowManagedHooksOnly": true}',
            'strictPluginOnlyCustomization': '{"strictPluginOnlyCustomization": ["skills", "hooks"]}',
            'strictPluginOnlyCustomization: true': '{"strictPluginOnlyCustomization": true}',
            'strictPluginOnlyCustomization (non-boolean': '{"strictPluginOnlyCustomization": "yes"}',
            'cannot be parsed': '{"disableAllHooks": true,}',
            'not a JSON object': '[]',
        }
        for index, (needle, text) in enumerate(cases.items()):
            folder = self.case('policy-managed-%d' % index)
            settings = folder / 'settings.json'
            managed = folder / 'ClaudeCode' / 'managed-settings.json'
            self.write(managed, text)
            code, out = self.install(settings, BIN, '--managed-settings-path', str(managed))
            self.assertNotEqual(code, 0, out)
            self.assertIn(needle, out)
            self.assertIn('managed', out)
            self.assertFalse(settings.exists())

    def test_managed_dropins(self):
        folder = self.case('policy-dropin')
        settings = folder / 'settings.json'
        managed = folder / 'ClaudeCode' / 'managed-settings.json'
        self.write(folder / 'ClaudeCode' / 'managed-settings.d' / '10-security.json', '{"disableAllHooks": true}')
        code, out = self.install(settings, BIN, '--managed-settings-path', str(managed))
        self.assertNotEqual(code, 0, out)
        self.assertIn('disableAllHooks', out)
        self.assertIn('10-security', out)
        self.assertFalse(settings.exists())
        # Claude Code ignores hidden drop-ins and files that do not end in .json.
        (folder / 'ClaudeCode' / 'managed-settings.d' / '10-security.json').unlink()
        self.write(folder / 'ClaudeCode' / 'managed-settings.d' / '.hidden.json', '{"disableAllHooks": true}')
        self.write(folder / 'ClaudeCode' / 'managed-settings.d' / 'notes.txt', '{"disableAllHooks": true}')
        self.assert_ok(self.install(settings, BIN, '--managed-settings-path', str(managed)))

    def test_non_blocking_policy_and_status(self):
        folder = self.case('policy-off')
        settings = folder / 'settings.json'
        managed = folder / 'ClaudeCode' / 'managed-settings.json'
        self.write(managed, '{"disableAllHooks": false, "strictPluginOnlyCustomization": ["skills"]}')
        self.write(folder / 'ClaudeCode' / 'managed-settings.d' / '20-empty.json', '  \n')
        self.assert_ok(self.install(settings, BIN, '--managed-settings-path', str(managed)))
        self.assertEqual(len(ours(self.read_json(settings))), 1)
        code, out = self.run_helper('status', '--harness', 'claude', '--settings-path', str(settings),
                                    '--managed-settings-path', str(managed))
        self.assertEqual(code, 0, out)
        self.assertIn('installed', out)
        self.assertNotIn('not installed', out)
        self.assertIn(BIN, out)
        code, out = self.run_helper('status', '--harness', 'claude', '--settings-path', str(folder / 'none.json'),
                                    '--managed-settings-path', str(managed))
        self.assertEqual(code, 0, out)
        self.assertIn('not installed', out)

    def test_macos_managed_preferences(self):
        module = load_module()
        folder = self.case('plist')
        managed = str(folder / 'managed-settings.json')
        plist = folder / 'com.anthropic.claudecode.plist'
        with open(str(plist), 'wb') as handle:
            plistlib.dump({'disableAllHooks': True}, handle)
        blocks = module.managed_blocks(managed, [str(plist)])
        self.assertEqual(len(blocks), 1)
        self.assertIn('disableAllHooks', blocks[0])
        self.assertIn(str(plist), blocks[0])
        with open(str(plist), 'wb') as handle:
            plistlib.dump({'strictPluginOnlyCustomization': ['hooks']}, handle, fmt=plistlib.FMT_BINARY)
        self.assertIn('strictPluginOnlyCustomization', module.managed_blocks(managed, [str(plist)])[0])
        plist.write_bytes(b'<?xml version="1.0"?><plist><dict><key>x</key>')
        self.assertIn('cannot be parsed', module.managed_blocks(managed, [str(plist)])[0])
        with open(str(plist), 'wb') as handle:
            plistlib.dump(['disableAllHooks'], handle)
        self.assertEqual(len(module.managed_blocks(managed, [str(plist)])), 1)
        with open(str(plist), 'wb') as handle:
            plistlib.dump({'disableAllHooks': False, 'model': 'opus'}, handle)
        self.assertEqual(module.managed_blocks(managed, [str(plist), str(folder / 'absent.plist')]), [])

    def test_default_managed_locations(self):
        module = load_module()
        self.assertEqual(module.default_managed_path('linux'), '/etc/claude-code/managed-settings.json')
        self.assertEqual(module.default_managed_path('darwin'),
                         '/Library/Application Support/ClaudeCode/managed-settings.json')
        self.assertEqual(module.managed_preference_paths('linux', 'alice'), [])
        self.assertEqual(module.managed_preference_paths('darwin', 'alice'), [
            '/Library/Managed Preferences/com.anthropic.claudecode.plist',
            '/Library/Managed Preferences/alice/com.anthropic.claudecode.plist',
        ])

    # --- 12. show and dry runs write nothing -------------------------------------------------
    def test_show_and_dry_runs(self):
        folder = self.case('show')
        settings = folder / 'cfg' / 'settings.json'
        code, out = self.run_helper('show', '--harness', 'claude', '--binary', BIN, '--settings-path', str(settings))
        self.assertEqual(code, 0, out)
        for needle in (json.dumps(BIN), '"hook"', '"claude"', str(settings), 'remove', 'smart_read', 'python3'):
            self.assertIn(needle, out)
        self.assertFalse((folder / 'cfg').exists())
        code, out = self.install(settings, BIN, '--dry-run')
        self.assertEqual(code, 0, out)
        self.assertIn('dry-run', out)
        self.assertFalse((folder / 'cfg').exists())

        settings = folder / 'settings.json'
        self.write(settings, NEIGHBOUR)
        self.assert_ok(self.install(settings))
        Path(str(settings) + '.mmhmem.bak').unlink()
        installed = settings.read_bytes()
        code, out = self.remove(settings, '--dry-run')
        self.assertEqual(code, 0, out)
        self.assertIn('dry-run', out)
        self.assertEqual(settings.read_bytes(), installed)
        self.assertFalse(Path(str(settings) + '.mmhmem.bak').exists())
        self.assert_ok(self.install(settings, BIN2, '--dry-run'))
        self.assertEqual(settings.read_bytes(), installed)
        self.assertEqual(sorted(p.name for p in folder.iterdir()), ['settings.json'])

    # --- 13. CLAUDE_CONFIG_DIR and HOME select the user settings file -------------------
    def test_user_settings_location(self):
        config = self.case('config-dir') / 'claude-home'
        env = dict(self.env, CLAUDE_CONFIG_DIR=str(config))
        code, out = self.run_helper('show', '--harness', 'claude', '--binary', BIN, env=env)
        self.assertEqual(code, 0, out)
        self.assertIn(str(config / 'settings.json'), out)
        code, out = self.run_helper('show', '--harness', 'claude', '--binary', BIN)
        self.assertEqual(code, 0, out)
        self.assertIn(os.path.join(self.env['HOME'], '.claude', 'settings.json'), out)
        self.assertFalse((self.scratch / 'home').exists())

    # --- 14. user scope only --------------------------------------------------------------
    def test_project_scope_refused(self):
        project = self.case('project')
        self.write(project / '.claude' / 'settings.json', '{"model":"sonnet"}')
        self.write(project / '.claude' / 'settings.local.json', '{}')

        def snapshot():
            return sorted((p.name, p.read_bytes()) for p in (project / '.claude').iterdir())
        before = snapshot()
        settings = self.case('project-user') / 'settings.json'
        code, out = self.run_helper('install', '--harness', 'claude', '--binary', BIN,
                                    '--settings-path', str(settings), cwd=project)
        self.assertEqual(code, 0, out)
        self.assertEqual(snapshot(), before)
        self.assertEqual([p.name for p in project.iterdir()], ['.claude'])
        for bad in (project / '.claude' / 'settings.json', project / '.claude' / 'settings.local.json'):
            code, out = self.run_helper('install', '--harness', 'claude', '--binary', BIN,
                                        '--settings-path', str(bad), cwd=project)
            self.assertNotEqual(code, 0, out)
            self.assertIn('project settings', out)
        self.assertEqual(snapshot(), before)

    # --- 15. the binary must be an absolute mmhmem path --------------------------------
    def test_binary_validation(self):
        for binary in ('mmhmem', 'bin/mmhmem', '~/bin/mmhmem', './mmhmem', '/opt/bin/notmmhmem', '/opt/bin/'):
            target = self.scratch / 'rel.json'
            code, out = self.install(target, binary)
            self.assertNotEqual(code, 0, binary + ': ' + out)
            self.assertFalse(target.exists(), binary)
        code, out = self.run_helper('install', '--harness', 'claude', '--settings-path', str(self.scratch / 'x.json'))
        self.assertNotEqual(code, 0, out)
        target = self.scratch / 'norm.json'
        self.assert_ok(self.install(target, '/opt/./x/../bin/mmhmem'))
        self.assertEqual(ours(self.read_json(target))[0][1]['command'], '/opt/bin/mmhmem')

    # --- 16. case-distinct keys --------------------------------------------------------------
    def test_case_distinct_keys(self):
        settings = self.case('case-keys') / 'settings.json'
        self.write(settings, '{\n  "Hooks": 1,\n  "hooks": {}\n}\n')
        self.assert_ok(self.install(settings))
        self.assertIn('"Hooks": 1', settings.read_text(encoding='utf-8'))

    # --- Unix file handling: modes and symlinks ------------------------------------------
    @unittest.skipUnless(POSIX, 'file modes are POSIX-only; chmod on Windows only toggles read-only')
    def test_mode_preserved(self):
        settings = self.case('mode') / 'settings.json'
        self.write(settings, NEIGHBOUR)
        os.chmod(str(settings), 0o640)
        original = settings.read_bytes()
        self.assert_ok(self.install(settings))
        self.assertEqual(stat.S_IMODE(settings.stat().st_mode), 0o640)
        self.assertEqual(stat.S_IMODE(Path(str(settings) + '.mmhmem.bak').stat().st_mode), 0o640)
        os.chmod(str(settings), 0o600)
        self.assert_ok(self.remove(settings))
        self.assertEqual(stat.S_IMODE(settings.stat().st_mode), 0o600)
        self.assertEqual(settings.read_bytes(), original)

    def make_symlink(self, link, target, directory=False):
        try:
            os.symlink(str(target), str(link), target_is_directory=directory)
        except (OSError, NotImplementedError) as error:
            self.skipTest('cannot create symlinks for this test user on this platform: %s' % error)

    def test_symlinked_target_refused(self):
        folder = self.case('symlink-file')
        real = folder / 'real.json'
        self.write(real, NEIGHBOUR)
        link = folder / 'settings.json'
        self.make_symlink(link, real)
        result = self.install(link)
        self.assert_refused(result)
        self.assertIn('link', result[1])
        self.assertEqual(real.read_text(encoding='utf-8'), NEIGHBOUR)
        # remove is refused as well when there is an entry to take out
        installed = folder / 'installed.json'
        self.write(installed, NEIGHBOUR)
        self.assert_ok(self.install(installed))
        Path(str(installed) + '.mmhmem.bak').unlink()
        real.write_bytes(installed.read_bytes())
        before = real.read_bytes()
        result = self.remove(link)
        self.assert_refused(result)
        self.assertIn('link', result[1])
        self.assertEqual(real.read_bytes(), before)
        self.assertTrue(link.is_symlink())
        self.assertFalse(Path(str(link) + '.mmhmem.bak').exists())

    def test_symlinked_parent_refused(self):
        folder = self.case('symlink-dir')
        real = folder / 'real-home'
        self.write(real / 'settings.json', NEIGHBOUR)
        linked = folder / 'linked-home'
        self.make_symlink(linked, real, directory=True)
        result = self.install(linked / 'settings.json')
        self.assert_refused(result)
        self.assertIn('link', result[1])
        self.assertEqual((real / 'settings.json').read_text(encoding='utf-8'), NEIGHBOUR)
        self.assertEqual(sorted(p.name for p in real.iterdir()), ['settings.json'])


class LinkPolicyTest(unittest.TestCase):
    """In-process checks of the symlink rule, so every platform covers the root cases."""

    def setUp(self):
        self.module = load_module()

    def check(self, path, links, euid):
        """links maps a path to the uid owning that link; returns the refusal text or None."""
        module = self.module
        root = os.path.abspath(os.sep)
        mapped = dict((os.path.join(root, *key.strip('/').split('/')), uid) for key, uid in links.items())
        target = os.path.join(root, *path.strip('/').split('/'))
        with contextlib.ExitStack() as stack:
            stack.enter_context(mock.patch.object(module, 'POSIX', True))
            stack.enter_context(mock.patch.object(module, '_is_link', lambda item: item in mapped))
            stack.enter_context(mock.patch.object(module, '_link_owner', lambda item: mapped[item]))
            stack.enter_context(mock.patch.object(module, '_euid', lambda: euid))
            try:
                module.assert_plain_path(target)
            except module.HelperError as error:
                return str(error)
        return None

    def test_plain_path_accepted(self):
        for euid in (0, 1000):
            self.assertIsNone(self.check('/home/alice/.claude/settings.json', {}, euid))

    def test_target_link_always_refused(self):
        for uid, euid in ((0, 0), (0, 1000), (1000, 1000)):
            self.assertIn('symbolic link', self.check('/home/alice/.claude/settings.json',
                                                      {'/home/alice/.claude/settings.json': uid}, euid))

    def test_user_owned_parent_refused(self):
        for euid in (0, 1000):
            self.assertIn('symbolic link', self.check('/home/alice/.claude/settings.json',
                                                      {'/home/alice/.claude': 1000}, euid))

    def test_root_owned_parent_exempt_for_users_only(self):
        links = {'/var/tmp/work': 0}
        self.assertIsNone(self.check('/var/tmp/work/settings.json', links, 1000))
        self.assertIn('symbolic link', self.check('/var/tmp/work/settings.json', links, 0))

    def test_root_keeps_only_system_layout_links(self):
        # macOS: /var, /tmp and /etc link into /private; merged-/usr: /bin, /lib, ...
        for name in ('var', 'tmp', 'etc', 'bin', 'sbin', 'lib', 'lib64'):
            self.assertIsNone(self.check('/%s/root/.claude/settings.json' % name, {'/' + name: 0}, 0), name)
        # the same names deeper down, or owned by another user, are not system layout
        self.assertIn('symbolic link', self.check('/srv/var/settings.json', {'/srv/var': 0}, 0))
        self.assertIn('symbolic link', self.check('/var/x/settings.json', {'/var': 1000}, 0))
        self.assertIn('symbolic link', self.check('/home/settings.json', {'/home': 0}, 0))


class ManagedNoteTest(unittest.TestCase):
    """A managed source the OS will not let this user read is skipped with a note."""

    def setUp(self):
        self.module = load_module()
        self.scratch = Path(tempfile.mkdtemp(prefix='manage-hooks-notes-'))

    def tearDown(self):
        shutil.rmtree(str(self.scratch), ignore_errors=True)

    def blocks(self, managed, denied):
        module = self.module
        real_read, real_list = module._read_bytes, module._list_dir

        def read(path):
            if path in denied:
                raise PermissionError(13, 'Permission denied', path)
            return real_read(path)

        def listing(path):
            if path in denied:
                raise PermissionError(13, 'Permission denied', path)
            return real_list(path)
        err = io.StringIO()
        with mock.patch.object(module, '_read_bytes', read), mock.patch.object(module, '_list_dir', listing):
            with contextlib.redirect_stderr(err):
                found = module.managed_blocks(managed, [])
        return found, err.getvalue()

    def test_denied_file_and_directory_are_noted(self):
        managed = str(self.scratch / 'managed-settings.json')
        Path(managed).write_text('{"disableAllHooks": true}', encoding='utf-8')
        drop_ins = self.scratch / 'managed-settings.d'
        drop_ins.mkdir()
        (drop_ins / '10-a.json').write_text('{}', encoding='utf-8')
        found, note = self.blocks(managed, {managed})
        self.assertEqual(found, [])
        self.assertIn(managed, note)
        self.assertIn('permission denied', note.lower())
        found, note = self.blocks(managed, {str(drop_ins)})
        self.assertEqual(len(found), 1)
        self.assertIn(str(drop_ins), note)
        found, note = self.blocks(managed, set())
        self.assertEqual(note, '')


if __name__ == '__main__':
    unittest.main(verbosity=2)
