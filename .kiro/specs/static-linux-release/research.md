# Research and Design Decisions: Static Linux Release

## Summary
- **Discovery scope:** extension of existing release infrastructure.
- **Skills consulted:** kiro-spec-design, its light discovery, synthesis,
  design principles and design review gate; existing core and development
  environment steering.
- **Key findings:** the native dependency chain needs musl C compilation;
  Linux output currently has no hosted artifact export; existing tests do
  not certify the exported release bytes.
- Read-only codebase research was delegated to an integration sub-agent.
  Synthesis and review were performed in the main context.
- No compiler experiment, application execution or implementation was
  performed during this design phase.

## Research Log

### Existing CI and release contracts
Sources: `ci/src/mmhmem_ci/main.py`, `.github/workflows/ci.yml`,
`scripts/dagger-ci.ps1`, `install.sh`.

The current release function returns a GNU executable as a Dagger File.
Deploy verification consumes it, but the workflow uploads only Windows/macOS
artifacts. There is no automated Linux publication workflow. Preserve the
existing Linux asset name and add a verified bundle export. The local CI
source sync excludes .git, so native PowerShell must capture revision and
dirty state before syncing. Actual `rust-toolchain.toml` says stable despite
comments describing a pin; the release builder must select its compiler
explicitly and record its identity.

### Rust target and static linkage
Sources:
- [Rust target support](https://doc.rust-lang.org/rustc/platform-support.html)
- [Rust static and dynamic C runtimes](https://doc.rust-lang.org/reference/linkage.html#static-and-dynamic-c-runtimes)
- [Rust kernel baseline history](https://blog.rust-lang.org/2022/08/01/Increasing-glibc-kernel-requirements/)

Rust lists x86_64-unknown-linux-musl as a supported target and documents
static CRT defaults for it. Compiler success is not proof of final linkage;
inspect ELF interpreter and required-library metadata. Static PIE must not
be rejected solely for ET_DYN. The historical Rust kernel floor is not proof
of this application's present minimum. The design selects Linux 5.10 and
baseline x86-64/SSE2 as a conservative support policy and records the actual
tested kernel separately. Container matrices test userlands, not kernels.

### Native compilation
Sources: `Cargo.toml`, `Cargo.lock`,
[AWS-LC Linux build requirements](https://aws.github.io/aws-lc-rs/requirements/linux.html).

SQLite is bundled. Reqwest uses rustls, but the lockfile includes AWS-LC
1.19.0 / aws-lc-sys 0.47.0 and ring 0.17.14. The active musl target feature
graph determines which native code is built. Set the target-specific C
compiler as well as the Rust linker. Current AWS-LC non-FIPS guidance requires
a C/C++ compiler and does not require CMake, Go or bindgen; merely seeing a
cmake crate in Cargo.lock does not establish an executable build prerequisite.
Retain the existing crypto provider and fail on incompatible compilation
rather than silently changing TLS dependencies. Build feasibility remains
to be demonstrated in implementation.

### TLS and runtime probes
Sources: `src/services/worker.rs`, `tests/local_stdio.rs`,
`tests/multiprocess.rs`, `tests/daemon_smoke.rs`, `tests/stats_cli.rs`,
`tests/worker_client.rs`,
[rustls-platform-verifier](https://docs.rs/rustls-platform-verifier/latest/rustls_platform_verifier/),
[rustls-native-certs](https://docs.rs/rustls-native-certs/latest/rustls_native_certs/).

The existing worker constructs a default reqwest client. On Linux the
platform verifier discovers system/user CA roots and uses webpki; roots are
loaded on process startup. Preserve that policy and use fresh processes for
trusted and untrusted fixtures. Existing worker stubs use HTTP/IP addresses,
so they do not prove DNS or HTTPS. A Dagger service hostname and temporary
CA provide deterministic coverage without public network/model credentials.

Existing subprocess tests use Cargo's selected executable, not an arbitrary
published file. Preserve the full static-target suite, then run separate
black-box probes taking an explicit candidate path. Use public MCP writes
to seed persistent state, and receipt logs plus unique content to prevent
cache or raw-read fallback from satisfying a worker-success assertion.

## Architecture Pattern Evaluation

| Option | Strength | Limitation | Decision |
|---|---|---|---|
| Extend existing Dagger builder with musl target | Reuses established Rust/CI conventions | Native C compiler must be target-specific | Selected |
| Separate external cross-build service | Ready-made build environments | Adds another pipeline and trust boundary | Rejected |
| Static glibc | Smaller build delta | Does not meet the requested non-glibc approach | Rejected |
| Rebuild separately for Debian and Alpine | Easy per-distro compilation | Does not prove one portable artifact | Rejected |
| Change TLS provider to simplify linking | Might reduce native code | Alters runtime/security behavior outside scope | Rejected |

## Design Decisions

### One candidate, several verifications
The common operation is verification of one immutable file. Linkage inspection,
runtime probes and release upload all consume that same file/checksum.
This avoids both a generic build framework and per-distribution binaries.

### Adopt tools, add narrow orchestration
Use rustup's target support, musl-gcc, readelf, Dagger services, Python's standard
library and gh. Add only the glue for this repository's artifact and protocol
contracts. No new runtime library or custom TLS client is needed.

### Export before publication
The CI bundle carries checksum and structured evidence; a native PowerShell
publisher validates the successful hosted run and matching tag before draft
upload. It cannot build or publish a complete release. This closes the
existing Linux artifact gap without taking ownership of other platforms.

### No premature portability claims
Keep the glibc caveat until the latest-release endpoint actually serves the
verified static candidate. The Docker image remains GNU/OTel and must be
described separately. Normal Linux installs do not depend on WSL or Git Bash.

## Risks and Mitigations
- Musl C/native crypto compilation may fail: first implementation work proves
  the existing locked graph; no production dependency swap is pre-approved.
- Toolchain/image drift: select exact compiler, record digests/tool versions,
  isolate the target cache and rerun verification after changes.
- Runtime instrumentation may install shared libraries: pristine-image and
  scratch startup checks plus ELF inspection precede instrumented probes.
- Container kernels do not prove a minimum kernel: label the declared support
  floor and actual tested environment separately; do not claim old-kernel tests.
- TLS certificate fixtures can expire in cached layers: fresh validation ID
  invalidates fixture/result caching, while compilation caches remain reusable.
- Rustls trust roots are cached in-process: use a new child for each trust case.
- Rebuilding during publication could change bytes: publisher accepts only the
  matching successful workflow bundle and re-downloads the uploaded draft asset.
- Current Linux release remains dynamic until migration completes: documentation
  changes are sequenced after publication verification.
