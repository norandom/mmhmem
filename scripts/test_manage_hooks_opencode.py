import os, pathlib, subprocess, tempfile, unittest, json, sys
ROOT=pathlib.Path(__file__).resolve().parents[1]
class OpenCodeHooks(unittest.TestCase):
 def test_install_policy_remove(self):
  self.exercise(False)
 def test_windows_install_policy_remove(self):
  if os.name != "nt": self.skipTest("Windows helper")
  self.exercise(True)
 def exercise(self, windows):
  with tempfile.TemporaryDirectory(dir=ROOT/'tmp') as folder:
   root=pathlib.Path(folder); target=root/'plugins/mmhmem-read-routing.js'; policy=root/'managed.json'
   def run(command,*extra):
    args = (['powershell', '-NoProfile', '-File',str(ROOT/'deploy/manage_hooks.ps1'),command,'-Harness','opencode','-Binary',str(ROOT/'target/debug/mmhmem.exe'),'-SettingsPath',str(target),'-ManagedSettingsPath',str(policy)] + ['-DryRun' for item in extra]) if windows else [sys.executable,str(ROOT/'deploy/manage_hooks.py'),command,'--harness','opencode','--binary','/opt/mmhmem','--settings-path',str(target),'--managed-settings-path',str(policy),*extra]
    return subprocess.run(args,capture_output=True,text=True)
   self.assertEqual(run('install','--dry-run').returncode,0); self.assertFalse(target.exists())
   self.assertEqual(run('install').returncode,0); first=target.read_bytes()
   self.assertIn(b'mmhmem',first)
   self.assertEqual(run('install').returncode,0); self.assertEqual(target.read_bytes(),first)
   policy.write_text(json.dumps({'policies':[{'action':'plugin.use','resource':'*','effect':'deny'}]}))
   self.assertNotEqual(run('install').returncode,0); self.assertEqual(target.read_bytes(),first)
   self.assertEqual(run('remove').returncode,0); self.assertFalse(target.exists())
if __name__=='__main__': unittest.main()
