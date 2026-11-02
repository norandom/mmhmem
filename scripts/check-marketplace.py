"""CI: validate Agent Skills metadata and detect stale generated packages."""
import hashlib
import difflib
import json
from pathlib import Path
import re
import subprocess

import yaml

ROOT = Path(__file__).resolve().parents[1]
GENERATED = ["plugins", ".agents/plugins", ".claude-plugin", ".gemini-plugin"]


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
        assert data["name"] == name and data["version"] == config["version"]
        assert data["repository"] == "https://github.com/norandom/mmhmem"

before = snapshot()
before_text = {name: (ROOT / name).read_text() for name in before}
subprocess.run(["bash", "build-marketplace.sh"], cwd=ROOT, check=True)
after = snapshot()
for name in sorted(set(before) | set(after)):
    if before.get(name) != after.get(name):
        print("Mismatch:", name)
        print("".join(difflib.unified_diff(before_text.get(name, "").splitlines(True),
              (ROOT / name).read_text().splitlines(True))))
assert after == before, "Generated packages/catalogs are stale or builders differ"
print("PASS: Agent Skills metadata, token budget, package contents, reproducible marketplace output")
