"""Exercise an explicit release file through public CLI and MCP interfaces."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import signal
import subprocess
import sys
import tempfile
import time

from release_support import ProbeWorkspace, TARGET, run, validate_identity

TOOLS = {"session_resume_or_handoff", "smart_read", "boilerplate_write", "mem_search",
         "mem_get_observations", "mem_record_observation", "shunt_warmup", "shunt_prune",
         "mem_search_breadcrumbs"}


def verify_binary(binary, expected_sha256):
    if not re.fullmatch(r"[0-9a-f]{64}", expected_sha256):
        raise ValueError("expected checksum must be a lowercase SHA-256 digest")
    actual = hashlib.sha256(Path(binary).read_bytes()).hexdigest()
    if actual != expected_sha256:
        raise ValueError(f"candidate checksum mismatch: expected {expected_sha256}, got {actual}")
    return actual


class McpProcess:
    """Bound every protocol operation and retain stderr without pipe deadlocks."""

    def __init__(self, binary, environment, timeout=20):
        self.binary, self.environment, self.timeout = binary, environment, timeout
        self.buffer, self.next_id = b"", 0

    def __enter__(self):
        self.stderr = tempfile.TemporaryFile(dir="/probe")
        try:
            self.process = subprocess.Popen([str(self.binary), "stdio"], env=self.environment,
                                            cwd=self.environment.get("HOME"), stdin=subprocess.PIPE,
                                            stdout=subprocess.PIPE, stderr=self.stderr, start_new_session=True)
        except OSError:
            self.stderr.close()
            raise
        os.set_blocking(self.process.stdin.fileno(), False)
        os.set_blocking(self.process.stdout.fileno(), False)
        return self

    def diagnostic(self, message):
        self.stderr.seek(0)
        return RuntimeError(f"{message}\nstderr: {self.stderr.read().decode(errors='replace')}")

    def send(self, message, deadline):
        payload = (json.dumps(message) + "\n").encode()
        with selectors.DefaultSelector() as ready:
            ready.register(self.process.stdin, selectors.EVENT_WRITE)
            while payload:
                remaining = deadline - time.monotonic()
                if remaining <= 0 or not ready.select(remaining):
                    raise self.diagnostic("candidate stdin timed out")
                try:
                    payload = payload[os.write(self.process.stdin.fileno(), payload):]
                except (BrokenPipeError, OSError) as error:
                    raise self.diagnostic(f"candidate stdin failed: {error}") from error

    def read_message(self, deadline):
        with selectors.DefaultSelector() as ready:
            ready.register(self.process.stdout, selectors.EVENT_READ)
            while b"\n" not in self.buffer:
                remaining = deadline - time.monotonic()
                if remaining <= 0 or not ready.select(remaining):
                    raise self.diagnostic("candidate response timed out")
                chunk = os.read(self.process.stdout.fileno(), 65536)
                if not chunk:
                    raise self.diagnostic("candidate stdout closed before response")
                self.buffer += chunk
                if len(self.buffer) > 4 * 1024 * 1024:
                    raise self.diagnostic("candidate protocol response exceeds 4 MiB")
            line, self.buffer = self.buffer.split(b"\n", 1)
        try:
            message = json.loads(line)
            if not isinstance(message, dict) or message.get("jsonrpc") != "2.0":
                raise ValueError("expected JSON-RPC object")
            return message
        except (ValueError, UnicodeDecodeError) as error:
            raise self.diagnostic(f"candidate malformed protocol response: {line[:200]!r}") from error

    def request(self, method, params):
        self.next_id += 1
        request_id, deadline = self.next_id, time.monotonic() + self.timeout
        self.send({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}, deadline)
        while True:
            reply = self.read_message(deadline)
            if reply.get("id") == request_id:
                if "error" in reply or not isinstance(reply.get("result"), dict):
                    raise self.diagnostic(f"{method} failed: {reply}")
                return reply["result"]

    def initialize(self, expected_version):
        reply = self.request("initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                             "clientInfo": {"name": "static-release-probe", "version": "1"}})
        if reply.get("serverInfo", {}).get("version") != expected_version or "tools" not in reply.get("capabilities", {}):
            raise self.diagnostic(f"initialize returned unexpected server identity/capabilities: {reply}")
        self.send({"jsonrpc": "2.0", "method": "notifications/initialized"}, time.monotonic() + self.timeout)
        return reply

    def tool(self, name, arguments):
        result = self.request("tools/call", {"name": name, "arguments": arguments})
        if result.get("isError"):
            raise self.diagnostic(f"{name} returned tool error: {result}")
        return "\n".join(item["text"] for item in result.get("content", []) if item.get("type") == "text")

    def __exit__(self, error_type, error, _):
        try:
            if error_type is not None and not isinstance(error, RuntimeError):
                raise self.diagnostic(f"candidate operation failed: {error}") from error
            if error_type is None:
                self.process.stdin.close()
                try:
                    code = self.process.wait(timeout=self.timeout)
                except subprocess.TimeoutExpired:
                    raise self.diagnostic("candidate clean EOF shutdown timed out") from None
                if code:
                    raise self.diagnostic(f"candidate exited {code} after EOF")
        finally:
            # Also kill descendants when the leader has already exited.
            try:
                os.killpg(self.process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            self.process.wait(timeout=5)
            self.process.stdin.close()
            self.process.stdout.close()
            self.stderr.close()


def local_environment(workspace):
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("AGENT_MEM_", "WORKER_", "OTEL_"))}
    environment.update({"HOME": str(workspace.root), "AGENT_MEM_DATA_DIR": str(workspace.store),
                        "WORKER_API_URL": "", "WORKER_MODEL": "", "WORKER_API_KEY": ""})
    return environment


def local_cases(binary, expected_version, validation_id):
    cases = {}
    version = run([str(binary), "--version"], timeout=20)
    if version != f"mmhmem {expected_version}":
        raise ValueError(f"candidate version mismatch: {version!r}")
    cases["version"] = {"status": "passed", "reported_version": version}
    with ProbeWorkspace() as workspace:
        environment = local_environment(workspace)
        context = {"project": "static-release-probe", "agent_harness": "opencode"}
        marker = "portability" + hashlib.sha256(validation_id.encode()).hexdigest()[:20]
        checkpoint = "Checkpoint " + marker
        narrative = "Durable narrative " + marker
        with McpProcess(binary, environment) as client:
            initialized = client.initialize(expected_version)
            cases["mcp_initialize"] = {"status": "passed", "server_info": initialized["serverInfo"]}
            names = {tool["name"] for tool in client.request("tools/list", {})["tools"]}
            if not TOOLS.issubset(names):
                raise ValueError(f"candidate missing existing MCP tools: {sorted(TOOLS - names)}")
            cases["tools_list"] = {"status": "passed", "tools": sorted(names)}
            recorded = client.tool("mem_record_observation", {**context, "obs_type": "discovery",
                                   "title": marker, "narrative": narrative, "facts": "persistent fixture"})
            match = re.search(r"Recorded observation #(\d+)", recorded)
            if not match:
                raise ValueError(f"observation insertion returned no ID: {recorded}")
            observation_id = int(match[1])
            client.tool("session_resume_or_handoff", {**context, "updated_working_state_md": checkpoint,
                        "client_usage": {"prompt_tokens": 1234, "completion_tokens": 56, "source": "static-probe"}})
        # A second OS process must recover the first process's durable state.
        with McpProcess(binary, environment) as restarted:
            restarted.initialize(expected_version)
            search = restarted.tool("mem_search", {**context, "query": marker})
            if marker not in search or not re.search(rf"\|\s*{observation_id}\s*\|", search):
                raise ValueError(f"persisted observation absent from search: {search}")
            details = restarted.tool("mem_get_observations", {**context, "ids": [observation_id]})
            resumed = restarted.tool("session_resume_or_handoff", context)
            if narrative not in details or checkpoint not in resumed:
                raise ValueError(f"restart lost narrative/checkpoint: {details}\n{resumed}")
        cases["persistence"] = {"status": "passed", "observation_id": observation_id,
                                "marker": marker, "restarted": True}
        table = run([str(binary), "stats"], env=environment, cwd=workspace.root, timeout=20)
        if not all(value in table for value in ("AVOIDED (EST.)", "CLIENT-REPORTED", "TOTAL", context["project"], "opencode")):
            raise ValueError(f"statistics table did not recover recorded session: {table}")
        cases["stats_text"] = {"status": "passed", "output": table}
        statistics = json.loads(run([str(binary), "stats", "--json"], env=environment, cwd=workspace.root, timeout=20))
        validate_statistics(statistics, context["project"])
        cases["stats_json"] = {"status": "passed", "output": statistics}
    return cases


def validate_statistics(statistics, project):
    days, totals = statistics.get("days", []), statistics.get("totals", {})
    if not any(row.get("project") == project and row.get("harness") == "opencode"
               and re.fullmatch(r"\d{4}-\d{2}-\d{2}", row.get("day", "")) for row in days):
        raise ValueError(f"statistics JSON missing recorded session row: {statistics}")
    # Public client_usage creates a row; current CLI aggregates its existing
    # estimate columns (zero for that row), not the usage stored in details.
    for key in ("avoided_input_tokens_est", "avoided_input_tokens_client_reported",
                "worker_prompt_tokens", "worker_completion_tokens"):
        if (type(totals.get(key)) is not int or any(type(row.get(key)) is not int for row in days)
                or totals[key] != sum(row[key] for row in days)):
            raise ValueError(f"statistics JSON has invalid/inconsistent {key}: {statistics}")


def probe(binary, expected_sha256, expected_version, validation_id):
    validate_identity("0" * 40, validation_id)
    binary = Path(binary).resolve(strict=True)
    verify_binary(binary, expected_sha256)
    cases = local_cases(binary, expected_version, validation_id)
    verify_binary(binary, expected_sha256)
    return {"status": "passed", "validation_id": validation_id, "target": TARGET,
            "artifact_sha256": expected_sha256, "package_version": expected_version, "cases": cases}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--expected-sha256", required=True)
    parser.add_argument("--expected-version", required=True)
    parser.add_argument("--validation-id", required=True)
    parser.add_argument("--report", required=True, type=Path)
    args = parser.parse_args()
    try:
        result = probe(args.binary, args.expected_sha256, args.expected_version, args.validation_id)
    except (OSError, ValueError, RuntimeError, KeyError, TypeError) as error:
        args.report.write_text(json.dumps({"status": "failed", "error": str(error)}) + "\n")
        print(f"release probe failed: {error}", file=sys.stderr)
        return 1
    args.report.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result))
    return 0


if __name__ == "__main__":
    sys.exit(main())
