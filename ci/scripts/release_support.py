"""Standard-library infrastructure shared by explicit-artifact release checks."""

import json
import os
from pathlib import Path
import re
import signal
import subprocess
import tempfile
import uuid

TARGET = "x86_64-unknown-linux-musl"
ARTIFACT_NAME = "mmhmem-linux-x86_64"
RUNTIME_NAMES = ("debian-bookworm", "alpine-3.22")
REQUIRED_CASES = (
    "pristine_startup", "version", "mcp_initialize", "tools_list", "persistence",
    "stats_text", "stats_json", "server_health", "server_auth",
)


def new_validation_id():
    return "validation-" + uuid.uuid4().hex


def validate_identity(revision, validation_id):
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("revision must be a full lowercase Git commit ID")
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,127}", validation_id):
        raise ValueError("validation ID must be a nonempty safe label (maximum 128 characters)")


def run(arguments, *, timeout=30, **kwargs):
    """Bound execution and kill the whole child process group on timeout."""
    with subprocess.Popen(arguments, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                          text=True, start_new_session=True, **kwargs) as process:
        try:
            stdout, stderr = process.communicate(timeout=timeout)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            stdout, stderr = process.communicate()
            raise RuntimeError(f"{arguments!r} timed out after {timeout}s\nstdout: {stdout}\nstderr: {stderr}") from None
        if process.returncode:
            raise RuntimeError(f"{arguments!r} exited {process.returncode}\nstdout: {stdout}\nstderr: {stderr}")
        return stdout.strip()


class ProbeWorkspace:
    """Each process/store test gets a disposable writable root and report path."""

    def __enter__(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="release-probe-", dir="/probe")
        self.root = Path(self.temporary.name)
        self.store, self.reports = self.root / "store", self.root / "reports"
        self.store.mkdir(mode=0o700)
        self.reports.mkdir(mode=0o700)
        return self

    def write_report(self, name, report):
        if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]*\.json", name):
            raise ValueError("report name must be a simple JSON filename")
        (self.reports / name).write_text(json.dumps(report, indent=2) + "\n")

    def __exit__(self, *_):
        self.temporary.cleanup()


def create_certificates(destination, hostname, validation_id):
    """Generate private test material; callers export only reports/public roots."""
    validate_identity("0" * 40, validation_id)
    if not re.fullmatch(r"[a-z][a-z0-9-]{0,62}", hostname):
        raise ValueError("fixture hostname must be a DNS service label")
    destination = Path(destination)
    destination.mkdir(mode=0o700, parents=True)
    for name in ("trusted-ca", "untrusted-ca"):
        run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "2",
             "-subj", f"/CN={name}-{validation_id}", "-addext", "basicConstraints=critical,CA:TRUE",
             "-keyout", str(destination / f"{name}.key"), "-out", str(destination / f"{name}.crt")])
    run(["openssl", "req", "-new", "-newkey", "rsa:2048", "-nodes",
         "-subj", f"/CN={hostname}", "-keyout", str(destination / "server.key"), "-out", str(destination / "server.csr")])
    extensions = destination / "server.ext"
    extensions.write_text(f"subjectAltName=DNS:{hostname}\nbasicConstraints=critical,CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n")
    run(["openssl", "x509", "-req", "-in", str(destination / "server.csr"), "-days", "2",
         "-CA", str(destination / "trusted-ca.crt"), "-CAkey", str(destination / "trusted-ca.key"),
         "-CAcreateserial", "-extfile", str(extensions), "-out", str(destination / "server.crt")])
    for key in destination.glob("*.key"):
        key.chmod(0o600)
    return destination
