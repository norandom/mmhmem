"""mmhmem CI pipeline (task 14.1; Req 13.5/13.6, 11.2, 8.6, 8.7).

One Dagger module runs everywhere: locally via the WSL dagger CLI
(`cd ci && dagger call all --src ..`) and in GitHub Actions
(.github/workflows/ci.yml) — never two scripts (dev-environment steering).

Blocking gates: fmt, glibc clippy `--features otel` (the Windows lesson:
gated code escapes plain clippy); default-feature clippy and the
deterministic test suite (including tests/loc_caps.rs, which
machine-enforces the Req 13.3 size caps) run on musl inside static_release.
The public `clippy` and `test` functions keep the glibc passes for local use
(ci-build-time Decision 3).
Informational: tokei per-file LOC report (dev tool only, Req 13.5) runs in
its own non-blocking workflow job; `loc_report` stays callable locally
(ci-dependency-cache 4.5). Hosted runs pass the GitHub cache credentials so
every Rust build runs under sccache (ci-dependency-cache); local runs omit
them and build exactly as before.
Validation: release build (plain, zero otel crates), Docker image built with
`--features otel` (Req 11.2/8.6) whose payload runs, `docker compose config`
on both compose files (Req 8.6), and the Linux deploy artifacts
(systemd-analyze verify + logrotate --debug, Req 8.1/8.2) — the pristine
versions of the checks that DrvFS/sudo limitations only approximate locally.

NOTE: every function that reads container output awaits `.stdout()` — in the
Python SDK the leaf getters are coroutines; a sync function returning one
fails at runtime with FunctionError.
"""

import json
import time

import dagger
from dagger import dag, function, object_type

from .sccache import OFF, STATS_PATH, CacheConfig, cached_exec, off_record, with_cache
from .stages import Stage, check_memory, run_stages, timings_line

RUST_IMAGE = "rust:1.98-slim-bookworm"

# The `all` stage list (design "`all` stage list" table): (name, heavy, needs),
# in list order. Heavy stages count toward the memory guard. The glibc
# default-feature clippy and test passes are deliberately absent (Decision 3).
# The tokei LOC report left the gate for its own workflow job
# (ci-dependency-cache 4.5).
ALL_STAGE_PLAN = (
    ("static_release", True, ()),
    ("docker_build", True, ()),
    ("clippy_otel", True, ()),
    ("stage_runner_tests", False, ()),
    ("distribution", False, ()),
    ("fmt", False, ()),
    ("compose_config", False, ()),
    ("static_installer", False, ("static_release",)),
    ("deploy_artifacts", False, ("static_release",)),
)

# Prefix of the gate's compile-cache line, read by ci/scripts/timing_summary.py.
CACHE_PREFIX = "== cache: "

# `== <label>: ok` summary lines.
STAGE_LABELS = {
    "static_release": "static_release (musl + same-artifact runtime gate)",
    "docker_build": "docker_build (otel)",
    "clippy_otel": "clippy_otel (glibc --features otel)",
    "stage_runner_tests": "stage_runner_tests (ci stage runner unit tests)",
    "distribution": "distribution (Debian + Alpine)",
    "fmt": "fmt",
    "compose_config": "compose_config (prod + e2e)",
    "static_installer": "static_installer (verified candidate install + update)",
    "deploy_artifacts": "deploy_artifacts (systemd + logrotate)",
}

# Reads the engine VM's memory (design "Memory guard").
MEMORY_PROBE_IMAGE = "alpine:3.22"


def validate_concurrency(concurrency) -> None:
    """Reject a concurrency below one before any container is built."""
    if type(concurrency) is not int or concurrency < 1:
        raise ValueError(f"concurrency must be an integer of at least 1, got {concurrency!r}")


def parse_mem_total_kib(meminfo: str) -> int:
    """Return MemTotal in KiB from /proc/meminfo text."""
    for line in meminfo.splitlines():
        fields = line.split()
        if len(fields) >= 2 and fields[0] == "MemTotal:":
            return int(fields[1])
    raise ValueError("engine /proc/meminfo has no MemTotal line")


def all_summary(results, cache_stats) -> str:
    """Per-stage ok lines, the verified bundle manifest, the compile-cache
    line, then the timings line."""
    outputs = {result.name: result.output for result in results}
    summary = "\n".join(f"== {STAGE_LABELS[r.name]}: ok" for r in results if r.name in STAGE_LABELS)
    cache_line = CACHE_PREFIX + json.dumps(cache_stats, separators=(",", ":"), sort_keys=True)
    return (f"{summary}\n== verified Linux bundle:\n{outputs['static_release']}\n"
            f"{cache_line}\n{timings_line(results)}")

# The repo mount keeps the rustup toolchain pin out (its rust-analyzer
# component is a dev-host concern; the image's 1.98 toolchain is used as-is).
TOOLCHAIN_PIN = "rust-toolchain.toml"


def rust_container(src: dagger.Directory, cargo_jobs: int = 2, cache: CacheConfig = OFF) -> dagger.Container:
    """Rust 1.98 container with the repo mounted and persistent cargo caches.

    Caches mount the registry/git/target dirs, NOT all of CARGO_HOME: env
    vars are container-global in dagger, so overriding CARGO_HOME would also
    hide the rustup proxies (cargo-fmt/clippy-driver) from PATH.
    cargo_jobs bounds rustc parallelism (default 2: the local memory guard).
    """
    from .static_release import validate_cargo_jobs

    validate_cargo_jobs(cargo_jobs)
    toolchain = (
        dag.container()
        .from_(RUST_IMAGE)
        .with_exec(["apt-get", "update", "-qq"])
        .with_exec(["apt-get", "install", "-y", "-qq", "gcc", "libc6-dev"])
        # The slim image ships no rustfmt/clippy components.
        .with_exec(["rustup", "component", "add", "rustfmt", "clippy"])
        .with_mounted_cache(
            "/usr/local/cargo/registry", dag.cache_volume("mmhmem-cargo-registry")
        )
        .with_mounted_cache("/usr/local/cargo/git", dag.cache_volume("mmhmem-cargo-git"))
        .with_mounted_cache("/cargo-target", dag.cache_volume("mmhmem-cargo-target"))
        .with_env_variable("CARGO_TARGET_DIR", "/cargo-target")
        # One-shot CI builds: no incremental state, bounded rustc parallelism
        # (the dev-host WSL engine VM has ~6 GB RAM; unbounded cargo builds
        # OOM-kill the dagger engine mid-run — observed twice).
        .with_env_variable("CARGO_INCREMENTAL", "0")
        .with_env_variable("CARGO_BUILD_JOBS", str(cargo_jobs))
    )
    # An enabled cache adds sccache before the sources; OFF changes nothing.
    return (with_cache(toolchain, cache)
            .with_mounted_directory("/src", src.without_file(TOOLCHAIN_PIN))
            .with_workdir("/src"))


async def _clippy_otel(src: dagger.Directory, cargo_jobs: int = 2, cache: CacheConfig = OFF) -> tuple:
    """The glibc `--features otel` clippy pass: compiles the feature-gated code.

    Returns (output, compile-cache stats record)."""
    clippy = ["cargo", "clippy", "--all-targets", "--features", "otel", "--", "-D", "warnings"]
    ctr = rust_container(src, cargo_jobs=cargo_jobs, cache=cache)
    if cache.mode == "off":
        return await ctr.with_exec(clippy).stdout(), off_record()
    done = ctr.with_exec(cached_exec(clippy))
    output = await done.stdout()
    return output, json.loads(await done.file(STATS_PATH).contents())


async def _stage_runner_tests(src: dagger.Directory, validation_id: str) -> str:
    """Run the stage runner's unit tests in the Debian bookworm runtime (Python 3.11).

    The full source mount keeps the ci/scripts and ci/src/mmhmem_ci layout
    that test_stages.py resolves the module from.
    """
    from .static_release import instrumented_runtime

    return await (instrumented_runtime("debian-bookworm", src, validation_id)
                  .with_exec(["python3", "-m", "unittest", "discover", "-s", "ci/scripts", "-p", "test_stages.py", "-v"])
                  .stderr())


@object_type
class MmhmemCi:
    @function
    async def static_installer(self, src: dagger.Directory, revision: str, validation_id: str,
                               source_clean: bool = False, cargo_jobs: int = 2, cache_mode: str = "off",
                               cache_url: dagger.Secret | None = None,
                               cache_token: dagger.Secret | None = None) -> str:
        """Install/update the verified candidate in isolated supported userlands."""
        from .static_release import installer_check

        cache = CacheConfig.from_options(cache_mode, cache_url, cache_token)
        return await installer_check(src, revision, validation_id, source_clean, cargo_jobs=cargo_jobs, cache=cache)

    @function
    async def static_release(self, src: dagger.Directory, revision: str, validation_id: str,
                             source_clean: bool = False, cargo_jobs: int = 2, cache_mode: str = "off",
                             cache_url: dagger.Secret | None = None,
                             cache_token: dagger.Secret | None = None) -> dagger.Directory:
        """Return only the same-artifact verified Linux bundle. The workflow's
        export step must pass the same cargo_jobs and cache options as the
        gate so the engine reuses the in-session build."""
        from .static_release import verified_bundle

        cache = CacheConfig.from_options(cache_mode, cache_url, cache_token)
        return await verified_bundle(src, revision, validation_id, source_clean, cargo_jobs=cargo_jobs, cache=cache)

    @function
    async def static_gate(self, src: dagger.Directory, revision: str, validation_id: str,
                          source_clean: bool = False) -> str:
        """Exercise successful bundle assembly and fail-closed error injection."""
        from .static_release import static_gate_check

        return await static_gate_check(src, revision, validation_id, source_clean)

    @function
    async def static_gate_tests(self, src: dagger.Directory, validation_id: str) -> str:
        """Check fail-closed bundle assembly before invoking the integrated gate."""
        from .static_release import gate_tests

        return await gate_tests(src, validation_id)

    @function
    async def server_probe(self, src: dagger.Directory, validation_id: str) -> str:
        """Check candidate server startup, health and bearer authentication."""
        from .static_release import server_probe_check

        return await server_probe_check(src, validation_id)

    @function
    async def local_probe(self, src: dagger.Directory, validation_id: str) -> str:
        """Exercise the explicit static candidate's local public interfaces."""
        from .static_release import local_probe_check

        return await local_probe_check(src, validation_id)

    @function
    async def bundle_check(self, src: dagger.Directory, validation_id: str) -> str:
        """Exercise bundle schema and evidence consistency rejection checks."""
        from .static_release import bundle_check_tests

        return await bundle_check_tests(src, validation_id)

    @function
    async def worker_fixture(self, src: dagger.Directory, validation_id: str) -> str:
        """Check deterministic worker HTTPS behavior through service DNS."""
        from .static_release import worker_fixture_check

        return await worker_fixture_check(src, validation_id)

    @function
    async def static_inspection(self, src: dagger.Directory, validation_id: str) -> str:
        """Exercise standalone ELF acceptance against static and incompatible files."""
        from .static_release import inspection_check

        return await inspection_check(src, validation_id)

    @function
    async def static_build(self, src: dagger.Directory, validation_id: str) -> str:
        """Status-only candidate build; full runtime verification precedes export."""
        from .static_release import candidate_build_check

        return await candidate_build_check(src, validation_id)

    @function
    async def runtime_setup(self, src: dagger.Directory, validation_id: str) -> str:
        """Status-only check; callers must supply a fresh safe validation ID."""
        from .static_release import runtime_setup_check

        return await runtime_setup_check(src, validation_id)

    @function
    async def static_setup(self, src: dagger.Directory) -> str:
        """Status-only musl toolchain check; never exports a release candidate."""
        from .static_release import setup_check

        return await setup_check(src)

    @function
    async def distribution(self, src: dagger.Directory) -> str:
        """Installer and marketplace checks on GNU and BusyBox Linux userlands."""
        results = []
        for image, setup in [
            ("debian:bookworm-slim", ["sh", "-c", "apt-get update -qq && apt-get install -y -qq bash jq python3 python3-yaml"]),
            ("alpine:3.22", ["apk", "add", "--no-cache", "bash", "jq", "python3", "py3-yaml"]),
        ]:
            output = await (
                dag.container().from_(image)
                .with_exec(setup)
                .with_directory("/src", src)
                .with_workdir("/src")
                .with_exec(["bash", "scripts/test-install.sh"])
                .with_exec(["bash", "scripts/test-routing-instructions.sh"])
                .with_exec(["python3", "scripts/check-marketplace.py"])
                .stdout()
            )
            results.append(f"{image}: {output}")
        return "\n".join(results)

    @function
    async def fmt(self, src: dagger.Directory) -> str:
        """Blocking: rustfmt check (Req 13.5)."""
        return await rust_container(src).with_exec(["cargo", "fmt", "--check"]).stdout()

    @function
    async def clippy(self, src: dagger.Directory) -> str:
        """Blocking: clippy -D warnings on default AND `otel` features.

        The cognitive-complexity cap (Req 13.1/13.2) comes from clippy.toml +
        [lints.clippy]; the otel pass is what actually compiles the
        feature-gated code that plain clippy never sees.
        """
        ctr = rust_container(src)
        plain = await ctr.with_exec(
            ["cargo", "clippy", "--all-targets", "--", "-D", "warnings"]
        ).stdout()
        otel = await ctr.with_exec(
            ["cargo", "clippy", "--all-targets", "--features", "otel", "--", "-D", "warnings"]
        ).stdout()
        return f"clippy (default): {plain}\nclippy (otel): {otel}"

    @function
    async def test(self, src: dagger.Directory) -> str:
        """Blocking: the deterministic suite (97+ tests, worker stubbed).

        Includes tests/loc_caps.rs, which machine-enforces Req 13.3
        (<=500 LOC/src file, <=80 LOC/fn, <=800 LOC/test file).
        """
        return await rust_container(src).with_exec(["cargo", "test"]).stdout()

    @function
    async def loc_report(self, src: dagger.Directory) -> str:
        """Informational: tokei per-file LOC report (Req 13.5; never blocking,
        never a runtime dependency).

        tokei stopped shipping prebuilt release binaries (v15.0.0 has no
        release assets), and Debian bookworm, the base of RUST_IMAGE, has no
        tokei package (only trixie and later), so it is installed from source
        via `cargo install` into the persistent cargo cache volume. On a cold
        engine that build stays off the critical path because `all` schedules
        this stage right after the heavy stages (design Decision 4 fallback)."""
        return await (
            rust_container(src)
            .with_exec(
                [
                    "sh",
                    "-c",
                    "tokei --version >/dev/null 2>&1 || cargo install tokei --locked",
                ]
            )
            .with_exec(["tokei", "--files"])
            .stdout()
        )

    @function
    async def release(self, src: dagger.Directory, revision: str, validation_id: str,
                      source_clean: bool = False, cargo_jobs: int = 2, cache_mode: str = "off",
                      cache_url: dagger.Secret | None = None,
                      cache_token: dagger.Secret | None = None) -> dagger.File:
        """File-compatible entrypoint selecting only the verified Linux bytes."""
        bundle = await self.static_release(src, revision, validation_id, source_clean, cargo_jobs=cargo_jobs,
                                           cache_mode=cache_mode, cache_url=cache_url, cache_token=cache_token)
        return bundle.file("mmhmem-linux-x86_64")

    @function
    async def docker_build(self, src: dagger.Directory) -> str:
        """Blocking: build the image per Dockerfile (compiles with the otel
        feature, Req 11.2/8.6) and prove the runtime payload runs by
        executing `mmhmem --version` inside the distroless image
        (use_entrypoint: the image has ENTRYPOINT mmhmem and no shell, so
        the args must be passed to the entrypoint, not run as a command)."""
        image = src.docker_build()
        version = await image.with_exec(["--version"], use_entrypoint=True).stdout()
        return f"docker image built; payload: {version.strip()}"

    @function
    async def compose_config(self, src: dagger.Directory) -> str:
        """Blocking: `docker compose config` on the production stack and the
        e2e override (Req 8.6; the merge-level check `--debug` cannot do)."""
        val_env = (
            "AGENT_MEM_TOKEN=ci\n"
            "AGENT_MEM_ALLOWED_ROOTS=/srv/workspace\n"
            "MMHMEM_PORT=18443\n"
            "MMHMEM_DATA_DIR=/tmp/val/data\n"
            "MMHMEM_LOG_DIR=/tmp/val/log\n"
            "MMHMEM_WORKSPACE_ROOT=/tmp/val/workspace\n"
            "E2E_WORKER_API_URL=http://host.docker.internal:11434/v1/chat/completions\n"
            "E2E_WORKER_MODEL=ci-worker\n"
            "E2E_WORKER_API_KEY=\n"
        )
        ctr = (
            dag.container()
            .from_("docker:29-cli")
            .with_mounted_directory("/repo", src)
            .with_workdir("/repo")
            .with_new_file("/tmp/val/.env", val_env)
        )
        prod = await ctr.with_exec(
            [
                "docker",
                "compose",
                "-f",
                "/repo/docker-compose.yml",
                "--project-directory",
                "/tmp/val",
                "config",
                "-q",
            ]
        ).stdout()
        e2e = await ctr.with_exec(
            [
                "docker",
                "compose",
                "-f",
                "/repo/docker-compose.yml",
                "-f",
                "/repo/tests/e2e/compose.e2e.yml",
                "--project-directory",
                "/tmp/val",
                "config",
                "-q",
            ]
        ).stdout()
        return f"compose config (prod): {prod or 'ok'}\ncompose config (e2e): {e2e or 'ok'}"

    @function
    async def deploy_artifacts(self, src: dagger.Directory, revision: str, validation_id: str,
                               source_clean: bool = False, cargo_jobs: int = 2, cache_mode: str = "off",
                               cache_url: dagger.Secret | None = None,
                               cache_token: dagger.Secret | None = None) -> str:
        """Blocking: pristine Linux validation of the deploy artifacts
        (Req 8.1/8.2) — systemd-analyze verify against the REAL release
        binary and logrotate --debug as root with the REAL agent-memory
        user (the local WSL approximations hit DrvFS/sudo limits)."""
        ctr = (
            dag.container()
            .from_("debian:bookworm-slim")
            .with_env_variable("DEBIAN_FRONTEND", "noninteractive")
            .with_exec(["apt-get", "update", "-qq"])
            .with_exec(
                ["apt-get", "install", "-y", "-qq", "systemd", "logrotate"]
            )
            .with_exec(
                [
                    "useradd",
                    "--system",
                    "--home-dir",
                    "/var/lib/agent-memory",
                    "--create-home",
                    "--shell",
                    "/usr/sbin/nologin",
                    "agent-memory",
                ]
            )
            .with_file(
                "/usr/local/bin/mmhmem",
                await self.release(src, revision, validation_id, source_clean, cargo_jobs=cargo_jobs,
                                   cache_mode=cache_mode, cache_url=cache_url, cache_token=cache_token),
                permissions=0o755,
            )
            .with_exec(["install", "-d", "-m", "0750", "/var/log/agent-memory"])
            .with_mounted_directory("/repo", src)
            .with_workdir("/repo")
        )
        verify = await ctr.with_exec(
            ["systemd-analyze", "verify", "deploy/systemd/mmhmem.service"]
        ).stdout()
        rotate = await (
            ctr.with_exec(
                ["cp", "deploy/logrotate.d/agent-memory", "/etc/logrotate.d/agent-memory"]
            )
            .with_exec(["logrotate", "--debug", "/etc/logrotate.d/agent-memory"])
            .stdout()
        )
        return f"systemd-analyze verify: {verify or 'ok'}\nlogrotate --debug: ok\n{rotate[-400:]}"

    @function
    async def all(self, src: dagger.Directory, revision: str, validation_id: str,
                  source_clean: bool = False, cargo_jobs: int = 2, concurrency: int = 1,
                  cache_mode: str = "off", cache_url: dagger.Secret | None = None,
                  cache_token: dagger.Secret | None = None) -> str:
        """The full blocking pipeline (without the model-gated e2e suite,
        which runs as a separate workflow job with credentials).

        Stages follow ALL_STAGE_PLAN on the dependency-aware runner, at most
        `concurrency` at once (default 1: list order, the local memory
        guard). A failed stage never cancels a sibling; one StageFailure at
        the end names every failed and skipped stage. The engine memory is
        checked against the heavy-stage budget before any build. With the
        cache options (hosted runs), every Rust build runs under sccache and
        the summary carries a compile-cache line; without them nothing
        changes.
        """
        from .static_release import candidate_cache_stats, validate_cargo_jobs

        validate_concurrency(concurrency)
        validate_cargo_jobs(cargo_jobs)
        cache = CacheConfig.from_options(cache_mode, cache_url, cache_token)
        cache_stats: dict = {}
        # The probe must run every call: a cached /proc/meminfo would be stale.
        meminfo = await (dag.container().from_(MEMORY_PROBE_IMAGE)
                         .with_env_variable("MMHMEM_MEMORY_PROBE", str(time.time_ns()))
                         .with_exec(["cat", "/proc/meminfo"]).stdout())
        heavy_count = sum(1 for _, heavy, _ in ALL_STAGE_PLAN if heavy)
        check_memory(parse_mem_total_kib(meminfo), concurrency, heavy_count)

        async def static_release_stage() -> str:
            # The verified bundle is produced here once; its manifest is the report.
            bundle = await self.static_release(src, revision, validation_id, source_clean, cargo_jobs=cargo_jobs,
                                               cache_mode=cache_mode, cache_url=cache_url, cache_token=cache_token)
            manifest = await bundle.file("verification.json").contents()
            cache_stats["static_release"] = await candidate_cache_stats(
                src, validation_id, cargo_jobs=cargo_jobs, cache=cache)
            return manifest

        async def docker_build_stage() -> str:
            cache_stats["docker_build"] = off_record("Docker builder stage not cached")
            return await self.docker_build(src)

        async def clippy_otel_stage() -> str:
            output, cache_stats["clippy_otel"] = await _clippy_otel(src, cargo_jobs=cargo_jobs, cache=cache)
            return output

        async def stage_runner_tests_stage() -> str:
            return await _stage_runner_tests(src, validation_id)

        async def distribution_stage() -> str:
            return await self.distribution(src)

        async def fmt_stage() -> str:
            return await self.fmt(src)

        async def compose_config_stage() -> str:
            return await self.compose_config(src)

        async def static_installer_stage() -> str:
            # Runs after static_release, so the bundle's operations are engine-cached.
            return await self.static_installer(src, revision, validation_id, source_clean, cargo_jobs=cargo_jobs,
                                               cache_mode=cache_mode, cache_url=cache_url, cache_token=cache_token)

        async def deploy_artifacts_stage() -> str:
            return await self.deploy_artifacts(src, revision, validation_id, source_clean, cargo_jobs=cargo_jobs,
                                               cache_mode=cache_mode, cache_url=cache_url, cache_token=cache_token)

        runs = {
            "static_release": static_release_stage,
            "docker_build": docker_build_stage,
            "clippy_otel": clippy_otel_stage,
            "stage_runner_tests": stage_runner_tests_stage,
            "distribution": distribution_stage,
            "fmt": fmt_stage,
            "compose_config": compose_config_stage,
            "static_installer": static_installer_stage,
            "deploy_artifacts": deploy_artifacts_stage,
        }
        stages = [Stage(name, runs[name], tuple(needs), heavy) for name, heavy, needs in ALL_STAGE_PLAN]
        results = await run_stages(stages, concurrency)
        return all_summary(results, cache_stats)
