"""Static routing contract checks; live tool selection requires adoption tests."""
import argparse
from pathlib import Path
import tempfile

ROOT = Path(__file__).resolve().parents[1]
GUIDANCE = "deploy/instructions/mmhmem-routing.md"
SKILL = "deploy/skills/unified-memory/SKILL.md"
SCENARIOS = {
    "ordinary inspection": ("ordinary repository inspection", "smart_read", "before a full native or shell read"),
    "authorization": ("available and authorized", "permissions", "allowed roots"),
    "exact evidence": ("offset", "limit", "exact source", "summaries are not exact evidence"),
    "fallback": ("unavailable", "fails", "do not retry", "authorized native"),
    "preferences": ("explicit user", "search", "build", "test", "edit"),
    "accounting": ("do not claim savings", "connection"),
}


def validate(root):
    errors = []
    try:
        text = " ".join((root / GUIDANCE).read_text(encoding="utf-8").lower().split())
        skill = (root / SKILL).read_text(encoding="utf-8")
    except OSError as error:
        return [str(error)]
    for name, phrases in SCENARIOS.items():
        if any(phrase not in text for phrase in phrases):
            errors.append(f"missing {name} guidance")
    description = next((line for line in skill.splitlines() if line.startswith("description:")), "").lower()
    if any(word not in description for word in ("file", "token", "checkpoint")):
        errors.append("skill description must advertise file reading, tokens and checkpoints")
    if len(skill.split()) * 1.3 > 200:
        errors.append("skill exceeds 200-token estimate")
    return errors


def self_test(root):
    scratch = root / "tmp"
    scratch.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="routing-guidance-", dir=scratch) as temp:
        fixture = Path(temp)
        for path in (GUIDANCE, SKILL):
            target = fixture / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes((root / path).read_bytes())
        original = (fixture / GUIDANCE).read_text(encoding="utf-8")
        for name, phrases in SCENARIOS.items():
            (fixture / GUIDANCE).write_text(original.lower().replace(phrases[0], "removed"), encoding="utf-8")
            assert f"missing {name} guidance" in validate(fixture), name
        (fixture / GUIDANCE).write_text(original, encoding="utf-8")
        target = fixture / SKILL
        target.write_text(target.read_text(encoding="utf-8").replace("description:", "removed:"), encoding="utf-8")
        assert validate(fixture), "missing skill discovery accepted"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    errors = validate(ROOT)
    if errors:
        raise SystemExit("\n".join(errors))
    if args.self_test:
        self_test(ROOT)
    print("PASS: routing contracts and skill discovery; live adoption remains untested")


if __name__ == "__main__":
    main()
