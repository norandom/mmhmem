# Implementation Plan

All tasks are required. Each executable task is sized for approximately 1–3 hours.
File ownership follows the approved design's File Structure Plan. No task changes
Cargo dependencies, the TLS crypto provider, the bundle manifest or release
publication. Every commit must pass the project pre-commit gate.

- [ ] 1. Build the compile-cache foundation
- [ ] 1.1 Build the cache configuration and provisioning module with its unit tests
  - Accept a cache mode of off, read-only or read-write with the results URL and runtime token as secrets; treat both secrets absent as off and reject a single secret or an unknown mode before any build.
  - Pin an exact sccache release of at least 0.10 for the static Linux target by URL and SHA-256, and verify the checksum before the binary is used.
  - Map an enabled configuration to the container environment (wrapper, GitHub backend on, cache version label, read-write mode, ignore server IO errors) with the two credentials as secret variables only.
  - Leave containers unchanged when the configuration is off.
  - Done when unit tests show the configuration rules, the environment mapping per mode, and that an off configuration adds nothing, and the module compiles alongside the existing Dagger module.
  - _Requirements: 1.3, 1.4, 2.3, 2.5_

- [ ] 1.2 (P) Write the in-container wrapper that runs a build under sccache and records its statistics
  - Start the cache server before the build, and on failure run the build uncached and record the reason without printing any environment value.
  - Run the build command, keep its exit code, and write hit and miss counts, split into Rust and C where sccache reports them, to the requested statistics file.
  - Record a not-configured result when no wrapper is set.
  - Done when unit tests on recorded sccache statistics output and a simulated failed server start produce the expected records, the exit code is preserved, and no credential text appears in any record.
  - _Boundary: sccache_stats_
  - _Requirements: 1.2, 1.3, 5.1, 5.2_

- [ ] 2. Apply the cache and reshape the gate
- [ ] 2.1 Cache the musl candidate and otel clippy builds and report per-stage cache results
  - Thread the cache configuration through the full pipeline, the static release path and the otel clippy stage, applying it as the last container step and running each build through the wrapper in the same exec.
  - Emit one machine-readable cache line in the gate output with the per-stage statistics, after the summary and before the timings line.
  - Remove the LOC stage from the gate's stage plan while keeping the standalone LOC function for local use.
  - Done when stub-based plan and output tests pass, and the local wrapper run with no cache options still builds the same container definitions as before.
  - _Depends: 1.1, 1.2_
  - _Requirements: 1.1, 1.3, 1.4, 4.5, 5.1_

- [ ] 2.2 Give the Dockerfile builder stage an optional sccache path
  - Add empty-default build arguments for the sccache URL and checksum and an optional secret mount for the cache environment; without them the build step stays exactly as today.
  - When configured, verify and use sccache in the builder stage and write its statistics file there.
  - Build the builder stage once to read the statistics, then the full image with identical inputs, and add the Docker stage's statistics to the cache line.
  - Done when a plain `docker build` without arguments or secret still produces the image, and the Docker stage contributes a cache record when configured.
  - _Depends: 2.1_
  - _Requirements: 1.1, 1.2, 2.3, 2.4, 5.1_

- [ ] 2.3 Add the compile-cache table to the job-summary report
  - Parse the cache line and render a per-stage table of enabled state, hits and misses, with the reason for disabled stages.
  - Report a missing cache line as such, and keep the script's always-zero exit.
  - Done when unit tests cover enabled stages, a disabled stage with its reason, and a missing line.
  - _Boundary: timing_summary_
  - _Depends: 2.1_
  - _Requirements: 5.1, 5.2_

- [ ] 2.4 (P) Document the TLS stack and the compile cache in the development guide
  - State that the TLS stack is rustls with aws-lc-rs and serves only outbound HTTPS: worker-model API calls and the OTLP exporter in the otel image; the mmhmem server terminates no TLS.
  - State that its C build is cached rather than replaced and how to confirm C cache hits in the job summary; describe the cache modes, the pull-request read-only mode and how to purge the cache.
  - Done when the guide contains both sections and the guidance checks still pass.
  - _Boundary: development guide_
  - _Requirements: 3.1, 3.2, 3.3_

- [ ] 3. Integrate with the workflow
- [ ] 3.1 Wire the cache credentials, cache options and LOC job into the workflow
  - Confirm the secret-argument syntax of the pinned Dagger CLI, then export the results URL and runtime token in the gate job and pass them, with read-write mode on pushes and read-only mode on pull requests, to both the gate and the bundle export step.
  - Add the LOC job that runs alongside the gate with continue-on-error, restores the tokei binary by version, installs it only on a miss, and writes the report to its job summary; nothing depends on it.
  - Extend the workflow contract test: identical cache options on both Dagger steps, read-only on pull requests, no credential text in run bodies, the LOC job's independence, and no LOC stage in the gate plan.
  - Done when the contract test passes on Windows PowerShell 5.1 and PowerShell 7 and the workflow parses.
  - _Depends: 2.1, 2.2, 2.3_
  - _Requirements: 1.1, 2.2, 2.3, 4.1, 4.2, 4.3, 4.4_

- [ ] 4. Validate locally and on hosted runners
- [ ] 4.1 Run the local regression gates
  - Run the pre-commit gate, the CI unit tests and the workflow contract test.
  - Run the full local pipeline with default options and confirm it passes with no cache options, no credential and no cache-service traffic, and that the bundle manifest schema is unchanged.
  - Build the Docker image without build arguments or secret.
  - Done when all three pass and the results are recorded in the validation report.
  - _Requirements: 1.4, 2.1, 2.4_

- [ ] 4.2 Measure hosted acceptance and record the validation report
  - Requires the user's authorization to push to `main`.
  - Record one cache-populating run and then five consecutive green `main` runs: gate median, per-run cache tables including C hit counts for the TLS stack and SQLite, the cold-run duration, an unchanged manifest schema, and the LOC job summary.
  - Name any dependency whose C build stayed uncached, update the timing baseline to the measured warm durations, and mark each criterion PASS or NOT_VERIFIED.
  - Done when the validation report lists measured evidence for every acceptance criterion.
  - _Depends: 3.1, 4.1_
  - _Requirements: 1.2, 1.5, 1.6, 2.1, 2.2, 4.1, 4.2, 4.3, 4.4, 5.1, 5.2, 5.3_
