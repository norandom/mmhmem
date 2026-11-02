"""Isolated static Linux build infrastructure, shared by release CI gates."""

import json

import dagger
from dagger import dag

RUST_VERSION = "1.98.1"
BUILD_IMAGE = "rust:1.98-slim-bookworm"
TARGET = "x86_64-unknown-linux-musl"


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
