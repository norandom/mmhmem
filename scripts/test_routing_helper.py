import importlib.util, pathlib, sys, tempfile, unittest
ROOT=pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'tests/fixtures/routing-instructions'))
from runner import run_cases
spec=importlib.util.spec_from_file_location('routing',ROOT/'deploy/manage_routing.py')
routing=importlib.util.module_from_spec(spec);spec.loader.exec_module(routing)
class RoutingTests(unittest.TestCase):
 def test_golden(self):
  with tempfile.TemporaryDirectory(dir=ROOT/'tmp') as directory:
   def adapter(case,source,canonical):
    path=pathlib.Path(directory)/'AGENTS.md'
    if path.exists():path.unlink()
    if source is not None:path.write_bytes(source)
    try:
     plan=routing.plan(path,case['operation'],canonical,case['dedicated'])
     return plan['outcome'],plan['bytes']
    except ValueError:return 'error',None
   self.assertEqual(run_cases(adapter),48)
 def test_commits(self):
  with tempfile.TemporaryDirectory(dir=ROOT/'tmp') as directory:
   path=pathlib.Path(directory)/'AGENTS.md';path.write_bytes(b'user')
   value=routing.plan(path,'install',b'read')
   routing.commit(value,dry_run=True);self.assertEqual(path.read_bytes(),b'user')
   for failure in ['backup','stage','replace']:
    with self.assertRaises(OSError):routing.commit(value,fail_at=failure)
    self.assertEqual(path.read_bytes(),b'user')
   routing.commit(value);self.assertEqual(pathlib.Path(str(path)+'.mmhmem.bak').read_bytes(),b'user')
   current=path.read_bytes();routing.commit(routing.plan(path,'install',b'updated'))
   self.assertEqual(pathlib.Path(str(path)+'.mmhmem.bak').read_bytes(),current)
   routing.commit(routing.plan(path,'remove'));self.assertEqual(path.read_bytes(),b'user')
   stale=routing.plan(path,'install',b'read')
   with self.assertRaises(ValueError):routing.commit(stale,before_commit=lambda:path.write_bytes(b'concurrent'))
   self.assertEqual(path.read_bytes(),b'concurrent')
   absent=path.parent/'new.md';value=routing.plan(absent,'install',b'read')
   with self.assertRaises(ValueError):routing.commit(value,before_commit=lambda:absent.write_bytes(b'appeared'))
   self.assertEqual(absent.read_bytes(),b'appeared')
   dedicated=path.parent/'rules.md';routing.commit(routing.plan(dedicated,'install',b'read',True))
   routing.commit(routing.plan(dedicated,'remove',dedicated=True));self.assertFalse(dedicated.exists())
   self.assertFalse(list(path.parent.glob('.mmhmem-*.tmp')))
 def test_dryrun_missing(self):
  with tempfile.TemporaryDirectory(dir=ROOT/'tmp') as directory:
   path=pathlib.Path(directory)/'missing'/'file.md'
   routing.commit(routing.plan(path,'install',b'read'),dry_run=True)
   self.assertFalse(path.parent.exists())
 def test_symlink(self):
  if sys.platform=='win32':self.skipTest('symlink privileges not assumed; canonical Linux gate verifies')
  with tempfile.TemporaryDirectory(dir=ROOT/'tmp') as directory:
   root=pathlib.Path(directory);target=root/'target';target.mkdir();link=root/'link';link.symlink_to(target,target_is_directory=True)
   with self.assertRaises(ValueError):routing.plan(link/'file','install',b'read')
   path=root/'file';path.write_bytes(b'user');backup=pathlib.Path(str(path)+'.mmhmem.bak');backup.symlink_to(target/'other')
   with self.assertRaises(ValueError):routing.commit(routing.plan(path,'install',b'read'))
   self.assertEqual(path.read_bytes(),b'user')
if __name__=='__main__':unittest.main()
