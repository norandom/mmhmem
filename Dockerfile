# mmhmem multi-stage image (Req 8.6, task 12.3).
#
# The runtime stage contains ONLY the dedicated mmhmem binary on a minimal
# non-root base — no shell, no curl, no package manager. Container
# healthchecks therefore run the binary itself: `mmhmem healthcheck` probes
# `http://127.0.0.1:$AGENT_MEM_PORT/healthz` (Req 1.1 healthz endpoint).
#
# Build (Req 11.2): the otel cargo feature is enabled ONLY here, never in the
# plain release binary.
#   docker build -t mmhmem:local .
#   docker compose up -d --wait

ARG RUST_VERSION=1.98

# --- builder: compile the static-ish release binary with `--features otel` ---
FROM rust:${RUST_VERSION}-slim-bookworm AS builder
# gcc + libc6-dev: linker and libc headers for rusqlite's bundled SQLite.
RUN apt-get update \
    && apt-get install -y --no-install-recommends gcc libc6-dev \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /build
# Manifests first for dependency-layer caching, then the sources. The image's
# own 1.98 toolchain is used; rust-toolchain.toml stays out (its rust-analyzer
# component is irrelevant in a build container) and Cargo.lock pins the deps.
COPY Cargo.toml Cargo.lock clippy.toml ./
COPY src ./src
RUN --mount=type=cache,target=/build/target \
    cargo build --release --locked --features otel \
    && cp target/release/mmhmem /build/mmhmem-out

# --- runtime: only the binary, non-root, no shell ---
FROM gcr.io/distroless/cc-debian12:nonroot AS runtime
COPY --from=builder /build/mmhmem-out /usr/local/bin/mmhmem
# Container-internal listen port (AGENT_MEM_PORT default, Req 1.1); the
# published port is chosen by docker-compose.
EXPOSE 8443
# Req 1.1 healthz probe without curl: the binary checks itself.
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD ["/usr/local/bin/mmhmem", "healthcheck"]
USER nonroot
ENTRYPOINT ["/usr/local/bin/mmhmem"]
