"""mmhmem CI pipeline (task 14.1; Req 13.5/13.6, 11.2, 8.6, 8.7).

One Dagger module runs everywhere: locally via the WSL dagger CLI
(`cd ci && dagger call all --src ..`) and in GitHub Actions
(.github/workflows/ci.yml) — never two scripts (dev-environment steering).

Blocking gates: fmt, clippy (default AND `--features otel` — the Windows
lesson: gated code escapes plain clippy), the deterministic test suite
(including tests/loc_caps.rs, which machine-enforces the Req 13.3 size caps).
Informational: tokei per-file LOC report (dev tool only, Req 13.5).
Validation: release build (plain, zero otel crates), Docker image built with
`--features otel` (Req 11.2/8.6) whose payload runs, `docker compose config`
on both compose files (Req 8.6), and the Linux deploy artifacts
(systemd-analyze verify + logrotate --debug, Req 8.1/8.2) — the pristine
versions of the checks that DrvFS/sudo limitations only approximate locally.

NOTE: every function that reads container output awaits `.stdout()` — in the
Python SDK the leaf getters are coroutines; a sync function returning one
fails at runtime with FunctionError.
"""

import dagger
from dagger import dag, function, object_type

RUST_IMAGE = "rust:1.98-slim-bookworm"

# The repo mount keeps the rustup toolchain pin out (its rust-analyzer
# component is a dev-host concern; the image's 1.98 toolchain is used as-is).
TOOLCHAIN_PIN = "rust-toolchain.toml"


def rust_container(src: dagger.Directory) -> dagger.Container:
    """Rust 1.98 container with the repo mounted and persistent cargo caches.

    Caches mount the registry/git/target dirs, NOT all of CARGO_HOME: env
    vars are container-global in dagger, so overriding CARGO_HOME would also
    hide the rustup proxies (cargo-fmt/clippy-driver) from PATH.
    """
    return (
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
        .with_mounted_directory("/src", src.without_file(TOOLCHAIN_PIN))
        .with_workdir("/src")
    )


@object_type
class MmhmemCi:
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
        release assets), so it is installed from source via `cargo install`
        into the persistent cargo cache volume — built once, cached across
        subsequent runs."""
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
    async def release(self, src: dagger.Directory) -> dagger.File:
        """Blocking: plain release build (no otel feature, Req 11.2)."""
        ctr = rust_container(src).with_exec(["cargo", "build", "--release", "--locked"])
        return ctr.file("/cargo-target/release/mmhmem")

    @function
    async def docker_build(self, src: dagger.Directory) -> str:
        """Blocking: build the image per Dockerfile (compiles with the otel
        feature, Req 11.2/8.6) and prove the runtime payload runs by
        executing `mmhmem --version` inside the distroless image."""
        image = src.docker_build()
        version = await (
            image.with_entrypoint(["/usr/local/bin/mmhmem", "--version"]).stdout()
        )
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
            ["docker", "compose", "--project-directory", "/tmp/val", "config", "-q"]
        ).stdout()
        e2e = await ctr.with_exec(
            [
                "docker",
                "compose",
                "-f",
                "docker-compose.yml",
                "-f",
                "tests/e2e/compose.e2e.yml",
                "--project-directory",
                "/tmp/val",
                "config",
                "-q",
            ]
        ).stdout()
        return f"compose config (prod): {prod or 'ok'}\ncompose config (e2e): {e2e or 'ok'}"

    @function
    async def deploy_artifacts(self, src: dagger.Directory) -> str:
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
                await self.release(src),
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
    async def all(self, src: dagger.Directory) -> str:
        """The full blocking pipeline (without the model-gated e2e suite,
        which runs as a separate workflow job with credentials)."""
        results: dict[str, str] = {
            "fmt": await self.fmt(src),
            "clippy (default + otel)": await self.clippy(src),
            "test": await self.test(src),
            "docker_build (otel)": await self.docker_build(src),
            "compose_config (prod + e2e)": await self.compose_config(src),
            "deploy_artifacts (systemd + logrotate)": await self.deploy_artifacts(src),
        }
        loc = await self.loc_report(src)
        summary = "\n".join(f"== {name}: ok" for name in results)
        return f"{summary}\n== tokei (informational):\n{loc}"
