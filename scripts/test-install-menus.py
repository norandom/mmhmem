"""Exercise piped Bash bootstrap menus through a real pseudo-terminal (CI only)."""
import errno
import json
import os
from pathlib import Path
import pty
import re
import select
import signal
import subprocess
import sys
import time

root, scratch = map(Path, sys.argv[1:])
home = scratch / 'menu-home'
(home / '.config/Claude').mkdir(parents=True)
(home / '.config/opencode').mkdir(parents=True)
(home / '.config/opencode/opencode.json').write_text('{}')
env = dict(os.environ, HOME=str(home), NO_COLOR='1', TEST_REAL_ENGINE='1',
           TEST_OS='Darwin', TEST_ARCH='arm64', TMPDIR=str(scratch / 'temp'))


def terminal_case(answers, dry_run=True):
    pid, fd = pty.fork()
    if pid == 0:
        command = 'cat "$1" | bash -s --' + (' --dry-run' if dry_run else '')
        os.execvpe('bash', ['bash', '-c', command,
                          'test', str(root / 'install.sh')], env)
    output = bytearray()
    deadline = time.monotonic() + 20
    os.write(fd, answers.encode())
    try:
        while time.monotonic() < deadline:
            if not select.select([fd], [], [], 0.1)[0]:
                continue
            try:
                chunk = os.read(fd, 65536)
            except OSError as exc:
                if exc.errno == errno.EIO:
                    break
                raise
            if not chunk:
                break
            output.extend(chunk)
        else:
            raise AssertionError('Menu hung: ' + output.decode(errors='replace'))
        _, status = os.waitpid(pid, 0)
        assert os.waitstatus_to_exitcode(status) == 0, output.decode(errors='replace')
        return output.decode(errors='replace')
    finally:
        os.close(fd)
        try:
            os.kill(pid, signal.SIGKILL)
            os.waitpid(pid, 0)
        except ProcessLookupError:
            pass


output = terminal_case('1\n9\n1,3\nn\nn\nn\n')
assert '[3] codex' in output and 'Please choose numbers' in output, output
assert set(re.search(r'harnesses: ([^\r\n]+)', output)[1].split()) == {'opencode', 'codex'}, output
assert 'harnesses: claude-desktop' not in output, output
output = terminal_case('2\n3\n')
assert 'local mode removed' in output and 'harnesses: codex' in output, output
assert 'shared binary kept' in output, output
output = terminal_case('1\na\nn\nn\nn\n')
assert set(re.search(r'harnesses: ([^\r\n]+)', output)[1].split()) == {'opencode', 'claude-desktop', 'codex'}, output
output = terminal_case('q\n')
assert 'running the installer' not in output, output
output = terminal_case('1\nq\n', dry_run=False)
assert not (home / '.local/bin/mmhmem').exists(), 'Cancel installed a binary'
output = terminal_case('1\n3\nn\ny\ntest-model\nhttps://worker.example/v1/\n\ny\n')
assert 'worker model: test-model' in output and 'worker_api_url' in output, output
assert 'shunt_warmup' in output and '25 files' in output, output
saved = home / '.mmhmem.json'
saved.write_text(json.dumps({'worker_model': 'saved-model',
                            'worker_api_url': 'https://saved.example/v1/chat/completions',
                            'worker_api_key': 'private-fixture-token'}))
output = terminal_case('1\n3\nn\nn\nn\n')
assert 'Current base URL: https://saved.example/v1' in output, output
assert 'Current model ID: saved-model' in output and 'API key: ***' in output, output
assert 'private-fixture-token' not in output, output
assert output.index('Current model ID') < output.index('Select agents'), output
output = terminal_case('1\n3\nn\ny\n\n\n\nn\n')
assert 'Model ID [saved-model]' in output and 'base URL [https://saved.example/v1]' in output, output
assert 'API key [***]' in output and 'worker model: saved-model' in output, output
assert 'private-fixture-token' not in output, output
result = subprocess.run(['bash', str(root / 'deploy/install_local.sh'), '--global', '--agent', 'codex',
                         '--binary', str(home / 'missing'), '--dry-run'],
                        input='', text=True, capture_output=True, env=env)
assert result.returncode == 0 and 'Current model ID: saved-model' in result.stdout, result
assert 'private-fixture-token' not in result.stdout + result.stderr
assert json.loads(saved.read_text())['worker_api_key'] == 'private-fixture-token'
saved.unlink()
result = subprocess.run(['bash', str(root / 'deploy/install_local.sh'), '--global', '--dry-run'],
                        input='', text=True, capture_output=True, env=env)
assert result.returncode != 0 and 'pass --agent' in result.stderr, result
assert not (home / '.agents').exists(), 'Dry-run menu wrote configuration'
print('PASS: piped bootstrap terminal menus, invalid input, multi-select, Codex, removal, all, cancel, unattended refusal')
