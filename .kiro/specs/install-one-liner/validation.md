# Distribution validation — 2026-11-02

- STATUS: VERIFIED for the checks below; not a full runtime FEATURE_GO.
- CLAIM_TYPE: TASK
- CLAIM: Release bootstraps, skill packaging and documentation have been
  corrected and tested within the distribution scope.

## Fresh evidence

- `powershell -NoProfile -File scripts/test-install.ps1`: exit 0. Mocked
  checkout-free downloads; actual registration engine dry runs; Codex skill,
  prompt and config actions; removal without companion files; no installed
  binary/config writes; staging cleanup; PowerShell 5.1 syntax.
- `powershell -NoProfile -File scripts/dagger-ci.ps1 -Call distribution`:
  exit 0. Debian bookworm and Alpine 3.22 containers both passed Bash syntax,
  simulated Linux/macOS platform selection, unsupported-platform refusal,
  dry-run behavior, preservation of the existing binary on download failure,
  removal and cleanup. Actual engine dry runs cover Codex registration,
  skill and recap installation/removal.
- The same CI gate parses skill YAML and checks supported metadata, name
  constraints, description length, 200-token estimated playbook budget,
  exact packaged skill contents, and equivalent Bash/PowerShell manifests.
  JSON escaping is normalized; skill contents are compared byte-for-byte.
- `claude plugin validate .` and
  `claude plugin validate ./plugins/unified-memory`: both exit 0.
- `build-marketplace.ps1 -Release v0.2.2`: expected rejection because that
  binary release lacks plugin manifests. Catalogs correctly track main.
- `git diff --check`: exit 0.
- `gh release view`: latest is v0.2.2, with Linux x86_64, both macOS
  architectures, and Windows x64 assets.

## Boundaries

Downloads in installer regression tests are mocked. Native macOS installation,
live marketplace installation and full Rust runtime regression tests were not
performed. Runtime Rust code is unchanged. The current Linux binary uses
glibc; passing shell tests on Alpine does not certify a musl runtime binary.

Windows tooling uses native PowerShell. Linux checks run only through the
canonical CI entrypoint and its containers. Earlier ad hoc WSL/Git Bash
checks were superseded following the user's explicit restriction; neither
is an installation target or a supported Windows script-testing route.

## Format references

- [Agent Skills specification](https://agentskills.io/specification)
- [Codex plugin packaging](https://developers.openai.com/plugins/build/plugins)
- [Claude marketplace format](https://code.claude.com/docs/en/plugin-marketplaces)

Gemini/Antigravity manifests follow the sibling repository convention. No
native Gemini CLI marketplace compatibility claim is made.
