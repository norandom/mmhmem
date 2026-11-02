"""Exercise the isolated musl compiler and inspect a disposable ELF fixture."""

import json
import os
import subprocess
import tempfile
from pathlib import Path


def run(*args):
    result = subprocess.run(args, capture_output=True, text=True, timeout=120)
    if result.returncode:
        raise RuntimeError(f"{args!r}: {result.stderr}")
    return result.stdout.strip()


def assert_static(binary):
    headers = run("readelf", "-hW", str(binary))
    program = run("readelf", "-lW", str(binary))
    dynamic = run("readelf", "-dW", str(binary))
    versions = run("readelf", "-VW", str(binary))
    assert "ELF64" in headers and "Advanced Micro Devices X86-64" in headers, headers
    assert "INTERP" not in program, program
    assert "(NEEDED)" not in dynamic, dynamic
    assert "GLIBC_" not in versions, versions


def main():
    compiler = run("rustc", "--version")
    assert compiler.startswith("rustc 1.98.1 "), compiler
    assert os.environ["CARGO_INCREMENTAL"] == "0"
    assert os.environ["CARGO_BUILD_JOBS"] == "2"
    assert "RUSTFLAGS" not in os.environ
    target = "x86_64-unknown-linux-musl"
    flags = os.environ["CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS"].split()
    with tempfile.TemporaryDirectory(prefix="musl-setup-") as temporary:
        root = Path(temporary)
        source = root / "fixture.rs"
        source.write_text('fn main() { println!("static setup fixture"); }\n')
        binary = root / "fixture"
        run("rustc", "--target", target, "-C", "linker=musl-gcc", *flags, str(source), "-o", str(binary))
        assert_static(binary)
        assert run(str(binary)) == "static setup fixture"
        # A GNU fixture must fail the same acceptance assertions.
        dynamic = root / "dynamic"
        run("rustc", str(source), "-o", str(dynamic))
        try:
            assert_static(dynamic)
        except AssertionError:
            pass
        else:
            raise AssertionError("dynamic fixture was incorrectly accepted")
    print(json.dumps({
        "status": "passed", "target": target, "rustc_version": compiler,
        "cargo_version": run("cargo", "--version"),
        "linker_version": run("musl-gcc", "--version"),
        "native_packages": run("dpkg-query", "-W", "musl", "musl-tools", "gcc", "binutils"),
        "musl_linker_specs": Path("/usr/lib/x86_64-linux-musl/musl-gcc.specs").read_text(),
        "fixture": "static ELF64 x86-64; dynamic negative rejected",
    }))


if __name__ == "__main__":
    main()
