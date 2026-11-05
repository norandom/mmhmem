"""Static file server for the installer e2e harness (scripts/e2e-run.ps1 -Phase serve).

Usage: e2e-http-server.py PORT DIRECTORY OUT_FILE LOG_FILE

The server outlives the workflow step that starts it, so every standard stream
is pointed at a file it owns before serving. On Linux and macOS, PowerShell's
Start-Process -RedirectStandardError is a pipe that PowerShell itself copies to
the file, and that pipe breaks when the starting step exits: the first request
logged after that raised BrokenPipeError inside the handler, so the server
accepted connections but answered none (installer-e2e run 41883221749).
"""
import functools
import http.server
import os
import sys


def own_streams(out_path, log_path):
    """Replace stdin/stdout/stderr (fds 0-2) with files this process owns."""
    flags = os.O_WRONLY | os.O_CREAT | os.O_APPEND
    for fd, path, mode in ((0, os.devnull, os.O_RDONLY), (1, out_path, flags), (2, log_path, flags)):
        opened = os.open(path, mode, 0o644)
        os.dup2(opened, fd)
        os.close(opened)
    sys.stdout = open(1, "w", buffering=1, encoding="utf-8", closefd=False)
    sys.stderr = open(2, "w", buffering=1, encoding="utf-8", closefd=False)


def main(argv):
    if len(argv) != 5:
        print(__doc__, file=sys.stderr)
        return 2
    port, directory, out_path, log_path = int(argv[1]), argv[2], argv[3], argv[4]
    own_streams(out_path, log_path)
    handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=directory)
    with http.server.ThreadingHTTPServer(("127.0.0.1", port), handler) as server:
        print(f"Serving {directory} on http://127.0.0.1:{port}/", flush=True)
        server.serve_forever()
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
