"""Inspect an explicit Linux release file; static PIE metadata is permitted."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import sys

from release_support import TARGET, run


def validate_header(header):
    fields = dict(re.findall(r"^\s*(Class|Machine|Type):\s*(.+)$", header, re.MULTILINE))
    if fields.get("Class") != "ELF64":
        raise ValueError("release must use the ELF64 class")
    if fields.get("Machine") != "Advanced Micro Devices X86-64":
        raise ValueError("release must target x86-64 (AMD64)")
    elf_type = fields.get("Type", "").split()
    if not elf_type or elf_type[0] not in {"EXEC", "DYN"}:
        raise ValueError("release must be an executable ELF (EXEC or static PIE DYN)")
    return {"class": "ELF64", "architecture": "x86_64", "type": elf_type[0]}


def validate_linkage(program, dynamic, versions):
    if re.search(r"^\s*INTERP\s", program, re.MULTILINE):
        raise ValueError("release declares PT_INTERP: a dynamic loader is required")
    if "(NEEDED)" in dynamic:
        raise ValueError("release declares DT_NEEDED: shared libraries are required")
    if "GLIBC_" in versions:
        raise ValueError("release declares GLIBC version requirements")


def inspect(binary, expected_sha256):
    """Return evidence for these exact bytes, or raise a diagnostic failure."""
    if not re.fullmatch(r"[0-9a-f]{64}", expected_sha256):
        raise ValueError("expected SHA-256 must contain exactly 64 lowercase hexadecimal characters")
    binary = Path(binary)
    payload = binary.read_bytes()
    digest = hashlib.sha256(payload).hexdigest()
    if digest != expected_sha256:
        raise ValueError(f"artifact checksum mismatch: expected {expected_sha256}, got {digest}")
    environment = {**os.environ, "LC_ALL": "C"}
    header = run(["readelf", "-hW", str(binary)], env=environment)
    elf = validate_header(header)
    outputs = {"header": header}
    for name, option in (("program", "-lW"), ("dynamic", "-dW"), ("versions", "-VW")):
        outputs[name] = run(["readelf", option, str(binary)], env=environment)
    validate_linkage(outputs["program"], outputs["dynamic"], outputs["versions"])
    if not re.search(r"^\s*LOAD\s", outputs["program"], re.MULTILINE):
        raise ValueError("release has no loadable executable segments")
    # Detect mutation while external inspection tools were reading the file.
    if hashlib.sha256(binary.read_bytes()).hexdigest() != digest:
        raise ValueError("artifact checksum changed during ELF inspection")
    return {
        "status": "passed", "target": TARGET,
        "artifact": {"name": binary.name, "sha256": digest, "size_bytes": len(payload)},
        "elf": elf, "checks": {name: {"status": "passed"} for name in
                                  ("architecture", "elf_class", "interpreter", "shared_libraries", "glibc_versions")},
        "readelf_version": run(["readelf", "--version"], env=environment), "readelf": outputs,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--expected-sha256", required=True)
    parser.add_argument("--report", required=True, type=Path)
    arguments = parser.parse_args()
    if arguments.binary.resolve() == arguments.report.resolve():
        parser.error("report must not overwrite the candidate binary")
    try:
        report = inspect(arguments.binary, arguments.expected_sha256)
    except (OSError, RuntimeError, ValueError) as error:
        report = {"status": "failed", "binary": str(arguments.binary), "error": str(error)}
        print(f"static ELF inspection failed: {error}", file=sys.stderr)
    arguments.report.parent.mkdir(parents=True, exist_ok=True)
    arguments.report.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    sys.exit(main())
