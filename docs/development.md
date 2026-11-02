# Development and releases

## Native development builds

With a checkout and the repository's Rust toolchain:

```sh
cargo build --release --locked
cargo test
```

This is a native development build, not the portable Linux distribution gate.
To register it locally, use `bash deploy/install_local.sh` on Linux/macOS or
`powershell -File deploy\install_local.ps1` on Windows. The installer can find
`target/release/mmhmem` or accept an explicit binary path.

## Default Linux release build

The canonical Dagger pipeline defaults to a static
`x86_64-unknown-linux-musl` release. It builds the locked dependency graph,
checks ELF linkage, exercises the same executable on Debian and Alpine,
and exports a checksum-bound verification bundle.

On the Windows development host:

```powershell
powershell -NoProfile -File scripts\dagger-ci.ps1 -Call all -ExportPath tmp/verified-linux
```

Choose a fresh output directory. The wrapper captures revision and cleanliness
before CI sync; dirty local bundles are non-publishable. WSL is CI transport
only, never a user deployment target. See the [release procedure](../deploy/README.md#4-verified-linux-release-transition)
for hosted artifacts, guarded draft delivery, publication checks and rollback.

The same pipeline runs in [GitHub Actions](../.github/workflows/ci.yml):
formatting, default/OTel clippy, deterministic GNU and musl tests, runtime and
installer checks, the unchanged OTel Docker image, compose validation and
systemd/logrotate checks. Native Windows/macOS jobs build their own artifacts.
The tokei report is informational. A separate optional credentialed
model-measurement job compares token use with a real OpenCode harness.
