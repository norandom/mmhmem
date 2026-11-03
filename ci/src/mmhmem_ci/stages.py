"""Dependency-aware stage runner, timing record and memory guard (ci-build-time).

Pure standard library with no `dagger` import, so ci/scripts/test_stages.py
can test it on the host (Python 3.12) and inside the Debian bookworm runtime
(Python 3.11). `MmhmemCi.all` wires it into the pipeline.

Scheduling: stages start in list order as concurrency slots free up, and a
stage starts only after every prerequisite passed. A failed stage never
cancels a sibling; dependents of a failed or skipped stage are skipped. After
all runnable stages finish, one StageFailure names every failed and skipped
stage. With concurrency=1 the stages run strictly in list order.
"""

import asyncio
from dataclasses import dataclass
import json
from typing import Awaitable, Callable, Dict, List, Literal, Optional, Tuple

Status = Literal["passed", "failed", "skipped"]

ERROR_TAIL_LINES = 40
TIMINGS_PREFIX = "== timings: "

# Per concurrently running heavy stage; measured in tasks 4.1 and 4.2.
HEAVY_STAGE_BUDGET_GIB: float = 3.0
# Engine, BuildKit and light stages.
ENGINE_RESERVE_GIB: float = 2.0


@dataclass(frozen=True)
class Stage:
    """One named unit of pipeline work."""

    name: str  # unique, used in reports
    run: Callable[[], Awaitable[str]]  # returns the stage's report text
    needs: Tuple[str, ...] = ()  # names of earlier prerequisite stages
    heavy: bool = False  # counts toward the memory budget


@dataclass(frozen=True)
class StageResult:
    """Outcome and timing of one stage."""

    name: str
    status: Status
    started_s: float  # seconds since runner start
    duration_s: float
    output: str  # report text, or last 40 lines of the error


class StageFailure(Exception):
    """Raised after all runnable stages finished when at least one failed."""

    def __init__(self, results: List[StageResult]):
        super().__init__(results)
        self.results = results  # every stage, in list order

    def __str__(self) -> str:
        lines = []
        for result in self.results:
            if result.status == "failed":
                lines.append(f"== {result.name}: failed after {result.duration_s:.1f}s")
                lines.extend(f"   {line}" for line in result.output.splitlines())
            elif result.status == "skipped":
                lines.append(f"== {result.name}: skipped (a prerequisite failed or was skipped)")
        return "\n".join(lines)


def _error_tail(error: BaseException) -> str:
    """Return the last ERROR_TAIL_LINES lines of the error text."""
    text = str(error) or type(error).__name__
    return "\n".join(text.splitlines()[-ERROR_TAIL_LINES:])


def _validate(stages: List[Stage], concurrency: int) -> None:
    """Reject invalid graphs and concurrency before any stage starts."""
    if concurrency < 1:
        raise ValueError(f"concurrency must be at least 1, got {concurrency}")
    seen = set()
    for stage in stages:
        if stage.name in seen:
            raise ValueError(f"duplicate stage name: {stage.name}")
        for need in stage.needs:
            if need not in seen:
                raise ValueError(f"stage {stage.name} needs {need}, which is not an earlier stage")
        seen.add(stage.name)


async def run_stages(stages: List[Stage], concurrency: int) -> List[StageResult]:
    """Run stages under the concurrency limit and return results in list order.

    Raise StageFailure after all runnable stages finish if any failed.
    """
    _validate(stages, concurrency)
    loop = asyncio.get_running_loop()
    origin = loop.time()
    results: Dict[str, StageResult] = {}
    pending: List[Stage] = list(stages)
    running: Dict["asyncio.Task[str]", Tuple[Stage, float]] = {}

    def elapsed() -> float:
        return loop.time() - origin

    def next_ready() -> Optional[Stage]:
        """Skip blocked stages, then return the first stage whose needs passed."""
        changed = True
        while changed:
            changed = False
            for stage in list(pending):
                if any(need in results and results[need].status != "passed" for need in stage.needs):
                    pending.remove(stage)
                    results[stage.name] = StageResult(stage.name, "skipped", elapsed(), 0.0, "")
                    changed = True
        for stage in pending:
            if all(need in results for need in stage.needs):
                pending.remove(stage)
                return stage
        return None

    try:
        while pending or running:
            while len(running) < concurrency:
                stage = next_ready()
                if stage is None:
                    break
                task = asyncio.ensure_future(stage.run())
                running[task] = (stage, elapsed())
            if not running:
                break
            done, _ = await asyncio.wait(list(running), return_when=asyncio.FIRST_COMPLETED)
            for task in done:
                stage, started = running.pop(task)
                duration = elapsed() - started
                error = task.exception()
                if error is None:
                    results[stage.name] = StageResult(stage.name, "passed", started, duration,
                                                      task.result())
                else:
                    results[stage.name] = StageResult(stage.name, "failed", started, duration,
                                                      _error_tail(error))
    except BaseException:
        # Only an outer cancellation reaches here; stage errors are collected above.
        for task in running:
            task.cancel()
        await asyncio.gather(*running, return_exceptions=True)
        raise

    ordered = [results[stage.name] for stage in stages]
    if any(result.status == "failed" for result in ordered):
        raise StageFailure(ordered)
    return ordered


def timings_line(results: List[StageResult]) -> str:
    """Return '== timings: ' followed by compact JSON of name, status, started_s, duration_s."""
    records = [
        {"name": r.name, "status": r.status, "started_s": round(r.started_s, 3),
         "duration_s": round(r.duration_s, 3)}
        for r in results
    ]
    return TIMINGS_PREFIX + json.dumps(records, separators=(",", ":"))


def check_memory(mem_total_kib: int, concurrency: int, heavy_count: int) -> None:
    """Raise ValueError naming --concurrency when
    min(concurrency, heavy_count) * HEAVY_STAGE_BUDGET_GIB + ENGINE_RESERVE_GIB exceeds MemTotal."""
    required_gib = min(concurrency, heavy_count) * HEAVY_STAGE_BUDGET_GIB + ENGINE_RESERVE_GIB
    available_gib = mem_total_kib / (1024 * 1024)
    if required_gib > available_gib:
        raise ValueError(
            f"--concurrency={concurrency} needs {required_gib:.1f} GiB of engine memory "
            f"({min(concurrency, heavy_count)} heavy stages x {HEAVY_STAGE_BUDGET_GIB:.1f} GiB "
            f"+ {ENGINE_RESERVE_GIB:.1f} GiB reserve), but the engine has {available_gib:.1f} GiB; "
            f"lower --concurrency"
        )
