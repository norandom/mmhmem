import json, pathlib, unittest
class CorpusTests(unittest.TestCase):
 def test_contract(self):
  root=pathlib.Path(__file__).resolve().parent
  cases=json.loads((root/'cases.json').read_text())['cases']
  self.assertGreaterEqual(len(cases),20)
  for c in cases:
   with self.subTest(case=c['id']):
    self.assertIn(c['operation'],['install','remove'])
    for key in ['input','expected']:
     if c.get(key) is not None:self.assertTrue((root/c[key]).is_file())
    self.assertIn(c['outcome'],['changed','unchanged','error','delete'])
 def test_runner_rejects_wrong_bytes(self):
  from runner import run_cases
  with self.assertRaises(AssertionError):run_cases(lambda case, source, canonical: ('changed',b'wrong'))
 def test_runner_rejects_wrong_outcome(self):
  from runner import run_cases
  with self.assertRaises(AssertionError):run_cases(lambda case, source, canonical: ('unchanged',None))
if __name__=='__main__':unittest.main()
