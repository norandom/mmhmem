"""Synthetic schema fixtures for checker/publisher tests; never release evidence."""

import hashlib

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
