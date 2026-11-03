"""Offline bootstrap/update of an explicit, already verified release file."""

import argparse
from contextlib import contextmanager
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import re
import shutil
import sys
import threading
import traceback

from probe_release import McpProcess
from release_support import ProbeWorkspace, run


@contextmanager
def worker_fixture():
    """Exercise the real installer validator against an isolated local worker."""
    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            valid = (self.path == "/v1/chat/completions"
                     and request.get("model") == "preserved-model"
                     and request.get("messages", [])[-1:] == [{"role": "user", "content": "Reply OK."}])
            self.server.requests += int(valid)
            self.send_response(200 if valid else 400)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(json.dumps({"choices": [{"message": {"content": "OK"}}]}).encode())

        def log_message(self, *_args):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    server.requests = 0
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield server
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


def validate_candidate(binary, expected_sha256):
    if not re.fullmatch(r"[0-9a-f]{64}", expected_sha256):
        raise ValueError("expected candidate checksum must be lowercase SHA-256")
    actual = hashlib.sha256(Path(binary).read_bytes()).hexdigest()
    if actual != expected_sha256:
        raise ValueError("installer candidate checksum mismatch")
    return actual


def write_executable(path, content):
    path.write_text(content)
    path.chmod(0o755)


def setup_fixture(workspace, binary, source):
    home, tools = workspace.root / "home", workspace.root / "bin"
    home.mkdir()
    tools.mkdir()
    temporary = workspace.root / "temp"
    temporary.mkdir()
    shutil.copy2(source / "install.sh", workspace.root / "bootstrap.sh")
    write_executable(tools / "uname", '#!/bin/sh\ncase "$1" in -s) echo Linux;; -m) echo "${TEST_ARCH:-x86_64}";; esac\n')
    # Ordinary Linux userlands are tested even when their CI host kernel is WSL.
    grep = shutil.which("grep")
    write_executable(tools / "grep", '#!/bin/sh\nif [ "$*" = "-qi microsoft /proc/version" ]; then exit 1; fi\nexec "' + grep + '" "$@"\n')
    write_executable(tools / "curl", '''#!/bin/sh
set -eu
out="$3"; url="$4"
printf '%s\\n' "$url" >> "$TEST_DOWNLOADS"
case "$url" in
  https://github.com/norandom/mmhmem/releases/latest/download/mmhmem-linux-x86_64)
    [ "${FAIL_BINARY:-0}" = 0 ] || exit 22
    cp "$TEST_CANDIDATE" "$out" ;;
  https://raw.githubusercontent.com/norandom/mmhmem/HEAD/*)
    cp "$TEST_SOURCE/${url#*/HEAD/}" "$out" ;;
  *) exit 23 ;;
esac
''')
    write_executable(tools / "codex", "#!/bin/sh\nexit 0\n")
    for name in ("cargo", "rustc", "cc", "gcc", "git"):
        write_executable(tools / name, '#!/bin/sh\nprintf "compiler-or-checkout-tool\\n" >> "$TEST_TOOL_CALLS"\nexit 99\n')
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("AGENT_MEM_", "WORKER_", "OTEL_", "XDG_"))}
    environment.update(HOME=str(home), TMPDIR=str(temporary), PATH=str(tools) + ":" + os.environ["PATH"],
                       XDG_DATA_HOME=str(home / ".local/share"), WORKER_API_URL="", WORKER_MODEL="", WORKER_API_KEY="",
                       TEST_CANDIDATE=str(binary), TEST_SOURCE=str(source),
                       TEST_DOWNLOADS=str(workspace.root / "downloads"), TEST_TOOL_CALLS=str(workspace.root / "tool-calls"))
    return home, environment


def memory(binary, environment, version, marker, *, record):
    context = {"project": "installer-preservation", "agent_harness": "opencode"}
    with McpProcess(binary, environment) as client:
        client.initialize(version)
        if record:
            reply = client.tool("mem_record_observation", {**context, "obs_type": "discovery", "title": marker,
                                "narrative": "memory survives the release update", "facts": marker})
            if "Recorded observation #" not in reply:
                raise ValueError(f"installer memory setup failed: {reply}")
        elif marker not in client.tool("mem_search", {**context, "query": marker}):
            raise ValueError("installed candidate did not recover existing memory")


def snapshot(paths):
    return {str(path): path.read_bytes() for path in paths}


def installed_registration(home, binary):
    text = (home / ".codex/config.toml").read_text()
    if text.count("[mcp_servers.mmhmem]") != 1 or str(binary) not in text or '"stdio"' not in text:
        raise ValueError(f"installer changed the Codex registration contract: {text}")
    for relative in (".agents/skills/unified-memory/SKILL.md", ".codex/prompts/mmhmem-recap.md"):
        if not (home / relative).is_file():
            raise ValueError(f"installer omitted marketplace/command path: {relative}")
    return text


def probe(binary, expected_sha256, source):
    binary, source = Path(binary).resolve(strict=True), Path(source).resolve(strict=True)
    validate_candidate(binary, expected_sha256)
    version = run([str(binary), "--version"], timeout=20)
    if not version.startswith("mmhmem "):
        raise ValueError(f"candidate returned invalid version: {version}")
    with ProbeWorkspace() as workspace, worker_fixture() as worker_server:
        home, environment = setup_fixture(workspace, binary, source)
        bootstrap = ["bash", str(workspace.root / "bootstrap.sh"), "--agent", "codex"]
        installed = home / ".local/bin/mmhmem"
        run(bootstrap, env=environment, cwd=home, timeout=30)
        validate_candidate(installed, expected_sha256)
        if run([str(installed), "--version"], env=environment, cwd=home, timeout=20) != version:
            raise ValueError("first install changed executable version")
        installed_registration(home, installed)
        marker = "installmemory" + expected_sha256[:20]
        memory(installed, environment, version.removeprefix("mmhmem "), marker, record=True)
        worker = home / ".mmhmem.json"
        worker.write_text(json.dumps({"worker_model": "preserved-model",
                                     "worker_api_url": f"http://127.0.0.1:{worker_server.server_port}/v1/chat/completions",
                                     "custom": "preserved"}) + "\n")
        registration = home / ".codex/config.toml"
        registration.write_text('model = "preserved-codex-model"\n' + registration.read_text())
        store = home / ".local/share/mmhmem/memory.db"
        preserved = snapshot((store, worker))
        # Construct the previous-version fixture separately; never truncate a
        # previously executed file (Linux can reject that with ETXTBSY).
        previous = installed.with_name("previous-installation")
        previous.write_bytes(b"previous installation fixture\n")
        previous.replace(installed)
        run(bootstrap, env=environment, cwd=home, timeout=30)
        if worker_server.requests != 1:
            raise ValueError("installer update did not validate the preserved worker")
        validate_candidate(installed, expected_sha256)
        if run([str(installed), "--version"], env=environment, cwd=home, timeout=20) != version:
            raise ValueError("update changed executable version")
        if snapshot((store, worker)) != preserved:
            raise ValueError("update modified existing memory or worker settings")
        if 'model = "preserved-codex-model"' not in installed_registration(home, installed):
            raise ValueError("update discarded unrelated harness configuration")
        memory(installed, environment, version.removeprefix("mmhmem "), marker, record=False)
        watched = (installed, store, worker, registration)
        before = snapshot(watched)
        downloads = Path(environment["TEST_DOWNLOADS"])
        downloads.write_text("")
        run([*bootstrap, "--dry-run"], env=environment, cwd=home, timeout=30)
        if "/releases/latest/" in downloads.read_text() or snapshot(watched) != before:
            raise ValueError("dry-run downloaded/replaced the binary or changed existing state")
        for override in ({"FAIL_BINARY": "1"}, {"TEST_ARCH": "aarch64"}):
            downloads.write_text("")
            try:
                run(bootstrap, env={**environment, **override}, cwd=home, timeout=30)
            except RuntimeError as error:
                expected = "download failed" if "FAIL_BINARY" in override else "no Linux aarch64 release asset"
                if expected not in str(error):
                    raise ValueError(f"wrong installer failure: {error}") from error
            else:
                raise ValueError(f"installer accepted failed download/unsupported platform: {override}")
            if snapshot(watched) != before or ("TEST_ARCH" in override and downloads.read_text()):
                raise ValueError("failed download/platform refusal altered existing state")
        if Path(environment["TEST_TOOL_CALLS"]).exists() or (home / ".git").exists() or (home / "Cargo.toml").exists():
            raise ValueError("installer required a compiler or source checkout")
        validate_candidate(binary, expected_sha256)
    return {"status": "passed", "artifact_sha256": expected_sha256, "reported_version": version,
            "cases": {name: {"status": "passed"} for name in
                      ("first_install", "update", "persistent_memory", "worker_settings", "harness_registration",
                       "dry_run", "failed_download", "unsupported_platform", "no_checkout_compiler", "stable_latest_asset")}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--expected-sha256", required=True)
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    args = parser.parse_args()
    try:
        result = probe(args.binary, args.expected_sha256, args.source)
    except (OSError, ValueError, RuntimeError, KeyError, TypeError) as error:
        args.report.write_text(json.dumps({"status": "failed", "error": str(error)}) + "\n")
        traceback.print_exc(file=sys.stderr)
        print(f"candidate installer probe failed: {error}", file=sys.stderr)
        return 1
    args.report.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result))
    return 0


if __name__ == "__main__":
    sys.exit(main())
