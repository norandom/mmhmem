"""Server acceptance and failure diagnostics through disposable processes."""

import os
from pathlib import Path
import tempfile
import time
import unittest

from probe_release import ServerProcess, server_cases, server_initialize


class ServerProbeTests(unittest.TestCase):
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

    def test_startup_exit_keeps_stderr_and_reaps_child(self):
        binary = self.script("import sys\nprint('startup refused',file=sys.stderr,flush=True)\nsys.exit(7)\n")
        server = ServerProcess(binary, {**os.environ, "AGENT_MEM_PORT": "1"}, timeout=1)
        with self.assertRaisesRegex(RuntimeError, "startup refused"):
            with server:
                self.fail("failed startup was accepted")
        self.assertEqual(server.process.returncode, 7)
        self.assertTrue(server.stderr.closed)

    def test_health_deadline_kills_descendants_and_keeps_stderr(self):
        pid = self.root / "descendant.pid"
        binary = self.script(f"import subprocess,sys,time\np=subprocess.Popen([sys.executable,'-c','import time;time.sleep(30)'])\nopen({str(pid)!r},'w').write(str(p.pid))\nprint('health fixture waiting',file=sys.stderr,flush=True)\ntime.sleep(30)\n")
        server = ServerProcess(binary, {**os.environ, "AGENT_MEM_PORT": "1"}, timeout=0.3)
        began = time.monotonic()
        with self.assertRaisesRegex(RuntimeError, "health fixture waiting"):
            with server:
                self.fail("unhealthy startup was accepted")
        self.assertLess(time.monotonic() - began, 3)
        child = Path("/proc") / pid.read_text() / "stat"
        for _ in range(50):
            try:
                state = child.read_text().split()[2]
            except FileNotFoundError:
                break
            if state == "Z":
                break
            time.sleep(0.02)
        else:
            self.fail("health timeout leaked a descendant")

    def http_fixture(self, behavior="good", shutdown="clean"):
        return self.script(f'''import json,os,signal,sys,time
from http.server import BaseHTTPRequestHandler,HTTPServer
if sys.argv[1:] == ['--version']:
 print('mmhmem 0.2.2');sys.exit(0)
signal.signal(signal.SIGTERM, lambda *_: sys.exit(0) if {shutdown!r} == 'clean' else None)
class Handler(BaseHTTPRequestHandler):
 def log_message(self,*_): pass
 def respond(self,status,body):
  self.send_response(status);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
 def do_GET(self): self.respond(200,b'ok')
 def do_POST(self):
  authenticated=self.headers.get('Authorization') == 'Bearer '+os.environ['AGENT_MEM_TOKEN']
  if not authenticated and {behavior!r} != 'auth-bypass': self.respond(401,b'rejected');return
  payload={{'jsonrpc':'2.0','id':1,'result':{{'serverInfo':{{'name':'mmhmem','version':'0.2.2'}},'capabilities':{{'tools':{{}}}}}}}}
  if {behavior!r} == 'bad-initialize': payload['result']={{'serverInfo':{{'version':'wrong'}}}}
  self.respond(200,('event: message\\ndata: '+json.dumps(payload)+'\\n\\n').encode())
HTTPServer(('127.0.0.1',int(os.environ['AGENT_MEM_PORT'])),Handler).serve_forever()
''')

    def test_real_http_fixture_accepts_health_auth_and_clean_shutdown(self):
        cases = server_cases(self.http_fixture(), "0.2.2", "server-test")
        self.assertEqual(cases["server_health"]["status"], "passed")
        self.assertEqual(cases["server_auth"]["unauthenticated_status"], 401)
        self.assertEqual(cases["server_auth"]["wrong_token_status"], 401)
        self.assertEqual(cases["server_auth"]["shutdown_exit_code"], 0)

    def test_auth_bypass_and_malformed_initialization_are_rejected(self):
        for behavior, expected in (("auth-bypass", "unauthenticated"), ("bad-initialize", "identity/capabilities")):
            with self.subTest(behavior=behavior):
                with self.assertRaisesRegex(RuntimeError, expected):
                    server_cases(self.http_fixture(behavior), "0.2.2", "server-test")

    def test_initialize_accepts_empty_sse_preamble_and_multiline_data(self):
        body = ('data: \nid: 0\nretry: 3000\n\n'
                'event: message\r\ndata: {"jsonrpc":"2.0","id":1,\r\n'
                'data: "result":{"serverInfo":{"name":"mmhmem","version":"0.2.2"},'
                '"capabilities":{"tools":{}}}}\r\n\r\n')
        self.assertEqual(server_initialize(body, "0.2.2")["version"], "0.2.2")

    def test_shutdown_timeout_is_failure_and_forces_cleanup(self):
        binary = self.http_fixture(shutdown="ignore")
        environment = {**os.environ, "AGENT_MEM_PORT": "0", "AGENT_MEM_TOKEN": "fixture"}
        # Isolate shutdown from health readiness: direct process entry uses a
        # known ephemeral port supplied by the same public server helper.
        server = ServerProcess(binary, environment, timeout=0.3)
        with self.assertRaisesRegex(RuntimeError, "shutdown timed out"):
            with server:
                pass
        self.assertIsNotNone(server.process.returncode)
        self.assertTrue(server.stderr.closed)

    def test_trickling_headers_and_body_obey_absolute_deadline_and_cleanup(self):
        for stage in ("health-headers", "request-headers", "request-body"):
            with self.subTest(stage=stage):
                pid = self.root / "trickle-child.pid"
                binary = self.script(f'''import os,signal,subprocess,sys,time
from http.server import BaseHTTPRequestHandler,HTTPServer
child=subprocess.Popen([sys.executable,'-c','import time;time.sleep(30)'])
open({str(pid)!r},'w').write(str(child.pid))
print('trickle fixture diagnostic',file=sys.stderr,flush=True)
signal.signal(signal.SIGTERM,lambda *_:sys.exit(0))
class Handler(BaseHTTPRequestHandler):
 def log_message(self,*_): pass
 def drip(self,payload):
  for byte in payload:
   self.connection.sendall(bytes([byte]));time.sleep(0.03)
 def reply(self,drip_headers=False,drip_body=False):
  body=b' '*40 if drip_body else b'ok'
  header=('HTTP/1.0 200 OK\\r\\nContent-Length: '+str(len(body))+'\\r\\n\\r\\n').encode()
  self.drip(header) if drip_headers else self.connection.sendall(header)
  self.drip(body) if drip_body else self.connection.sendall(body)
 def do_GET(self): self.reply(drip_headers={stage!r} == 'health-headers')
 def do_POST(self): self.reply(drip_headers={stage!r} == 'request-headers',drip_body={stage!r} == 'request-body')
HTTPServer(('127.0.0.1',int(os.environ['AGENT_MEM_PORT'])),Handler).serve_forever()
''')
                server = ServerProcess(binary, {**os.environ, "AGENT_MEM_PORT": "0"}, timeout=0.3)
                began = time.monotonic()
                expected = "health startup timed out" if stage == "health-headers" else "HTTP absolute deadline"
                with self.assertRaisesRegex(RuntimeError, "(?s)" + expected + ".*trickle fixture diagnostic"):
                    with server:
                        server.http("POST", "/mcp", timeout=0.2)
                self.assertLess(time.monotonic() - began, 0.9, "trickling extended the absolute deadline")
                self.assertIsNotNone(server.process.returncode)
                child = Path("/proc") / pid.read_text() / "stat"
                for _ in range(50):
                    try:
                        state = child.read_text().split()[2]
                    except FileNotFoundError:
                        break
                    if state == "Z":
                        break
                    time.sleep(0.02)
                else:
                    self.fail("HTTP deadline leaked a descendant")


if __name__ == "__main__":
    unittest.main()
