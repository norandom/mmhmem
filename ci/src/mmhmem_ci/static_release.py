"""Isolated static Linux build infrastructure, shared by release CI gates."""

import json
import re

import dagger
from dagger import dag

RUST_VERSION = "1.98.1"
BUILD_IMAGE = "rust:1.98-slim-bookworm"
TARGET = "x86_64-unknown-linux-musl"


RUNTIME_IMAGES = {"debian-bookworm": "debian:bookworm-slim", "alpine-3.22": "alpine:3.22"}


def pristine_runtime(name: str) -> dagger.Container:
    """An untouched distribution image, without probe or compatibility packages."""
    return dag.container(platform=dagger.Platform("linux/amd64")).from_(RUNTIME_IMAGES[name])


def startup_stage(binary: dagger.File, name: str = "scratch") -> dagger.Container:
    """Only the candidate is added; startup cannot depend on instrumentation."""
    base = dag.container(platform=dagger.Platform("linux/amd64")) if name == "scratch" else pristine_runtime(name)
    return base.with_file("/candidate", binary, permissions=0o755)


def instrumented_runtime(name: str, src: dagger.Directory, validation_id: str) -> dagger.Container:
    """Probe dependencies are deliberately separate from pristine startup."""
    ctr = pristine_runtime(name)
    packages = ["python3", "binutils", "openssl", "ca-certificates"]
    if name == "debian-bookworm":
        ctr = ctr.with_exec(["apt-get", "update", "-qq"]).with_exec(["apt-get", "install", "-y", "-qq", *packages])
    else:
        ctr = ctr.with_exec(["apk", "add", "--no-cache", *packages])
    return (ctr.with_directory("/src", src).with_workdir("/src")
            .with_env_variable("VALIDATION_ID", validation_id)
            .with_exec(["mkdir", "-p", "/probe", "/reports"]))


def with_trusted_root(ctr: dagger.Container, certificate: dagger.File) -> dagger.Container:
    """Both supported distributions use update-ca-certificates for local roots."""
    return (ctr.with_file("/usr/local/share/ca-certificates/mmhmem-fixture.crt", certificate)
            .with_exec(["update-ca-certificates"]))


async def runtime_setup_check(src: dagger.Directory, validation_id: str) -> str:
    """Exercise runtime prerequisites with a static fixture, never application bytes."""
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,127}", validation_id):
        raise ValueError("validation ID must be a nonempty safe label (maximum 128 characters)")
    fixture = (musl_container(src)
               .with_new_file("/fixture.rs", 'fn main() { println!("runtime infrastructure fixture"); }')
               .with_exec(["sh", "-c", 'rustc --target x86_64-unknown-linux-musl -C linker=musl-gcc $CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS /fixture.rs -o /fixture'])
               .file("/fixture"))
    scratch = await startup_stage(fixture).with_env_variable("VALIDATION_ID", validation_id).with_exec(["/candidate"]).stdout()
    if scratch.strip() != "runtime infrastructure fixture":
        raise ValueError("scratch startup did not execute the static fixture")
    results = []
    for name in RUNTIME_IMAGES:
        image_ref = await pristine_runtime(name).image_ref()
        startup = await startup_stage(fixture, name).with_env_variable("VALIDATION_ID", validation_id).with_exec(["/candidate"]).stdout()
        if startup.strip() != scratch.strip():
            raise ValueError(f"{name} pristine startup differs from scratch")
        ctr = instrumented_runtime(name, src, validation_id)
        # Tests include fresh SAN certificates and accepted/rejected CA verification.
        await ctr.with_exec(["python3", "-m", "unittest", "discover", "-s", "ci/scripts", "-p", "test_release_support.py", "-v"]).stdout()
        certs = ctr.with_exec(["python3", "-c", "import sys; sys.path.insert(0,'ci/scripts'); from release_support import create_certificates; create_certificates('/probe/ca','worker-fixture',__import__('os').environ['VALIDATION_ID'])"])
        trusted = with_trusted_root(certs, certs.file("/probe/ca/trusted-ca.crt"))
        await trusted.with_exec(["openssl", "verify", "-verify_hostname", "worker-fixture", "/probe/ca/server.crt"]).stdout()
        identity = json.loads(await ctr.with_exec(["python3", "-c", "import json,platform; print(json.dumps({'kernel':platform.release(),'architecture':platform.machine(),'cpu':open('/proc/cpuinfo').read().split('\\n\\n')[0]}))"]).stdout())
        results.append({"name": name, "image": image_ref, **identity, "status": "passed"})
    return json.dumps({"status": "passed", "validation_id": validation_id, "scratch_startup": "passed", "runtimes": results}, indent=2)


def musl_container(src: dagger.Directory) -> dagger.Container:
    """Keep native build scripts and GNU/OTel caches independent of musl."""
    return (
        dag.container(platform=dagger.Platform("linux/amd64"))
        .from_(BUILD_IMAGE)
        .with_exec(["apt-get", "update", "-qq"])
        .with_exec(["apt-get", "install", "-y", "-qq", "musl-tools", "gcc", "libc6-dev", "binutils", "python3"])
        .with_exec(["rustup", "toolchain", "install", RUST_VERSION, "--profile", "minimal", "--component", "clippy"])
        .with_exec(["rustup", "target", "add", "--toolchain", RUST_VERSION, TARGET])
        .with_env_variable("RUSTUP_TOOLCHAIN", RUST_VERSION)
        .with_env_variable("CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER", "musl-gcc")
        .with_env_variable("CC_x86_64_unknown_linux_musl", "musl-gcc")
        # Debian's musl-gcc specs inject an interpreter for -static-pie.
        # Keep PIE, but explicitly suppress that loader declaration.
        .with_env_variable("CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS", "-C target-feature=+crt-static -C target-cpu=x86-64 -C link-arg=-Wl,--no-dynamic-linker")
        .with_env_variable("CARGO_INCREMENTAL", "0")
        .with_env_variable("CARGO_BUILD_JOBS", "2")
        .with_env_variable("CARGO_TARGET_DIR", "/cargo-target-musl")
        .with_mounted_cache("/usr/local/cargo/registry", dag.cache_volume("mmhmem-cargo-registry"))
        .with_mounted_cache("/usr/local/cargo/git", dag.cache_volume("mmhmem-cargo-git"))
        .with_mounted_cache("/cargo-target-musl", dag.cache_volume("mmhmem-cargo-target-musl-1.98.1"))
        .with_mounted_directory("/src", src.without_file("rust-toolchain.toml"))
        .with_workdir("/src")
    )


async def setup_check(src: dagger.Directory) -> str:
    """Compile/inspect fixtures and return status and identities, never a File."""
    image = dag.container(platform=dagger.Platform("linux/amd64")).from_(BUILD_IMAGE)
    image_ref = await image.image_ref()
    output = await musl_container(src).with_exec(["python3", "ci/scripts/check_static_setup.py"]).stdout()
    report = json.loads(output)
    report["build_image"] = image_ref
    return json.dumps(report, indent=2)
