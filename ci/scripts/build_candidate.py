"""Build the locked plain musl candidate and retain target regression evidence."""

import hashlib
import json
import os
from pathlib import Path
import shutil
import sys
import tomllib

from release_support import ARTIFACT_NAME, TARGET, run, validate_identity


def candidate_identity(binary):
    binary = Path(binary)
    payload = binary.read_bytes()
    return {"name": binary.name, "sha256": hashlib.sha256(payload).hexdigest(), "size_bytes": len(payload)}


def validate_dependencies(graph):
    names = sorted({line.split()[0] for line in graph.splitlines() if line.strip()})
    forbidden = [name for name in names if name.startswith("opentelemetry") or name in {"native-tls", "openssl", "openssl-sys"}]
    if forbidden:
        raise ValueError(f"plain musl target has forbidden active dependencies: {', '.join(forbidden)}")
    if "mmhmem" not in names or "rustls" not in names:
        raise ValueError("active dependency graph is missing mmhmem or its existing rustls provider")
    return names


def cargo_phase(name, arguments, reports):
    print(f"static builder: {name}", file=sys.stderr, flush=True)
    try:
        output = run(["sh", "-c", 'exec "$@" 2>&1', "--", "cargo", *arguments], timeout=1200)
    except RuntimeError as error:
        (reports / f"{name}.txt").write_text(str(error) + "\n")
        raise RuntimeError(f"static musl {name} failed: {error}") from error
    (reports / f"{name}.txt").write_text(output + "\n")
    return output


def build():
    validation_id = os.environ["VALIDATION_ID"]
    validate_identity("0" * 40, validation_id)
    output = Path("/candidate")
    reports = output / "reports"
    reports.mkdir(parents=True, exist_ok=True)
    lockfile_hash = hashlib.sha256(Path("Cargo.lock").read_bytes()).hexdigest()
    graph = cargo_phase("target-dependencies", ["tree", "--locked", "--target", TARGET, "-e", "normal", "--prefix", "none", "--format", "{p}"], reports)
    dependencies = validate_dependencies(graph)
    cargo_phase("clippy", ["clippy", "--locked", "--target", TARGET, "--all-targets", "--", "-D", "warnings"], reports)
    cargo_phase("tests", ["test", "--locked", "--target", TARGET], reports)
    cargo_phase("release-build", ["build", "--release", "--locked", "--target", TARGET], reports)
    binary = output / ARTIFACT_NAME
    shutil.copy2(Path(os.environ["CARGO_TARGET_DIR"]) / TARGET / "release" / "mmhmem", binary)
    version = tomllib.loads(Path("Cargo.toml").read_text())["package"]["version"]
    reported_version = run([str(binary), "--version"], timeout=20)
    if reported_version != f"mmhmem {version}":
        raise ValueError(f"candidate version mismatch: expected {version}, got {reported_version!r}")
    if lockfile_hash != hashlib.sha256(Path("Cargo.lock").read_bytes()).hexdigest():
        raise ValueError("locked build modified Cargo.lock")
    report = {
        "status": "passed", "validation_id": validation_id, "target": TARGET,
        "package_version": version, "reported_version": reported_version,
        "artifact": candidate_identity(binary), "lockfile_sha256": lockfile_hash,
        "rustc_version": run(["rustc", "--version"]), "cargo_version": run(["cargo", "--version"]),
        "native_tools": {"musl-gcc": run(["musl-gcc", "--version"]), "readelf": run(["readelf", "--version"])},
        "active_dependencies": dependencies, "optional_telemetry": False,
        "checks": {name: {"status": "passed", "report": f"reports/{name}.txt"}
                   for name in ("target-dependencies", "clippy", "tests", "release-build")},
    }
    (reports / "target-suite.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))


if __name__ == "__main__":
    build()
