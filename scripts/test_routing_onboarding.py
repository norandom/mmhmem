import importlib.util
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('routing_onboarding', ROOT/'deploy/routing_onboarding.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
class OnboardingTests(unittest.TestCase):
    def test_scope_repeat_remove(self):
        with tempfile.TemporaryDirectory(dir=ROOT/'tmp') as directory, patch.dict(os.environ, {}, clear=True):
            home = Path(directory); project = home/'project'; project.mkdir()
            for agent in ('claude', 'opencode'):
                for global_scope in (False, True):
                    value = module.prepare(agent, home, project, global_scope)
                    module.commit(value, True)
                    self.assertFalse(value['path'].exists())
                    module.commit(value)
                    repeated = module.prepare(agent, home, project, global_scope)
                    self.assertEqual(repeated['outcome'], 'unchanged')
                    module.commit(module.prepare(agent, home, project, global_scope, True))
                    self.assertFalse(value['path'].exists()) if agent == 'claude' else self.assertEqual(value['path'].read_bytes(), b'')
    def test_controls_do_not_leak_or_block_remove(self):
        with tempfile.TemporaryDirectory(dir=ROOT/'tmp') as directory:
            home = Path(directory)
            for name in ('OPENCODE_CONFIG_CONTENT', 'OPENCODE_CONFIG', 'OPENCODE_CONFIG_DIR', 'XDG_CONFIG_HOME'):
                with patch.dict(os.environ, {name: 'secret-content'}, clear=True):
                    with self.assertRaises(ValueError) as caught: module.prepare('opencode', home, home, True)
                    self.assertNotIn('secret-content', str(caught.exception))
                    self.assertIn(name, str(caught.exception))
            with patch.dict(os.environ, {'OPENCODE_DISABLE_PROJECT_CONFIG':'1'}, clear=True):
                with self.assertRaises(ValueError): module.prepare('opencode', home, home, False)
                module.prepare('opencode', home, home, False, True)
            with patch.dict(os.environ, {'CLAUDE_CODE_SIMPLE':'1'}, clear=True):
                with self.assertRaises(ValueError): module.prepare('claude', home, home, True)
                module.prepare('claude', home, home, True, True)
    def test_ownership_and_legacy_conflicts(self):
        with tempfile.TemporaryDirectory(dir=ROOT/'tmp') as directory, patch.dict(os.environ, {}, clear=True):
            home = Path(directory)
            (home/'CLAUDE.md').write_text('user')
            with patch.object(module.subprocess, 'run', return_value=type('Result', (), {'stdout':'1.0.0'})()):
                with self.assertRaises(ValueError): module.prepare('opencode', home, home, False)
            with patch.object(module.subprocess, 'run', return_value=type('Result', (), {'stdout':'2.2.5'})()):
                module.prepare('opencode', home, home, False)
            (home/'AGENTS.md').write_text('<!-- mmhmem:routing:start -->')
            with self.assertRaises(ValueError): module.prepare('opencode', home, home, False)
if __name__ == '__main__': unittest.main()
