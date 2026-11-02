"""Loopback HTTP checks for installer validation; no provider credentials needed."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

root = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('check_worker', root / 'deploy/check_worker.py')
worker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(worker)
requests = []


class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        requests.append((body, self.headers.get('Authorization')))
        status = int(self.path.strip('/'))
        self.send_response(status)
        self.send_header('Content-Type', 'application/json')
        self.end_headers()
        self.wfile.write(json.dumps(
            {'choices': [{'message': {'content': 'OK'}}]} if status == 200
            else {'error': 'fixture-key must never appear in output'}
        ).encode())

    def log_message(self, *args):
        pass


with ThreadingHTTPServer(('127.0.0.1', 0), Handler) as server:
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    for status in (200, 201, 400, 401, 403, 404, 429, 500):
        config = {'worker_model': 'fixture-model', 'worker_api_key': 'fixture-key',
                  'worker_api_url': f'http://127.0.0.1:{server.server_port}/{status}'}
        with contextlib.redirect_stdout(io.StringIO()) as output:
            try:
                worker.check(config)
                assert status == 200, f'Bad response {status} accepted'
            except ValueError as error:
                assert status != 200
                assert 'fixture-key' not in str(error)
        assert 'fixture-key' not in output.getvalue()
    # Verify failed setup cannot change a prior installation; successful setup
    # updates PATH once and retains exactly one previous binary.
    with tempfile.TemporaryDirectory(dir=root / 'tmp', prefix='worker-install-') as directory:
        home = Path(directory)
        tools = home / 'tools'
        tools.mkdir()
        codex = tools / 'codex'
        codex.write_text('#!/bin/sh\nexit 0\n')
        codex.chmod(0o755)
        binary = home / '.local/bin/mmhmem'
        binary.parent.mkdir(parents=True)
        binary.write_text('old binary')
        source = home / 'release'
        source.write_text('new binary')
        config_file = home / '.mmhmem.json'
        original = json.dumps({'worker_model': 'old-model', 'worker_api_key': 'fixture-key'})
        config_file.write_text(original)
        env = dict(os.environ, HOME=str(home), SHELL='/bin/bash', PATH=str(tools) + ':' + os.environ['PATH'])
        args = ['bash', str(root / 'deploy/install_local.sh'), '--global', '--agent', 'codex',
                '--binary', str(binary), '--install-source', str(source), '--worker-model', 'fixture-model']
        for status in (401, 200, 200):
            result = subprocess.run(args + ['--worker-url', f'http://127.0.0.1:{server.server_port}/{status}'],
                                    env=env, input='', text=True, capture_output=True)
            assert 'fixture-key' not in result.stdout + result.stderr
            if status == 401:
                assert result.returncode != 0 and 'Authentication failed' in result.stderr, result
                assert binary.read_text() == 'old binary' and config_file.read_text() == original
                assert not (home / '.bashrc').exists() and not (home / '.codex').exists()
            else:
                assert result.returncode == 0, result.stderr
                assert binary.read_text() == 'new binary'
                assert len(list(binary.parent.glob('*.old'))) == 1
                assert (home / '.bashrc').read_text().count('# mmhmem user tools') == 1
    server.shutdown()
    thread.join()
assert all(body['model'] == 'fixture-model' and body['temperature'] == 0
           and auth == 'Bearer fixture-key' for body, auth in requests)
print('PASS: loopback worker validation, authentication/model failures, malformed response, secret-safe errors')
