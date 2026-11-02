# Requirements Document

## Project Description (Input)

Linux x86_64 users currently receive a glibc-dependent mmhmem release binary,
which cannot run unchanged on musl-only distributions. Provide one fully
static, non-glibc Linux release using musl, verify its linkage and actual
runtime behavior on glibc and musl distributions through the canonical Dagger
CI pipeline, and transition the existing latest-release installation flow
to that verified artifact. Keep Windows/macOS releases and the skill
marketplace intact. WSL is CI infrastructure only; Git Bash is prohibited
on Windows. The user requested this work through the Kiro specification
workflow, with its requirements, design and tasks approval gates.

## Requirements

## Boundary Context

- **In scope:** the plain Linux x86_64 release binary, its build and runtime
  verification, release-asset delivery, and installer/documentation transition.
- **Out of scope:** Linux ARM64 or other new architectures, changes to memory
  semantics or storage format, Windows/macOS binary changes, skill/plugin
  packaging changes, and migration of the optional OTel-enabled Docker image.
- **Adjacent expectations:** the existing MCP interfaces, local store,
  worker configuration, installer registration/removal behavior, and release
  marketplace remain compatible. Static linking removes shared-library
  dependencies; it does not remove the Linux kernel, filesystem permissions,
  DNS configuration, or HTTPS trust-store requirements.
- **Support baseline:** runtime acceptance covers Debian 12 and Alpine 3.22
  on x86_64 using the same artifact. This does not promise compatibility with
  every kernel, CPU extension set, or distribution configuration. Design must
  establish and document the minimum kernel/CPU requirements before tasks
  are approved.
- **Implementation constraint from the request:** use a static musl target.
  Compiler, linker, caching and inspection commands belong in the design.

### Requirement 1: Self-contained Linux release

**Objective:** As a Linux user, I want one release binary without a glibc
dependency, so that I can use mmhmem across glibc and musl distributions.

#### Acceptance Criteria
1. The mmhmem Linux release shall provide a single x86_64 executable that requires no dynamic loader or shared libraries to start and perform its supported operations.
2. When the Linux release is verified, the release pipeline shall reject an executable containing a dynamic-interpreter declaration or required shared-library entries.
3. When the verified executable is run on either supported baseline distribution, mmhmem shall start without installing glibc compatibility packages, system SQLite libraries, or system OpenSSL libraries.
4. The Linux release shall retain the existing plain-build feature scope, without enabling optional telemetry exports.

### Requirement 2: Functional runtime compatibility

**Objective:** As an mmhmem user, I want the static release to preserve working
memory and network behavior, so that portability does not reduce functionality.

#### Acceptance Criteria
1. When the candidate artifact is tested on each supported baseline distribution, the verification suite shall execute the same artifact bytes and confirm the reported release version.
2. When a client initializes an MCP stdio session with the candidate, mmhmem shall complete initialization and serve the existing memory-tool interface.
3. When observations and a session checkpoint are written and the process is restarted, mmhmem shall recover the stored state and return matching observations through memory search and retrieval.
4. When local statistics are requested after a verified session, mmhmem shall return statistics in the existing CLI output formats.
5. When the candidate is started in server mode with the existing configuration, mmhmem shall serve its health endpoint and enforce the existing MCP authentication behavior.
6. When a worker endpoint is addressed by hostname with a certificate trusted by the test environment, mmhmem shall resolve the hostname and complete an HTTPS worker request.
7. If a worker endpoint presents an untrusted certificate, then mmhmem shall reject the TLS connection without disabling certificate verification.

### Requirement 3: Repeatable CI verification

**Objective:** As a maintainer, I want a blocking portability gate, so that an
unverified or accidentally dynamic artifact cannot become the Linux release.

#### Acceptance Criteria
1. When Linux release validation runs locally or in hosted CI, the project shall use the same canonical Dagger pipeline to build and verify the candidate.
2. The release pipeline shall build the candidate from the identified repository revision and committed dependency lockfile.
3. When a candidate is built, the release pipeline shall run the deterministic regression suite for the static target as well as the artifact checks and runtime scenarios in Requirements 1 and 2.
4. If compilation, regression tests, linkage inspection, or a required runtime scenario fails, then the release pipeline shall fail the release gate and identify the failed check.
5. When validation succeeds, the release pipeline shall retain evidence identifying the source revision, build target, artifact checksum, and runtime environments associated with the result.
6. While Windows is the development host, project validation shall use native PowerShell and the canonical CI entrypoint, without Git Bash or ad hoc script execution in WSL.
7. Where WSL hosts CI infrastructure, the pipeline shall execute Linux build and runtime validation inside CI containers and shall not install or run mmhmem directly in WSL as a user deployment.

### Requirement 4: Release and installer transition

**Objective:** As a user of the one-line installer, I want the latest release
to deliver the verified portable binary without a source checkout.

#### Acceptance Criteria
1. When a release containing the static Linux candidate is published, its `mmhmem-linux-x86_64` asset shall match the checksum of the candidate that passed the release gate.
2. When the Linux x86_64 bootstrap installs that release, it shall obtain the binary from this repository's latest-release asset endpoint without requiring Rust, a compiler, or a source checkout.
3. When the static release replaces an existing installation, the installation flow shall preserve the existing memory store, worker settings, and harness registration contract.
4. When the bootstrap is run in dry-run mode or a download fails, it shall preserve the existing installed binary and user configuration according to the existing installer contract.
5. If the installation platform is unsupported, then the bootstrap shall report the unsupported platform without selecting an incompatible release asset.
6. The release transition shall preserve the Windows and macOS asset names and the repository's skill-marketplace installation paths.

### Requirement 5: Accurate portability guidance

**Objective:** As a Linux user, I want installation guidance that distinguishes
binary portability from environment requirements, so that I can deploy it correctly.

#### Acceptance Criteria
1. When the verified static artifact is available in the latest release, the installation guide shall identify the Linux asset as statically linked and remove the obsolete glibc-only limitation.
2. Until that release is available, the installation guide shall retain accurate limitations for the currently downloadable binary.
3. The installation guide shall state the supported architecture, tested distribution baselines, minimum kernel/CPU requirements established during design, and remaining shell/download-tool prerequisites.
4. Where HTTPS worker connections are used, the installation guide shall explain the required trust-store availability and the behavior when certificate trust cannot be established.
5. The installation guide shall describe normal Linux installations without presenting WSL or Git Bash as installation targets.

## Requirements Review Gate

Passed for draft generation: numeric IDs and EARS criteria checked; coverage
includes linkage, actual runtime behavior, persistence, DNS/TLS, release
provenance, upgrade compatibility and unsupported platforms. Architecture and
Docker-image boundaries are explicit. Musl and Dagger are retained as user
and project constraints; implementation mechanics are deferred to design.
This review does not constitute human approval.
