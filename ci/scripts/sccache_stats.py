"""Run one build under sccache and record its cache statistics.

ci-dependency-cache task 1.2 (Requirements 1.2, 1.3, 5.1, 5.2; design
"sccache_stats.py"). Each Dagger exec is its own process tree, so the sccache
server and its statistics live only for that exec: this wrapper starts the
server, runs the build, and reads the statistics in the same exec.

    python3 ci/scripts/sccache_stats.py --stats /tmp/stats.json -- cargo build ...

Without `RUSTC_WRAPPER=sccache` the build runs plainly. If the server cannot
start, the build runs uncached with the same verdict. Records never contain
environment values. Standard library only; Python 3.11+.
"""

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
from typing import Callable, Dict, List, Tuple

# The stats parser is shared with the Dagger module (the Docker stage parses
# the same sccache JSON); /src keeps the repository layout in containers.
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src" / "mmhmem_ci"))
from sccache import parse_stats  # noqa: E402

Runner = Callable[[List[str], Dict[str, str]], Tuple[int, str]]


def is_sccache(path: str) -> bool:
    """True for an sccache binary path, with or without an .exe suffix."""
    return Path(path).stem.lower() == "sccache"


def _run(args: List[str], env: Dict[str, str]) -> Tuple[int, str]:
    if len(args) > 1 and is_sccache(args[0]):
        done = subprocess.run(args, env=env, capture_output=True, text=True, timeout=120)
        return done.returncode, done.stdout
    return subprocess.run(args, env=env).returncode, ""


def run_under_sccache(cmd: List[str], env: Dict[str, str], runner: Runner = _run) -> Tuple[int, dict]:
    """Run `cmd`; return its exit code and the stats record."""
    wrapper = env.get("RUSTC_WRAPPER", "")
    if not is_sccache(wrapper):
        return runner(cmd, env)[0], {"enabled": False, "reason": "not configured"}
    started, _ = runner([wrapper, "--start-server"], env)
    if started != 0:
        uncached = {k: v for k, v in env.items() if k != "RUSTC_WRAPPER"}
        reason = f"sccache server did not start (exit {started}); built without the cache"
        return runner(cmd, uncached)[0], {"enabled": False, "reason": reason}
    code = runner(cmd, env)[0]
    shown, output = runner([wrapper, "--show-stats", "--stats-format", "json"], env)
    try:
        record = parse_stats(json.loads(output)) if shown == 0 else None
    except ValueError:
        record = None
    if record is None:
        record = {"enabled": True, "hits": 0, "misses": 0, "reason": "statistics unavailable"}
    return code, record


def main(argv: List[str], env: Dict[str, str], runner: Runner = _run) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--stats", required=True, help="where to write the stats record (JSON)")
    parser.add_argument("cmd", nargs=argparse.REMAINDER, help="-- followed by the build command")
    args = parser.parse_args(argv)
    cmd = args.cmd[1:] if args.cmd[:1] == ["--"] else args.cmd
    if not cmd:
        parser.error("a build command is required after --")
    code, record = run_under_sccache(cmd, env, runner)
    Path(args.stats).write_text(json.dumps(record) + "\n", encoding="utf-8")
    return code


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:], dict(os.environ)))
