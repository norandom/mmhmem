# Brief: ci-dependency-cache

## Problem

The blocking Dagger gate compiles the whole dependency graph from scratch on
every hosted run, about five times: musl clippy, musl tests, musl release,
glibc otel clippy and the otel Docker image. Each hosted job starts a fresh
Dagger engine, so its cache volumes are always empty. The graph has 212
crates (237 with `otel`), including C builds for the TLS stack (`aws-lc-sys`,
pulled in by `reqwest`'s default rustls provider) and bundled SQLite. The
mmhmem source itself is about 9,000 lines. The gate also builds `tokei` (124
crates) from source on every run, only for an informational LOC report.

ci-build-time (Decision 5) deferred cross-run caching. Its first hosted run
missed the 540 s target (666 s), so this spec takes up that follow-up.

## User decisions (2026-11-03)

- Dependency cache: sccache with the GitHub Actions cache backend, used
  inside the Dagger build containers.
- tokei: a separate, non-blocking CI job with the installed binary cached by
  version, keeping cross-agent-memory-server Requirement 13.5.
- TLS stack: keep rustls with the aws-lc-rs provider; make its C build a
  cache hit instead of replacing it.
- Releases continue from v0.4.1; v0.4.0 stays unpublished.

## Evidence

- Hosted run 41643349698: Docker otel release 7 min 17 s, static release
  chain 572 s, tokei install 2 min 36 s, otel clippy 2 min 36 s.
- Local warm-cache run (same pipeline, WSL engine): static release 118 s,
  Docker 34 s, whole gate 252 s.
- Native Windows and macOS jobs already cache and take 210 to 264 s.

## Boundary Candidates

- Owned: compile caching for every Rust build in the gate, the cache
  credential hand-off into containers, the tokei job, cache and timing
  reporting, the TLS stack documentation.
- Adjacent: ci-build-time (stage runner, timing summary, memory guard),
  static-linux-release (same-artifact verification), the Dockerfile (must
  stay buildable on its own), cross-agent-memory-server Requirement 13.5.
- Not owned: the crypto provider choice (kept as is), product code, release
  publication logic, the native Windows/macOS cache steps.
