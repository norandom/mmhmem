"""Isolated static Linux build infrastructure, shared by release CI gates."""

import asyncio
import json
import re

import dagger
from dagger import dag

RUST_VERSION = "1.98.1"
BUILD_IMAGE = "rust:1.98-slim-bookworm"
TARGET = "x86_64-unknown-linux-musl"


RUNTIME_IMAGES = {"debian-bookworm": "debian:bookworm-slim", "alpine-3.22": "alpine:3.22"}
ARTIFACT_NAME = "mmhmem-linux-x86_64"


def validate_cargo_jobs(cargo_jobs) -> None:
    """Reject a cargo job count below one before any container is built."""
    if type(cargo_jobs) is not int or cargo_jobs < 1:
        raise ValueError(f"cargo_jobs must be an integer of at least 1, got {cargo_jobs!r}")


async def installer_check(src: dagger.Directory, revision: str, validation_id: str,
                          source_clean: bool = False, cargo_jobs: int = 2) -> str:
    validate_cargo_jobs(cargo_jobs)
    tests = await (instrumented_runtime("debian-bookworm", src, validation_id)
                   .with_exec(["python3", "-m", "unittest", "discover", "-s", "ci/scripts", "-p", "test_probe_install.py", "-v"]).stderr())
    bundle = await verified_bundle(src, revision, validation_id, source_clean, cargo_jobs=cargo_jobs)
    manifest = json.loads(await bundle.file("verification.json").contents())
    results = []
    for name in RUNTIME_IMAGES:
        runtime = instrumented_runtime(name, src, validation_id)
        setup = (["apt-get", "install", "-y", "-qq", "bash", "jq"] if name == "debian-bookworm"
                 else ["apk", "add", "--no-cache", "bash", "jq"])
        checked = (runtime.with_exec(setup)
                   .with_file("/verified-candidate", bundle.file(ARTIFACT_NAME), permissions=0o755)
                   .with_exec(["bash", "scripts/test-install.sh", "--candidate", "/verified-candidate",
                               "--expected-sha256", manifest["artifact"]["sha256"], "--report", "/reports/installer.json"]))
        await checked.stdout()
        results.append({"name": name, "installer": json.loads(await checked.file("/reports/installer.json").contents())})
    return json.dumps({"status": "passed", "revision": revision, "validation_id": validation_id,
                       "artifact": manifest["artifact"], "tests": tests, "runtimes": results}, indent=2)


async def gate_tests(src: dagger.Directory, validation_id: str) -> str:
    return await (instrumented_runtime("debian-bookworm", src, validation_id)
                  .with_exec(["python3", "-m", "unittest", "discover", "-s", "ci/scripts", "-p", "test_assemble_bundle.py", "-v"]).stderr())


async def verified_bundle(src: dagger.Directory, revision: str, validation_id: str,
                          source_clean: bool = False, cargo_jobs: int = 2) -> dagger.Directory:
    """One candidate feeds every required check; only final checked bytes escape."""
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("revision must be a full lowercase Git commit ID")
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,127}", validation_id):
        raise ValueError("validation ID must be a nonempty safe label (maximum 128 characters)")
    if type(source_clean) is not bool:
        raise ValueError("source_clean must be an explicit boolean")
    validate_cargo_jobs(cargo_jobs)
    await gate_tests(src, validation_id)
    binary, build_reports, build = await build_candidate(src, validation_id, cargo_jobs=cargo_jobs)
    checksum, version = build["artifact"]["sha256"], build["package_version"]
    # Linkage evidence must retain the release basename, not /candidate.
    inspected = (instrumented_runtime("debian-bookworm", src, validation_id)
                 .with_directory("/bundle/reports", build_reports)
                 .with_file(f"/bundle/{ARTIFACT_NAME}", binary, permissions=0o755)
                 .with_exec(["python3", "ci/scripts/inspect_static.py", "--binary", f"/bundle/{ARTIFACT_NAME}",
                             "--expected-sha256", checksum, "--report", "/bundle/reports/linkage.json"]))
    await inspected.stdout()
    bundle = inspected.directory("/bundle")
    startup = await asyncio.wait_for(startup_stage(binary).with_env_variable("VALIDATION_ID", validation_id).with_exec(["/candidate", "--version"]).stdout(), timeout=30)
    if startup.strip() != f"mmhmem {version}":
        raise ValueError(f"scratch startup version mismatch: {startup!r}")
    scratch = {"status": "passed", "validation_id": validation_id, "artifact_sha256": checksum,
               "reported_version": startup.strip()}
    bundle = bundle.with_new_file("reports/scratch.json", json.dumps(scratch, indent=2) + "\n")
    for name in RUNTIME_IMAGES:
        pristine = await asyncio.wait_for(startup_stage(binary, name).with_env_variable("VALIDATION_ID", validation_id).with_exec(["/candidate", "--version"]).stdout(), timeout=30)
        if pristine.strip() != startup.strip():
            raise ValueError(f"{name} pristine startup version mismatch: {pristine!r}")
        runtime = (instrumented_runtime(name, src, validation_id)
                   .with_file(f"/usr/local/bin/{ARTIFACT_NAME}", binary, permissions=0o755)
                   .with_exec(["python3", "ci/scripts/probe_release.py", "--binary", f"/usr/local/bin/{ARTIFACT_NAME}",
                               "--expected-sha256", checksum, "--expected-version", version,
                               "--validation-id", validation_id, "--mode", "all", "--report", "/reports/probe.json"]))
        probe = json.loads(await runtime.stdout())
        identity = json.loads(await runtime.with_exec(["python3", "-c", "import json,platform; print(json.dumps({'kernel':platform.release(),'architecture':platform.machine(),'cpu':open('/proc/cpuinfo').read().split('\\n\\n')[0]}))"]).stdout())
        probe["cases"]["pristine_startup"] = {"status": "passed", "reported_version": pristine.strip(),
                                                "artifact_sha256": checksum}
        report = {**probe, **identity, "name": name, "image": await pristine_runtime(name).image_ref()}
        bundle = bundle.with_new_file(f"reports/{name}.json", json.dumps(report, indent=2) + "\n")
    command = ["python3", "ci/scripts/assemble_bundle.py", "--bundle", "/bundle", "--revision", revision,
               "--validation-id", validation_id]
    if source_clean:
        command.append("--source-clean")
    assembled = instrumented_runtime("debian-bookworm", src, validation_id).with_directory("/bundle", bundle).with_exec(command)
    # Await execution before returning a Directory; lazy export cannot bypass checks.
    await assembled.stdout()
    checked = assembled.with_exec(["python3", "ci/scripts/check_bundle.py", "--bundle", "/bundle", "--expected-revision", revision])
    await checked.stdout()
    return checked.directory("/bundle")


async def static_gate_check(src: dagger.Directory, revision: str, validation_id: str,
                            source_clean: bool = False) -> str:
    bundle = await verified_bundle(src, revision, validation_id, source_clean)
    tests = await (instrumented_runtime("debian-bookworm", src, validation_id)
                   .with_directory("/verified", bundle).with_env_variable("ACTUAL_BUNDLE", "/verified")
                   .with_exec(["python3", "-m", "unittest", "discover", "-s", "ci/scripts", "-p", "test_assemble_bundle.py", "-v"]).stderr())
    manifest = json.loads(await bundle.file("verification.json").contents())
    return json.dumps({"status": "passed", "revision": revision, "validation_id": validation_id,
                       "publishable": manifest["publishable"], "artifact": manifest["artifact"],
                       "runtimes": manifest["runtimes"], "scratch": manifest["scratch"], "tests": tests}, indent=2)


async def server_probe_check(src: dagger.Directory, validation_id: str) -> str:
    """Run server failures and exercise the same candidate in both userlands."""
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,127}", validation_id):
        raise ValueError("validation ID must be a nonempty safe label (maximum 128 characters)")
    tests = await (instrumented_runtime("debian-bookworm", src, validation_id)
                   .with_exec(["python3", "-m", "unittest", "discover", "-s", "ci/scripts", "-p", "test_probe_release.py", "-v"])
                   .with_exec(["python3", "-m", "unittest", "discover", "-s", "ci/scripts", "-p", "test_probe_server.py", "-v"]).stderr())
    binary, _, build = await build_candidate(src, validation_id)
    checksum = build["artifact"]["sha256"]
    results = []
    for name in RUNTIME_IMAGES:
        runtime = (instrumented_runtime(name, src, validation_id)
                   .with_file("/candidate", binary, permissions=0o755)
                   .with_exec(["python3", "ci/scripts/inspect_static.py", "--binary", "/candidate",
                               "--expected-sha256", checksum, "--report", "/reports/linkage.json"])
                   .with_exec(["python3", "ci/scripts/probe_release.py", "--binary", "/candidate",
                               "--expected-sha256", checksum, "--expected-version", build["package_version"],
                               "--validation-id", validation_id, "--mode", "all", "--report", "/reports/server.json"]))
        results.append({"name": name, "probe": json.loads(await runtime.stdout())})
    return json.dumps({"status": "passed", "validation_id": validation_id, "tests": tests, "runtimes": results}, indent=2)


async def local_probe_check(src: dagger.Directory, validation_id: str) -> str:
    """Run probe failure tests before exercising an internal application file."""
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,127}", validation_id):
        raise ValueError("validation ID must be a nonempty safe label (maximum 128 characters)")
    ctr = instrumented_runtime("debian-bookworm", src, validation_id)
    tests = await ctr.with_exec(["python3", "-m", "unittest", "discover", "-s", "ci/scripts", "-p", "test_probe_release.py", "-v"]).stderr()
    binary, _, build = await build_candidate(src, validation_id)
    checksum = build["artifact"]["sha256"]
    inspected = (musl_container(src).with_env_variable("VALIDATION_ID", validation_id)
                 .with_file("/candidate", binary, permissions=0o755)
                 .with_exec(["python3", "ci/scripts/inspect_static.py", "--binary", "/candidate", "--expected-sha256", checksum, "--report", "/linkage.json"]))
    await inspected.stdout()
    results = []
    for name in RUNTIME_IMAGES:
        runtime = (instrumented_runtime(name, src, validation_id)
                   .with_file("/candidate", binary, permissions=0o755)
                   .with_exec(["python3", "ci/scripts/probe_release.py", "--binary", "/candidate",
                               "--expected-sha256", checksum, "--expected-version", build["package_version"],
                               "--validation-id", validation_id, "--report", "/reports/local.json"]))
        results.append({"name": name, "probe": json.loads(await runtime.stdout())})
    return json.dumps({"status": "passed", "validation_id": validation_id, "tests": tests, "runtimes": results}, indent=2)


async def bundle_check_tests(src: dagger.Directory, validation_id: str) -> str:
    """Run synthetic bundle rejection tests without exporting release evidence."""
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,127}", validation_id):
        raise ValueError("validation ID must be a nonempty safe label (maximum 128 characters)")
    output = await (instrumented_runtime("debian-bookworm", src, validation_id)
                    .with_exec(["python3", "-m", "unittest", "discover", "-s", "ci/scripts", "-p", "test_release_support.py", "-v"])
                    .with_exec(["python3", "-m", "unittest", "discover", "-s", "ci/scripts", "-p", "test_check_bundle.py", "-v"])
                    .stderr())
    return json.dumps({"status": "passed", "validation_id": validation_id, "tests": output})


async def worker_fixture_check(src: dagger.Directory, validation_id: str) -> str:
    """Prove DNS, trust success/rejection and matching worker request receipts."""
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,127}", validation_id):
        raise ValueError("validation ID must be a nonempty safe label (maximum 128 characters)")
    base = instrumented_runtime("debian-bookworm", src, validation_id)
    output = await (base
                    .with_exec(["python3", "-m", "unittest", "discover", "-s", "ci/scripts", "-p", "test_worker_fixture.py", "-v"])
                    .stderr())
    service, trusted_ca, untrusted_ca = worker_service(src, validation_id)
    await service.start()
    try:
        client = base.with_service_binding("worker-fixture", service)
        trusted = with_trusted_root(client, trusted_ca)
        untrusted = with_trusted_root(client, untrusted_ca)
        command = ["python3", "ci/scripts/test_worker_fixture.py", "--endpoint", "https://worker-fixture:8443", "--validation-id", validation_id, "--mode"]
        accepted = json.loads(await trusted.with_exec([*command, "trusted"]).stdout())
        rejected = json.loads(await untrusted.with_exec([*command, "untrusted"]).stdout())
        receipts = json.loads(await trusted.with_exec([*command, "receipts"]).stdout())
        if len(receipts["tls_errors"]) <= accepted["tls_error_count"]:
            raise ValueError("untrusted client produced no fresh server TLS diagnostic")
    finally:
        await service.stop()
    return json.dumps({"status": "passed", "validation_id": validation_id, "tests": output,
                       "trusted": accepted, "untrusted": rejected, "receipts": receipts}, indent=2)


def worker_service(src: dagger.Directory, validation_id: str) -> tuple[dagger.Service, dagger.File, dagger.File]:
    """Return reusable HTTPS service and public roots; private keys stay internal."""
    base = instrumented_runtime("debian-bookworm", src, validation_id)
    certs = base.with_exec(["python3", "-c", "import sys; sys.path.insert(0,'ci/scripts'); from release_support import create_certificates; create_certificates('/probe/worker-ca','worker-fixture',__import__('os').environ['VALIDATION_ID'])"])
    service = certs.with_exposed_port(8443).as_service(args=[
        "python3", "ci/scripts/worker_fixture.py", "--certificate", "/probe/worker-ca/server.crt",
        "--key", "/probe/worker-ca/server.key", "--validation-id", validation_id])
    return service, certs.file("/probe/worker-ca/trusted-ca.crt"), certs.file("/probe/worker-ca/untrusted-ca.crt")


async def inspection_check(src: dagger.Directory, validation_id: str) -> str:
    """Run ELF inspector component tests without exporting an application binary."""
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,127}", validation_id):
        raise ValueError("validation ID must be a nonempty safe label (maximum 128 characters)")
    output = await (musl_container(src).with_env_variable("VALIDATION_ID", validation_id)
                    .with_exec(["python3", "-m", "unittest", "discover", "-s", "ci/scripts", "-p", "test_inspect_static.py", "-v"])
                    .stderr())
    return json.dumps({"status": "passed", "validation_id": validation_id, "tests": output})


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


def musl_container(src: dagger.Directory, cargo_jobs: int = 2) -> dagger.Container:
    """Keep native build scripts and GNU/OTel caches independent of musl.

    cargo_jobs bounds rustc parallelism; the default of 2 keeps the local WSL
    engine within memory (and check_static_setup.py asserts it).
    """
    validate_cargo_jobs(cargo_jobs)
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
        .with_env_variable("CARGO_BUILD_JOBS", str(cargo_jobs))
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


async def candidate_build_check(src: dagger.Directory, validation_id: str) -> str:
    """Produce an internal candidate, returning only its build status to callers."""
    _, _, report = await build_candidate(src, validation_id)
    return json.dumps(report, indent=2)


async def build_candidate(src: dagger.Directory, validation_id: str,
                          cargo_jobs: int = 2) -> tuple[dagger.File, dagger.Directory, dict]:
    """Internal candidate and regression evidence; not a verified release export."""
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,127}", validation_id):
        raise ValueError("validation ID must be a nonempty safe label (maximum 128 characters)")
    ctr = musl_container(src, cargo_jobs=cargo_jobs).with_env_variable("VALIDATION_ID", validation_id)
    tested = ctr.with_exec(["python3", "-m", "unittest", "discover", "-s", "ci/scripts", "-p", "test_build_candidate.py", "-v"])
    candidate = tested.with_exec(["python3", "ci/scripts/build_candidate.py"])
    report = json.loads(await candidate.stdout())
    report["build_image"] = await dag.container(platform=dagger.Platform("linux/amd64")).from_(BUILD_IMAGE).image_ref()
    # Include resolved image identity in the persisted regression report too.
    candidate = candidate.with_new_file("/candidate/reports/target-suite.json", json.dumps(report, indent=2) + "\n")
    return candidate.file("/candidate/mmhmem-linux-x86_64"), candidate.directory("/candidate/reports"), report
