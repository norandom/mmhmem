"""CI: validate Agent Skills metadata and detect stale generated packages."""
import hashlib
import difflib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib

import yaml

ROOT = Path(__file__).resolve().parents[1]
GENERATED = ["plugins", ".agents/plugins", ".claude-plugin", ".gemini-plugin"]

subprocess.run([sys.executable, str(ROOT / "scripts/check-checkpoint-guidance.py"),
                "--self-test"], cwd=ROOT, check=True)

subprocess.run([sys.executable, str(ROOT / "scripts/check-routing-guidance.py"),
                "--self-test"], cwd=ROOT, check=True)


def snapshot():
    # JSON escaping/whitespace differs between jq and Windows PowerShell 5.1.
    # Compare parsed manifests; require exact bytes for packaged skill files.
    return {
        str(path.relative_to(ROOT)): hashlib.sha256(
            json.dumps(json.loads(path.read_text()), sort_keys=True).encode()
            if path.suffix == ".json" else path.read_bytes()
        ).hexdigest()
        for folder in GENERATED
        for path in (ROOT / folder).rglob("*")
        if path.is_file()
    }


config = json.loads((ROOT / ".marketplace/skills.json").read_text())

# The plugin version follows the crate version, so agents see a new plugin
# with every release (the catalogs track main). The version bump commit also
# regenerates the plugin manifests (docs/development.md "Releases").
CARGO_VERSION = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
REGENERATE = (
    f'Set "version" in .marketplace/skills.json to {CARGO_VERSION} (the Cargo.toml version), '
    "then regenerate with .\\build-marketplace.ps1 -ManifestsOnly (Windows) or "
    "bash build-marketplace.sh --manifests-only, and commit the result with the version bump."
)


def check_version(where, version):
    if version != CARGO_VERSION:
        sys.exit(f"FAIL: {where} has plugin version {version!r}, Cargo.toml has "
                 f"{CARGO_VERSION!r}. {REGENERATE}")


check_version(".marketplace/skills.json", config["version"])
for plugin in config["plugins"]:
    name = plugin["name"]
    skill = ROOT / "deploy/skills" / name / "SKILL.md"
    text = skill.read_text(encoding="utf-8")
    assert text.startswith("---\n"), "Missing frontmatter"
    fields = yaml.safe_load(text.split("---", 2)[1])
    assert set(fields) <= {
        "name", "description", "license", "compatibility", "metadata", "allowed-tools"
    }
    assert fields["name"] == name
    assert len(name) <= 64 and re.fullmatch(r"[a-z0-9]+(?:-[a-z0-9]+)*", name)
    assert isinstance(fields["description"], str) and 0 < len(fields["description"]) <= 1024
    assert len(fields.get("compatibility", "")) <= 500
    assert len(text.split()) * 1.3 <= 200, "Playbook token estimate exceeds budget"
    package = ROOT / "plugins" / name
    assert (package / "skills" / name / "SKILL.md").read_bytes() == skill.read_bytes()
    for manifest in ["plugin.json", ".codex-plugin/plugin.json", ".claude-plugin/plugin.json", ".gemini-plugin/plugin.json"]:
        data = json.loads((package / manifest).read_text())
        check_version(f"plugins/{name}/{manifest}", data["version"])
        assert data["name"] == name and data["version"] == config["version"]
        assert data["repository"] == "https://github.com/norandom/mmhmem"

before = snapshot()
before_text = {name: (ROOT / name).read_text() for name in before}
builder = (["powershell", "-NoProfile", "-File", "build-marketplace.ps1"]
           if os.name == "nt" else ["bash", "build-marketplace.sh"])
builder_env = {k: v for k, v in os.environ.items() if k.upper() != "PSMODULEPATH"}
subprocess.run(builder, cwd=ROOT, env=builder_env, check=True)
after = snapshot()
for name in sorted(set(before) | set(after)):
    if before.get(name) != after.get(name):
        print("Mismatch:", name)
        print("".join(difflib.unified_diff(before_text.get(name, "").splitlines(True),
              (ROOT / name).read_text().splitlines(True))))
assert after == before, "Generated packages/catalogs are stale or builders differ"
print("PASS: Agent Skills metadata, token budget, package contents, reproducible marketplace output")
