"""Assemble a release manifest only after all same-artifact evidence passes."""

import argparse
import json
from pathlib import Path
import sys

from check_bundle import check_bundle, read_json
from release_support import ARTIFACT_NAME, RUNTIME_NAMES, validate_identity


def assemble(directory, revision, validation_id, source_clean=False):
    root = Path(directory)
    if root.is_symlink() or not root.is_dir():
        raise ValueError("assembly requires an existing non-symlink bundle directory")
    manifest_path, sums_path = root / "verification.json", root / "SHA256SUMS"
    # Revalidation must never leave an earlier success marker after a failure.
    manifest_path.unlink(missing_ok=True)
    sums_path.unlink(missing_ok=True)
    reports = root / "reports"
    reports.mkdir(exist_ok=True)
    try:
        validate_identity(revision, validation_id)
        if type(source_clean) is not bool:
            raise ValueError("source_clean must be an explicit boolean")
        build = read_json(reports / "target-suite.json")
        scratch = read_json(reports / "scratch.json")
        expected_startup = {"status": "passed", "validation_id": validation_id,
                            "artifact_sha256": build["artifact"]["sha256"],
                            "reported_version": "mmhmem " + build["package_version"]}
        if any(scratch.get(key) != value for key, value in expected_startup.items()):
            raise ValueError("scratch startup did not pass with this candidate/version/validation")
        runtimes = []
        for name in RUNTIME_NAMES:
            runtime = read_json(reports / f"{name}.json")
            if runtime.get("status") != "passed" or runtime.get("validation_id") != validation_id:
                raise ValueError(f"{name} runtime did not pass this validation")
            pristine = runtime.get("cases", {}).get("pristine_startup", {})
            if any(pristine.get(key) != value for key, value in expected_startup.items() if key != "validation_id"):
                raise ValueError(f"{name} pristine startup did not pass with this candidate/version")
            runtimes.append({**{key: runtime[key] for key in
                               ("name", "image", "kernel", "architecture", "cpu", "artifact_sha256", "cases")},
                             "report": f"reports/{name}.json"})
        manifest = {key: build[key] for key in
                    ("target", "package_version", "rustc_version", "cargo_version", "lockfile_sha256",
                     "build_image", "native_tools", "artifact")}
        manifest.update(schema_version=1, revision=revision, validation_id=validation_id,
                        source_clean=source_clean, publishable=source_clean, runtimes=runtimes,
                        linkage={"status": "passed", "report": "reports/linkage.json"},
                        regression={"status": "passed", "report": "reports/target-suite.json"},
                        scratch={**scratch, "report": "reports/scratch.json"})
        sums_path.write_text(f"{manifest['artifact']['sha256']}  {ARTIFACT_NAME}\n")
        manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
        check_bundle(root, revision)
    except (OSError, ValueError, KeyError, TypeError) as error:
        manifest_path.unlink(missing_ok=True)
        sums_path.unlink(missing_ok=True)
        (reports / "assembly-failure.json").write_text(json.dumps({"status": "failed", "error": str(error)}) + "\n")
        raise
    (reports / "assembly-failure.json").unlink(missing_ok=True)
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", required=True, type=Path)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--validation-id", required=True)
    parser.add_argument("--source-clean", action="store_true")
    args = parser.parse_args()
    try:
        result = assemble(args.bundle, args.revision, args.validation_id, args.source_clean)
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f"static release assembly failed: {error}", file=sys.stderr)
        return 1
    print(json.dumps({"status": "passed", "revision": result["revision"], "validation_id": result["validation_id"],
                      "publishable": result["publishable"], "artifact": result["artifact"]}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
