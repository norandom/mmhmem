# Implementation Plan

All tasks are required. Each executable task is sized for approximately 1–3 hours.
File ownership follows the approved design's File Structure Plan. Decision 3
(drop glibc default-feature clippy and tests from `all`) was approved by the
user at the design gate on 2026-11-03. No task changes product source, release
artifact names, or the manifest schema. Stage names are fixed by the design's
`all` stage-list table and are shared by the pipeline, the timing baseline and
the summary report.

- [ ] 1. Establish the stage-runner foundation
- [x] 1.1 Build the dependency-aware stage runner, timing record and memory guard
  - Run an ordered list of named stages under a concurrency limit, starting each only after its prerequisites passed, and never cancelling a sibling because another failed.
  - Mark dependents of a failed or skipped stage as skipped, then raise one aggregated failure naming every failed and skipped stage with its duration and error tail.
  - Emit one machine-readable timings line with name, status, start offset and duration per stage.
  - Refuse, before any stage starts, a configuration whose concurrent heavy stages exceed engine memory, naming the concurrency option and the required versus available memory. Start with the design's budget constants; tasks 4.1 and 4.2 measure them.
  - Reject duplicate stage names, unknown prerequisites and a concurrency below one before execution.
  - Done when the runner's unit tests pass for the concurrency limit, sibling survival, dependent skipping, list-order execution at concurrency one, the memory guard at 6 GiB with concurrency one and two, and invalid graphs.
  - _Requirements: 2.3, 2.4, 3.2, 3.4, 5.1_

- [ ] 2. Implement pipeline, reporting and cache changes
- [x] 2.1 Make cargo parallelism an explicit option of every Rust-compiling Dagger function
  - Thread a cargo job count through the glibc and musl build containers and through the static release, release, installer and deploy-artifact functions.
  - Keep the default at two jobs so every standalone and local call behaves exactly as today, including the static setup check's existing assertion.
  - Done when a default call yields two cargo jobs in both containers, an explicit option of four yields four in both, and the static setup check still passes unchanged.
  - _Requirements: 2.5, 3.1, 3.2, 3.3_

- [x] 2.2 Rebuild the full pipeline on the stage runner
  - Declare the design's stage list, order, heavy flags and dependencies, with the installer and deploy-artifact checks depending on the verified bundle.
  - Add the concurrency option with a default of one, probe engine memory before any build, and run the runner's own unit tests as a light stage.
  - Remove the glibc default-feature clippy and test passes from the full pipeline per approved Decision 3, keeping the glibc otel clippy pass and the standalone clippy and test functions.
  - Keep the existing per-stage ok lines, bundle manifest and LOC report in the returned text, followed by the timings line.
  - Done when a contract test over the built stage list with stubbed stage functions shows the same stage set and dependencies at concurrency one and three, an injected stub failure reports every failed and skipped stage, and the returned text ends with a parseable timings line. A real concurrent run is left to task 4.2 because the local engine refuses concurrency above one.
  - _Depends: 1.1, 2.1_
  - _Requirements: 1.1, 2.1, 2.2, 2.3, 2.4, 3.1, 3.4, 5.1_

- [x] 2.3 Obtain tokei for the informational LOC report without a per-run source build
  - Change only how the LOC stage installs tokei, and its position in the stage list if the fallback applies. The LOC stage keeps using the glibc build container as adjusted in 2.1.
  - Check whether the gate's Debian bookworm image provides a tokei package and record the version found in research.
  - Install it from the archive when available; otherwise keep the source install but schedule the stage so it overlaps the static chain, as design Decision 4 allows.
  - Done when the LOC stage no longer compiles tokei on a cold engine, or the fallback is recorded with a stub-runner test showing the stage starts alongside the static chain.
  - _Depends: 2.2_
  - _Requirements: 4.4_

- [ ] 2.4 (P) Write the job-summary timing report and its baseline
  - Parse the timings line, compare each stage with the recorded baseline, and render a table with a slow marker above 150 percent of baseline.
  - Take baseline keys from the design's stage-list table. Seed values from research for stages measured today, and mark the new stages without history (the otel clippy pass and the runner tests) as unbaselined until task 4.2 records their first hosted values.
  - Report a missing timings line or an unbaselined stage as such, and never change the job verdict.
  - Done when unit tests show a stage at 1.6 times baseline marked slow, one at 1.4 times unmarked, an unbaselined stage shown without a marker, and a missing line handled with a zero exit.
  - _Boundary: timing_summary_
  - _Depends: 1.1_
  - _Requirements: 1.3, 5.1, 5.3_

- [ ] 2.5 (P) Restore native cargo caches across version-only lockfile changes
  - Replace the Windows and macOS combined cache steps with separate restore and save steps keyed by OS, toolchain and lockfile hash, with a prefix fallback.
  - Save only on push events and only without an exact hit. Cache only the cargo registry, git checkouts and target directory, and pass no secrets to these jobs.
  - Append exact, fallback with matched key, or miss to each job's summary.
  - Keep the uploaded artifact names unchanged for the release publisher.
  - Done when the workflow contract test confirms restore keys, push-only save, unchanged artifact names, and the summary line on both native jobs.
  - _Boundary: Native cache steps_
  - _Requirements: 1.2, 4.1, 4.2, 4.3, 5.2_

- [ ] 3. Integrate the pipeline with the workflow
- [ ] 3.1 Wire concurrency, cargo jobs and the timing summary into the blocking gate job
  - Pass concurrency three and cargo jobs four to the full pipeline, and the same cargo job count to the bundle export step.
  - Confirm the Dagger action's output name for the pinned version from its action definition, then feed that output to the summary step, which always runs.
  - Extend the workflow contract test to assert matching cargo job counts on both Dagger steps, the concurrency option, and the unchanged hosted artifact identity.
  - Document the two options, the local defaults and the memory-guard message in the development guide.
  - Done when the workflow contract test passes and the local wrapper accepts the new options as safe function arguments without wrapper changes.
  - _Depends: 2.2, 2.3, 2.4, 2.5_
  - _Requirements: 2.5, 3.1, 3.3, 5.1_

- [ ] 4. Validate locally and on hosted runners
- [ ] 4.1 Run the local regression gates and memory-guard checks
  - Run the project's pre-commit gates, the CI Python unit tests, and the PowerShell workflow contract test.
  - Run the full pipeline through the local wrapper with default options on the WSL engine and record peak engine memory at concurrency one.
  - Run it again with concurrency two to confirm the guard refusal message.
  - Done when the default local run passes with sequential stages and two cargo jobs, its peak engine memory is recorded, and the concurrency-two run fails before any build with a message naming the concurrency option.
  - _Requirements: 2.1, 3.1, 3.2, 3.4_

- [ ] 4.2 Run the first hosted gate, sample memory and calibrate budgets
  - Requires the user's authorization to push the integrated change to `main`.
  - Sample runner memory during the first hosted gate run at concurrency three with a background step on the runner host that records `free -m` every five seconds and reports the peak used value.
  - If the measured peak contradicts the heavy-stage budget, change the budget constant, re-run 1.1's unit tests and 4.1's guard check, and note the revalidation in the validation report.
  - If a raised budget would refuse concurrency one on the 6 GB local WSL engine, stop and return to design instead of committing the constant.
  - Record first hosted durations for the unbaselined stages in the timing baseline.
  - Done when the first hosted run is green, its peak memory and the resulting budget decision are recorded in `.kiro/specs/ci-build-time/validation.md`, and every stage in the baseline has a value.
  - _Depends: 3.1, 4.1_
  - _Requirements: 1.3, 3.3, 3.4, 5.1, 5.3_

- [ ] 4.3 Measure five-run acceptance and finish the validation report
  - Requires the user's authorization for further pushes to `main`, including one version-bump commit.
  - Record five consecutive green main runs: gate median, whole-workflow median, per-stage summary tables, cache status lines, and an unchanged verification manifest schema.
  - Compare the version-bump commit's native job times against their warm medians.
  - Leave any missed target as NOT_VERIFIED with the measured numbers, and name cross-run Dagger caching as the follow-up per Decision 5.
  - Done when `.kiro/specs/ci-build-time/validation.md` lists measured evidence for every acceptance criterion, each marked PASS or NOT_VERIFIED.
  - _Requirements: 1.1, 1.2, 1.3, 2.1, 2.5, 4.1, 4.2, 5.1, 5.2, 5.3_

## Implementation Notes

- The stage runner must stay pure standard library and Python 3.11-compatible: its unit tests run on the host with `py -3.12` and inside the Debian bookworm runtime (Python 3.11). The Dagger module itself requires Python 3.14 and cannot be imported on the host.
- Task 2.2 must wrap every stage `run` as an `async def` coroutine: a callable that raises synchronously, or a stage raising `CancelledError`, escapes the runner's failure aggregation (reviewer probe on 1.1).
- `ci/scripts/test_stages.py` locates the module at `../src/mmhmem_ci` relative to itself, so any container that runs it must mount `ci/scripts` and `ci/src/mmhmem_ci` with the repository layout (the full source mount already does).
- Task 2.2 must pass `cargo_jobs` into the glibc `rust_container` stages it schedules (the otel clippy pass and any other compile in `all`); task 2.1 threaded it only through the verified-bundle path, so those stages still default to 2 jobs (reviewer finding on 2.1).
- The full stage plan lives in `ALL_STAGE_PLAN` and `STAGE_LABELS` (module literals in `main.py`) so stdlib tests can read it with `ast`; keep stage names there in sync with `ci/timing-baseline.json` (task 2.4).
- If tokei stays a source build (2.3 fallback), decide there whether its compile gets `cargo_jobs`; `loc_report` currently compiles at the default 2 jobs as a light stage.
- Task 4.1 must confirm the local WSL engine's MemTotal is at least 5 GiB, which the guard requires at concurrency 1; otherwise adjust `ENGINE_RESERVE_GIB` with evidence.
- Task 2.3 was reviewed inline by the parent (the reviewer subagent was stopped by the user); evidence: diff inspection plus 43 CI unit tests passing on 3.12 and 3.11.
