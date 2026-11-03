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
# Optional compile cache (ci-dependency-cache). Hosted CI passes the pinned
# sccache URL and SHA-256 plus a BuildKit secret `sccache_env` with the GitHub
# cache credentials; a plain `docker build` sets neither and runs the
# unchanged cargo build. Credentials never enter an ARG, ENV or layer.
ARG SCCACHE_URL=
ARG SCCACHE_SHA256=
RUN --mount=type=cache,target=/build/target \
    --mount=type=secret,id=sccache_env \
    if [ -n "$SCCACHE_URL" ] && [ -s /run/secrets/sccache_env ]; then \
        apt-get update -qq \
        && apt-get install -y -qq --no-install-recommends curl ca-certificates \
        && curl -fsSL "$SCCACHE_URL" -o /tmp/sccache.tgz \
        && echo "$SCCACHE_SHA256  /tmp/sccache.tgz" | sha256sum -c - \
        && tar -xzf /tmp/sccache.tgz -C /usr/local/bin --strip-components=1 --wildcards '*/sccache' \
        && . /run/secrets/sccache_env \
        && if sccache --start-server; then \
               cargo build --release --locked --features otel \
               && sccache --show-stats --stats-format json > /build/sccache-stats.json; \
           else \
               echo '{"enabled": false, "reason": "sccache server did not start; built without the cache"}' > /build/sccache-stats.json \
               && env -u RUSTC_WRAPPER cargo build --release --locked --features otel; \
           fi; \
    else cargo build --release --locked --features otel \
        && echo '{"enabled": false, "reason": "not configured"}' > /build/sccache-stats.json; \
    fi \
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
