"""Regression: reject a staged catalog violation even with an unstaged fix."""
from pathlib import Path
import os
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def git(*args, cwd):
    return subprocess.check_output(["git", *args], cwd=cwd)


def main():
    scratch = ROOT / "tmp"
    scratch.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="hook-regression-", dir=scratch) as directory:
        fixture = Path(directory)
        git("checkout-index", "--all", "--prefix=" + fixture.as_posix() + "/", cwd=ROOT)
        # Isolate fixture Git commands from hook-provided index/worktree variables.
        for key in ("GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE"):
            os.environ.pop(key, None)
        git("init", "--quiet", cwd=fixture)
        git("add", ".", cwd=fixture)
        git("-c", "user.name=Hook Test", "-c", "user.email=hook@example.invalid",
            "-c", "commit.gpgsign=false", "-c", "core.hooksPath=.no-hooks",
            "commit", "--quiet", "-m", "Fixture baseline", cwd=fixture)
        bad = fixture / "deploy/skills/unregistered/SKILL.md"
        bad.parent.mkdir(parents=True)
        bad.write_bytes((fixture / "deploy/skills/unified-memory/SKILL.md").read_bytes())
        git("add", ".", cwd=fixture)
        before = git("write-tree", cwd=fixture)
        bad.unlink()  # Working tree looks repaired, but the index is still broken.
        result = subprocess.run([sys.executable, str(ROOT / "scripts/pre-commit.py")],
                                cwd=fixture, capture_output=True, text=True)
        assert result.returncode != 0, "Hook accepted a staged marketplace violation"
        assert "config names must exactly match" in (result.stdout + result.stderr).lower(), result.stderr
        assert git("write-tree", cwd=fixture) == before, "Hook changed the index"
        assert not bad.exists(), "Hook changed unstaged edits"
        assert not list((fixture / "tmp").glob("pre-commit-*")), "Hook leaked its snapshot"
    print("PASS: staged catalog failure blocks hook; unstaged fix, index and cleanup preserved")


if __name__ == "__main__":
    main()
