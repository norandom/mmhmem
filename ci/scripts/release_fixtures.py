"""Synthetic schema fixtures for checker/publisher tests; never release evidence."""

import hashlib
import json
from pathlib import Path

from release_support import ARTIFACT_NAME, REQUIRED_CASES, RUNTIME_NAMES, TARGET


def bundle_fixture(payload, revision="a" * 40, validation_id="synthetic-fixture"):
    digest = hashlib.sha256(payload).hexdigest()
    return {
        "schema_version": 1, "revision": revision, "source_clean": True, "publishable": True,
        "validation_id": validation_id, "package_version": "0.2.2", "target": TARGET,
        "rustc_version": "rustc 1.98.1", "cargo_version": "cargo 1.98.1",
        "lockfile_sha256": "b" * 64,
        "build_image": "rust:1.98-slim-bookworm@sha256:" + "c" * 64,
        "native_tools": {"musl-gcc": "fixture compiler", "readelf": "fixture binutils"},
        "artifact": {"name": ARTIFACT_NAME, "sha256": digest, "size_bytes": len(payload)},
        "linkage": {"status": "passed", "report": "reports/linkage.json"},
        "regression": {"status": "passed", "report": "reports/target-suite.json"},
        "runtimes": [
            {"name": name, "image": image + "@sha256:" + "d" * 64,
             "kernel": "synthetic-kernel", "architecture": "x86_64", "cpu": "synthetic CPU",
             "artifact_sha256": digest, "report": f"reports/{name}.json",
             "cases": {case: {"status": "passed"} for case in REQUIRED_CASES}}
            for name, image in zip(RUNTIME_NAMES, ("debian:bookworm-slim", "alpine:3.22"))
        ],
    }


def write_bundle_fixture(directory, payload=b"synthetic bundle fixture, never an executable"):
    """Write coherent synthetic reports for schema/publisher negative tests only."""
    root = Path(directory)
    (root / "reports").mkdir(parents=True, exist_ok=True)
    manifest = bundle_fixture(payload)
    (root / ARTIFACT_NAME).write_bytes(payload)
    (root / "SHA256SUMS").write_text(f"{manifest['artifact']['sha256']}  {ARTIFACT_NAME}\n")
    linkage = {
        "status": "passed", "target": TARGET, "artifact": manifest["artifact"],
        "elf": {"class": "ELF64", "architecture": "x86_64", "type": "DYN"},
        "checks": {name: {"status": "passed"} for name in
                   ("architecture", "elf_class", "interpreter", "shared_libraries", "glibc_versions")},
        "readelf_version": "synthetic readelf", "readelf": {
            "header": " Class: ELF64\n Machine: Advanced Micro Devices X86-64\n Type: DYN (Position-Independent Executable file)",
            "program": "  LOAD 0x000000", "dynamic": "Dynamic section: static PIE metadata",
            "versions": "No version information found in this file.",
        },
    }
    regression = {key: manifest[key] for key in
                  ("target", "artifact", "validation_id", "package_version", "lockfile_sha256",
                   "rustc_version", "cargo_version", "build_image", "native_tools")}
    regression.update(status="passed", reported_version="mmhmem " + manifest["package_version"],
                      optional_telemetry=False, active_dependencies=["mmhmem", "rustls", "aws-lc-rs"],
                      checks={name: {"status": "passed", "report": f"reports/{name}.txt"}
                              for name in ("target-dependencies", "clippy", "tests", "release-build")})
    reports = {"reports/linkage.json": linkage, "reports/target-suite.json": regression}
    for runtime in manifest["runtimes"]:
        reports[runtime["report"]] = {**{key: value for key, value in runtime.items() if key != "report"},
                                      "status": "passed", "validation_id": manifest["validation_id"]}
    for name, report in reports.items():
        (root / name).write_text(json.dumps(report, indent=2) + "\n")
    for record in regression["checks"].values():
        (root / record["report"]).write_text("synthetic successful diagnostic output\n")
    (root / "verification.json").write_text(json.dumps(manifest, indent=2) + "\n")
    return manifest
