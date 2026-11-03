"""Cross-run compile cache for the gate (ci-dependency-cache spec, design
"sccache.py"; Requirements 1.3, 1.4, 2.3, 2.5).

Hosted runs start a fresh Dagger engine, so cache volumes are always empty.
sccache with the GitHub Actions cache backend persists compiled outputs
across runs. cc >= 1.0 prefixes sccache to C compiles whenever
RUSTC_WRAPPER names it, so the C objects of aws-lc-sys and the bundled
SQLite are cached too (research.md "Design discovery").

The module imports no dagger symbols at load time so its logic is unit
tested on the host; containers and secrets are passed in duck-typed.
"""

from dataclasses import dataclass
import shlex
from typing import Any, Callable, Dict, List, Optional

# Exact pinned release (published 2026-09-14), static musl build, verified
# against the release's published .sha256 before use.
SCCACHE_VERSION = "0.18.0"
SCCACHE_URL = (
    "https://github.com/mozilla/sccache/releases/download/"
    f"v{SCCACHE_VERSION}/sccache-v{SCCACHE_VERSION}-x86_64-unknown-linux-musl.tar.gz"
)
SCCACHE_SHA256 = "45f1447fbe231e3037bde351ef70677dd212216c8d62ae7ca409fecc4d6acc89"
SCCACHE_BIN = "/usr/local/bin/sccache"
# SCCACHE_GHA_VERSION namespaces every entry; bump it to purge the cache.
GHA_CACHE_VERSION = "mmhmem-1"

MODES = ("off", "read-only", "read-write")
# Where sccache_stats.py writes a build's stats record inside the container
# (outside /candidate, so the release bundle's report set is unchanged).
STATS_PATH = "/tmp/sccache-stats.json"
_TARBALL = "/opt/sccache/sccache.tar.gz"
# The stats wrapper needs python3; the glibc Rust image lacks it.
_INSTALL = (
    "(command -v python3 >/dev/null || (apt-get update -qq && apt-get install -y -qq python3)) "
    f"&& cd /opt/sccache && echo '{SCCACHE_SHA256}  sccache.tar.gz' | sha256sum -c - "
    f"&& tar -xzf sccache.tar.gz --strip-components=1 "
    f"sccache-v{SCCACHE_VERSION}-x86_64-unknown-linux-musl/sccache "
    f"&& install -m 0755 sccache {SCCACHE_BIN}"
)


@dataclass(frozen=True)
class CacheConfig:
    """Compile-cache setting for one gate run; secrets stay opaque."""

    mode: str
    url: Optional[Any] = None
    token: Optional[Any] = None

    @staticmethod
    def from_options(mode: str, url: Optional[Any], token: Optional[Any]) -> "CacheConfig":
        """'off' when both secrets are absent or mode is 'off'; ValueError for
        a single secret or an unknown mode, before any build."""
        if mode not in MODES:
            raise ValueError(f"unknown cache mode {mode!r}; expected one of {', '.join(MODES)}")
        if (url is None) != (token is None):
            raise ValueError("the cache needs both --cache-url and --cache-token, or neither")
        if url is None or mode == "off":
            return CacheConfig("off")
        return CacheConfig(mode, url, token)


OFF = CacheConfig("off")


def off_record(reason: str = "cache off") -> Dict[str, object]:
    """Stats record for a stage that ran without the cache (Requirement 5.2)."""
    return {"enabled": False, "reason": reason}


def cached_exec(cmd: List[str], stats_path: str = STATS_PATH) -> List[str]:
    """Run `cmd` through the in-container stats wrapper, so the sccache server
    and its statistics share one exec (each Dagger exec is its own process)."""
    return ["python3", "ci/scripts/sccache_stats.py", "--stats", stats_path, "--", *cmd]


def _counts(stats: dict, key: str) -> Dict[str, int]:
    counts = (stats.get(key) or {}).get("counts") or {}
    return {k: v for k, v in counts.items() if isinstance(v, int)}


def parse_stats(data: dict) -> dict:
    """Turn sccache `--show-stats --stats-format json` output into a stats
    record (shared by sccache_stats.py and the Docker stage)."""
    if "enabled" in data:  # already a record (e.g. a Dockerfile fallback note)
        return data
    stats = data.get("stats") or {}
    hits, misses = _counts(stats, "cache_hits"), _counts(stats, "cache_misses")
    non_cacheable = stats.get("non_cacheable_compilations")
    return {
        "enabled": True,
        "hits": sum(hits.values()),
        "misses": sum(misses.values()),
        "rust_hits": hits.get("Rust", 0),
        "rust_misses": misses.get("Rust", 0),
        "c_hits": hits.get("C/C++", 0),
        "c_misses": misses.get("C/C++", 0),
        "non_cacheable": non_cacheable if isinstance(non_cacheable, int) else 0,
    }


def docker_secret_env(mode: str, url: str, token: str) -> str:
    """Shell-sourced content of the Dockerfile's `sccache_env` secret: the
    credentials plus the non-secret cache environment, safely quoted."""
    pairs = {"ACTIONS_RESULTS_URL": url, "ACTIONS_RUNTIME_TOKEN": token, **cache_env(mode)}
    return "".join(f"export {name}={shlex.quote(value)}\n" for name, value in pairs.items())


def cache_env(mode: str) -> Dict[str, str]:
    """Non-secret environment for an enabled mode; empty for 'off'."""
    if mode == "off":
        return {}
    return {
        "RUSTC_WRAPPER": SCCACHE_BIN,
        "SCCACHE_GHA_ENABLED": "on",
        "SCCACHE_GHA_VERSION": GHA_CACHE_VERSION,
        "SCCACHE_GHA_RW_MODE": "READ_ONLY" if mode == "read-only" else "READ_WRITE",
        # Server or backend IO errors fall back to a local compile.
        "SCCACHE_IGNORE_SERVER_IO_ERROR": "1",
    }


def _dag_http(url: str) -> Any:
    from dagger import dag

    return dag.http(url)


def with_cache(ctr: Any, cfg: CacheConfig, fetch: Callable[[str], Any] = _dag_http) -> Any:
    """Install the verified sccache binary and configure the container; the
    identical container when the cache is off (local runs keep their engine
    cache). Credentials are set only as secret variables (Requirement 2.3)."""
    if cfg.mode == "off":
        return ctr
    ctr = ctr.with_file(_TARBALL, fetch(SCCACHE_URL)).with_exec(["sh", "-c", _INSTALL])
    for name, value in cache_env(cfg.mode).items():
        ctr = ctr.with_env_variable(name, value)
    return ctr.with_secret_variable("ACTIONS_RESULTS_URL", cfg.url).with_secret_variable(
        "ACTIONS_RUNTIME_TOKEN", cfg.token
    )
