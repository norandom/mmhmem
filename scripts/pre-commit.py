"""Validate the Git index in an isolated snapshot; never modify user files."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def run(*args, cwd, env=None):
    print("pre-commit: " + " ".join(map(str, args)), flush=True)
    if os.name == "nt" and args[0] == "powershell":
        # A Python child of pwsh otherwise forwards PowerShell 7 modules to 5.1.
        env = {k: v for k, v in (env or os.environ).items() if k.upper() != "PSMODULEPATH"}
    subprocess.run(args, cwd=cwd, env=env, check=True)


def main():
    root = Path(subprocess.check_output(
        ["git", "rev-parse", "--show-toplevel"], text=True).strip()).resolve()
    run("git", "diff", "--cached", "--check", cwd=root)
    scratch = root / "tmp"
    scratch.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="pre-commit-", dir=scratch) as directory:
        snapshot = Path(directory)
        # checkout-index exports staged contents, including additions/deletions,
        # without stashing, changing the index, or inspecting unstaged fixes.
        run("git", "checkout-index", "--all", "--prefix=" + snapshot.as_posix() + "/", cwd=root)
        run(sys.executable, "scripts/check-marketplace.py", cwd=snapshot)
        if os.name == "nt":
            run("powershell", "-NoProfile", "-File", "scripts/test-install.ps1", cwd=snapshot)
        else:
            run("bash", "scripts/test-install.sh", cwd=snapshot)
        run("cargo", "fmt", "--check", cwd=snapshot)
        build_env = dict(os.environ, CARGO_TARGET_DIR=str(root / "target/pre-commit"),
                         CARGO_INCREMENTAL="0", CARGO_BUILD_JOBS="2")
        run("cargo", "test", "--locked", "--test", "loc_caps", cwd=snapshot, env=build_env)
        for features in ([], ["--features", "otel"]):
            run("cargo", "clippy", "--locked", "--all-targets", *features,
                "--", "-D", "warnings", cwd=snapshot, env=build_env)
    print("PASS: staged packaging, installers, formatting, size and complexity checks")


if __name__ == "__main__":
    try:
        main()
    except (subprocess.CalledProcessError, OSError) as error:
        print(f"Pre-commit checks failed: {error}", file=sys.stderr)
        sys.exit(1)
