"""Validate an explicit static-release bundle and its retained evidence."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import sys

from inspect_static import validate_header, validate_linkage
from release_support import ARTIFACT_NAME, REQUIRED_CASES, RUNTIME_NAMES, TARGET, validate_identity


def need(record, key, kind):
    if not isinstance(record, dict) or key not in record or type(record[key]) is not kind:
        raise ValueError(f"missing or invalid {key}: expected {kind.__name__}")
    return record[key]


def nonempty(record, key):
    value = need(record, key, str)
    if not value.strip():
        raise ValueError(f"{key} must be nonempty")
    return value


def sha256(value, label):
    if not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{64}", value):
        raise ValueError(f"{label} must be a lowercase SHA-256 digest")
    return value


def image_digest(value, label, expected=None):
    if not isinstance(value, str) or not re.fullmatch(r"[^\s@]+@sha256:[0-9a-f]{64}", value):
        raise ValueError(f"{label} must include a resolved image digest")
    if expected and value.split("@")[0].removeprefix("docker.io/library/") != expected:
        raise ValueError(f"{label} must identify approved image {expected}")


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON field: {key}")
        result[key] = value
    return result


def bundle_file(root, relative, report=False):
    if not isinstance(relative, str) or not re.fullmatch(r"[A-Za-z0-9_.-]+(?:/[A-Za-z0-9_.-]+)*", relative):
        raise ValueError(f"invalid bundle {'report ' if report else ''}path: {relative!r}")
    parts = relative.split("/")
    if any(part in {".", ".."} for part in parts) or (report and (len(parts) < 2 or parts[0] != "reports")):
        raise ValueError(f"report path must stay inside bundle reports/: {relative}")
    path = root
    for part in parts:
        path = path / part
        if path.is_symlink():
            raise ValueError(f"bundle symlink is not permitted: {relative}")
    if not path.is_file():
        raise ValueError(f"bundle file is missing: {relative}")
    return path


def read_json(path):
    try:
        value = json.loads(path.read_text(), object_pairs_hook=unique_object)
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"cannot read JSON report {path.name}: {error}") from error
    if not isinstance(value, dict):
        raise ValueError(f"JSON report {path.name} must be an object")
    return value


def consistent_state(record, label):
    if "status" in record and record["status"] != "passed":
        raise ValueError(f"{label} did not pass")
    if any(record.get(key) for key in ("error", "errors", "failure", "failures")):
        raise ValueError(f"{label} has contradictory failure evidence")
    if any(key in record and record[key] is not True for key in ("passed", "success")):
        raise ValueError(f"{label} has contradictory success evidence")


def passed(record, label):
    need(record, "status", str)
    consistent_state(record, label)


def artifact_identity(record):
    if need(record, "name", str) != ARTIFACT_NAME:
        raise ValueError("artifact name must be mmhmem-linux-x86_64")
    sha256(need(record, "sha256", str), "artifact sha256")
    if need(record, "size_bytes", int) <= 0:
        raise ValueError("artifact size_bytes must be positive")


def cases_passed(cases, label):
    if not isinstance(cases, dict) or set(cases) != set(REQUIRED_CASES):
        raise ValueError(f"{label} must contain exactly the required runtime cases")
    for name, result in cases.items():
        passed(result, f"{label}/{name}")


def report_identity(report, manifest, *, validation_required=False):
    passed(report, "report")
    for key in ("revision", "target", "validation_id"):
        if key in report and report[key] != manifest[key]:
            raise ValueError(f"report {key} does not match bundle")
    if validation_required and need(report, "validation_id", str) != manifest["validation_id"]:
        raise ValueError("report validation_id does not match bundle")


def linkage_report(report, manifest):
    report_identity(report, manifest)
    if need(report, "target", str) != TARGET or need(report, "artifact", dict) != manifest["artifact"]:
        raise ValueError("linkage report artifact/target mismatch")
    checks = need(report, "checks", dict)
    names = {"architecture", "elf_class", "interpreter", "shared_libraries", "glibc_versions"}
    if set(checks) != names:
        raise ValueError("linkage report is missing required checks")
    for name, result in checks.items():
        passed(result, f"linkage/{name}")
    raw = need(report, "readelf", dict)
    elf = validate_header(nonempty(raw, "header"))
    if need(report, "elf", dict) != elf:
        raise ValueError("linkage ELF evidence contradicts its header")
    validate_linkage(nonempty(raw, "program"), nonempty(raw, "dynamic"), nonempty(raw, "versions"))
    if not re.search(r"^\s*LOAD\s", raw["program"], re.MULTILINE):
        raise ValueError("linkage report has no LOAD segment")
    nonempty(report, "readelf_version")


def regression_report(root, report, manifest):
    report_identity(report, manifest, validation_required=True)
    for key in ("target", "artifact", "package_version", "lockfile_sha256", "rustc_version", "cargo_version", "build_image", "native_tools"):
        if key not in report or report[key] != manifest[key]:
            raise ValueError(f"regression report {key} mismatch")
    if need(report, "optional_telemetry", bool) is not False:
        raise ValueError("regression report enabled optional telemetry")
    if need(report, "reported_version", str) != "mmhmem " + manifest["package_version"]:
        raise ValueError("regression report executable version mismatch")
    dependencies = need(report, "active_dependencies", list)
    if (any(not isinstance(name, str) for name in dependencies)
            or not {"mmhmem", "rustls"}.issubset(dependencies)
            or any(name.startswith("opentelemetry") or name in {"native-tls", "openssl", "openssl-sys"} for name in dependencies)):
        raise ValueError("regression report has invalid plain-target dependencies")
    checks = need(report, "checks", dict)
    if set(checks) != {"target-dependencies", "clippy", "tests", "release-build"}:
        raise ValueError("regression report is missing required checks")
    for name, result in checks.items():
        passed(result, f"regression/{name}")
        if not bundle_file(root, need(result, "report", str), report=True).read_text().strip():
            raise ValueError(f"regression/{name} diagnostic report is empty")


def check_bundle(directory, expected_revision, *, require_publishable=False):
    root = Path(directory)
    if root.is_symlink() or not root.is_dir():
        raise ValueError("bundle must be an existing directory, not a symlink")
    root = root.resolve()
    manifest = read_json(bundle_file(root, "verification.json"))
    consistent_state(manifest, "bundle")
    if need(manifest, "schema_version", int) != 1:
        raise ValueError("unsupported bundle schema_version")
    validate_identity(need(manifest, "revision", str), need(manifest, "validation_id", str))
    validate_identity(expected_revision, manifest["validation_id"])
    if manifest["revision"] != expected_revision:
        raise ValueError("bundle revision does not match expected revision")
    if need(manifest, "target", str) != TARGET:
        raise ValueError("bundle target must be x86_64-unknown-linux-musl")
    clean, publishable = need(manifest, "source_clean", bool), need(manifest, "publishable", bool)
    if publishable and not clean:
        raise ValueError("dirty source cannot be publishable")
    if require_publishable and not publishable:
        raise ValueError("bundle is not publishable")
    nonempty(manifest, "package_version")
    for key, prefix in (("rustc_version", "rustc"), ("cargo_version", "cargo")):
        if not re.match(rf"^{prefix} 1\.98\.1(?:\s|$)", nonempty(manifest, key)):
            raise ValueError(f"{key} must identify the approved 1.98.1 toolchain")
    sha256(need(manifest, "lockfile_sha256", str), "lockfile_sha256")
    image_digest(need(manifest, "build_image", str), "build_image", "rust:1.98-slim-bookworm")
    tools = need(manifest, "native_tools", dict)
    nonempty(tools, "musl-gcc")
    nonempty(tools, "readelf")
    artifact = need(manifest, "artifact", dict)
    artifact_identity(artifact)
    binary = bundle_file(root, ARTIFACT_NAME).read_bytes()
    if len(binary) != artifact["size_bytes"] or hashlib.sha256(binary).hexdigest() != artifact["sha256"]:
        raise ValueError("artifact bytes do not match manifest checksum/size")
    sums = bundle_file(root, "SHA256SUMS").read_text()
    if sums != f"{artifact['sha256']}  {ARTIFACT_NAME}\n":
        raise ValueError("SHA256SUMS must contain exactly the verified artifact checksum and name")
    report_paths = set()

    def load_report(record, label):
        passed(record, label)
        relative = need(record, "report", str)
        if relative in report_paths:
            raise ValueError(f"duplicate report reference: {relative}")
        report_paths.add(relative)
        return read_json(bundle_file(root, relative, report=True))

    linkage_report(load_report(need(manifest, "linkage", dict), "linkage"), manifest)
    regression_report(root, load_report(need(manifest, "regression", dict), "regression"), manifest)
    runtimes = need(manifest, "runtimes", list)
    if len(runtimes) != len(RUNTIME_NAMES):
        raise ValueError("bundle must contain exactly the required runtime records")
    names = [need(runtime, "name", str) for runtime in runtimes]
    if set(names) != set(RUNTIME_NAMES):
        raise ValueError("missing, unknown or duplicate runtime record")
    for runtime in runtimes:
        consistent_state(runtime, "runtime")
        expected_image = {"debian-bookworm": "debian:bookworm-slim", "alpine-3.22": "alpine:3.22"}[runtime["name"]]
        image_digest(nonempty(runtime, "image"), "runtime image", expected_image)
        for key in ("kernel", "cpu"):
            nonempty(runtime, key)
        if need(runtime, "architecture", str) != "x86_64":
            raise ValueError("runtime architecture must be x86_64")
        if need(runtime, "artifact_sha256", str) != artifact["sha256"]:
            raise ValueError("runtime artifact checksum mismatch")
        cases_passed(need(runtime, "cases", dict), runtime["name"])
        # The manifest runtime record owns image/cases; its report substantiates them.
        report = load_report({"status": "passed", "report": need(runtime, "report", str)}, runtime["name"])
        report_identity(report, manifest, validation_required=True)
        for key in ("name", "image", "kernel", "architecture", "cpu", "artifact_sha256", "cases"):
            if key not in report or report[key] != runtime[key]:
                raise ValueError(f"runtime report {key} contradicts manifest")
        cases_passed(report["cases"], runtime["name"] + " report")
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", required=True, type=Path)
    parser.add_argument("--expected-revision", required=True)
    parser.add_argument("--require-publishable", action="store_true")
    args = parser.parse_args()
    try:
        manifest = check_bundle(args.bundle, args.expected_revision, require_publishable=args.require_publishable)
    except (OSError, ValueError) as error:
        print(f"release bundle rejected: {error}", file=sys.stderr)
        return 1
    print(json.dumps({"status": "passed", "revision": manifest["revision"],
                      "validation_id": manifest["validation_id"], "publishable": manifest["publishable"],
                      "artifact": manifest["artifact"]}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
