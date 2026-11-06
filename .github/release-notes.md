## Install

**Windows x64** (PowerShell 5.1 or 7):

```powershell
irm https://github.com/norandom/mmhmem/releases/latest/download/install.ps1 | iex
```

**Linux x86_64 and macOS** (Apple Silicon or Intel):

```sh
curl -fsSL https://github.com/norandom/mmhmem/releases/latest/download/install.sh | sh
```

These commands install the latest signed release. The installer downloads
the archive for your computer, uses it only if its SHA-256 matches the one
built into the installer, and starts `mmhmem setup`. Setup checks the release
signature, lets you choose your tools and features, and shows every file it
will change before anything is written. Already installed? Run
`mmhmem update` (on v0.5.0: `mmhmem setup`).

Optionally, check where an archive came from:
`gh attestation verify <archive> --repo norandom/mmhmem`.
