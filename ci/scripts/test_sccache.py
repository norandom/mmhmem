"""Unit tests for the compile-cache configuration (ci-dependency-cache task 1.1).

Requirements 1.3, 1.4, 2.3, 2.5; design "sccache.py". The module is imported
directly from ci/src/mmhmem_ci (its package __init__ imports dagger).
"""

from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src" / "mmhmem_ci"))

import sccache  # noqa: E402
from sccache import CacheConfig, cache_env, with_cache  # noqa: E402


class FakeContainer:
    """Records the container calls with_cache makes, in order."""

    def __init__(self, calls=None):
        self.calls = list(calls or [])

    def _add(self, *call):
        return FakeContainer(self.calls + [call])

    def with_file(self, path, file, **kwargs):
        return self._add("file", path, file)

    def with_exec(self, args, **kwargs):
        return self._add("exec", tuple(args))

    def with_env_variable(self, name, value, **kwargs):
        return self._add("env", name, value)

    def with_secret_variable(self, name, secret):
        return self._add("secret", name, secret)


class ConfigTests(unittest.TestCase):
    def test_no_secrets_means_off(self):
        self.assertEqual(CacheConfig.from_options("read-write", None, None).mode, "off")
        self.assertEqual(CacheConfig.from_options("off", None, None).mode, "off")

    def test_both_secrets_enable_the_requested_mode(self):
        cfg = CacheConfig.from_options("read-only", "url-secret", "token-secret")
        self.assertEqual((cfg.mode, cfg.url, cfg.token), ("read-only", "url-secret", "token-secret"))

    def test_a_single_secret_is_rejected(self):
        for url, token in (("u", None), (None, "t")):
            with self.assertRaisesRegex(ValueError, "both"):
                CacheConfig.from_options("read-write", url, token)

    def test_unknown_mode_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "cache mode"):
            CacheConfig.from_options("write-only", "u", "t")

    def test_off_with_secrets_stays_off(self):
        self.assertEqual(CacheConfig.from_options("off", "u", "t").mode, "off")


class EnvTests(unittest.TestCase):
    def test_read_write_environment(self):
        env = cache_env("read-write")
        self.assertEqual(env["RUSTC_WRAPPER"], sccache.SCCACHE_BIN)
        self.assertEqual(env["SCCACHE_GHA_ENABLED"], "on")
        self.assertEqual(env["SCCACHE_GHA_RW_MODE"], "READ_WRITE")
        self.assertEqual(env["SCCACHE_GHA_VERSION"], sccache.GHA_CACHE_VERSION)
        self.assertEqual(env["SCCACHE_IGNORE_SERVER_IO_ERROR"], "1")

    def test_read_only_environment(self):
        self.assertEqual(cache_env("read-only")["SCCACHE_GHA_RW_MODE"], "READ_ONLY")

    def test_environment_never_carries_credentials(self):
        for mode in ("read-only", "read-write"):
            names = set(cache_env(mode))
            self.assertFalse(names & {"ACTIONS_RESULTS_URL", "ACTIONS_RUNTIME_TOKEN"})

    def test_off_has_no_environment(self):
        self.assertEqual(cache_env("off"), {})


class ProvisioningTests(unittest.TestCase):
    def test_off_leaves_the_container_unchanged(self):
        ctr = FakeContainer([("base",)])
        self.assertIs(with_cache(ctr, CacheConfig.from_options("off", None, None)), ctr)

    def test_enabled_installs_verified_binary_then_sets_env_and_secrets(self):
        fetched = []
        cfg = CacheConfig.from_options("read-write", "url-secret", "token-secret")
        out = with_cache(FakeContainer(), cfg, fetch=lambda url: fetched.append(url) or "tarball")
        self.assertEqual(fetched, [sccache.SCCACHE_URL])
        kinds = [c[0] for c in out.calls]
        self.assertEqual(kinds[:2], ["file", "exec"], "binary installed before anything else")
        install = " ".join(out.calls[1][1])
        self.assertIn(sccache.SCCACHE_SHA256, install, "checksum verified before use")
        self.assertIn("sha256sum -c", install)
        env = {c[1]: c[2] for c in out.calls if c[0] == "env"}
        self.assertEqual(env, cache_env("read-write"))
        secrets = {c[1]: c[2] for c in out.calls if c[0] == "secret"}
        self.assertEqual(secrets, {"ACTIONS_RESULTS_URL": "url-secret", "ACTIONS_RUNTIME_TOKEN": "token-secret"})

    def test_pin_is_an_exact_release_with_a_sha256(self):
        self.assertRegex(sccache.SCCACHE_VERSION, r"^0\.(1\d|[2-9]\d)\.\d+$")
        self.assertIn(f"v{sccache.SCCACHE_VERSION}", sccache.SCCACHE_URL)
        self.assertIn("x86_64-unknown-linux-musl", sccache.SCCACHE_URL)
        self.assertRegex(sccache.SCCACHE_SHA256, r"^[0-9a-f]{64}$")


if __name__ == "__main__":
    unittest.main()
