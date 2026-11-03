"""Check shipped instruction contracts; this does not test live agent compliance.

Scenario review remains necessary: phrase checks catch accidental instruction
removal, not contradictions or whether a model follows the instructions.
"""
import argparse
from pathlib import Path
import re
import shutil
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
CANONICAL = 'deploy/skills/unified-memory/SKILL.md'
PACKAGE = 'plugins/unified-memory/skills/unified-memory/SKILL.md'
SERVER = 'src/mcp/instructions.rs'
RECAPS = [f'deploy/commands/{agent}/mmhmem-recap.md'
          for agent in ('claude', 'codex', 'opencode')] + ['deploy/codex/skills/mmhmem-recap/SKILL.md']
INPUTS = [CANONICAL, PACKAGE, SERVER, *RECAPS]

# These named scenarios are intentionally checked per entry point, not across
# concatenated files: one harness must not mask missing guidance in another.
SCENARIOS = {
    'initial objective': ('accepted objective', 'next step', 'before execution'),
    'task milestone': ('task unit',),
    'validation milestone': ('validation batch',),
    'decision milestone': ('plan-changing decision',),
    'blocker milestone': ('blocker',),
    'changed final response': ('changed state', 'final response'),
    'unchanged final response': ('skip unchanged saves',),
    'advance-notice transition': ('agent-controlled clear', 'handoff'),
    'interruption boundary': ('ctrl+c', 'native clear', 'acknowledged checkpoint'),
    'failed save': ('acknowledgement', 'warn', 'not saved'),
    'project identity': ('exact project identity',),
    'resumable state': ('objective', 'verified', 'pending', 'decisions', 'blockers', 'next steps'),
    'checkpoint operation': ('updated_working_state_md', 'active_objective'),
    'history-only recovery': ('breadcrumb', 'activity', 'saved'),
    'separate chat history': ('chat history', 'separate'),
    'worker-independent recovery': ('restart', 'worker'),
}
RECAP_SCENARIOS = {
    'history without checkpoint': ('no checkpoint exists', 'recorded activity', 'without a saved working-state summary'),
    'empty history': ('neither exists', 'no saved state or recorded activity'),
    'task ID is not a save': ('task id is not a saved checkpoint',),
    'original conversation recovery': ('original conversation may create a checkpoint', 'automatically imported'),
    'interrupted work is not saved': ('do not claim interrupted work was saved',),
    'compliance limitation': ('depend on agent compliance',),
}


def normalized(text):
    # Rust source line continuations must not hide a phrase spanning two lines.
    return ' '.join(text.lower().replace('\\\n', ' ').split())


def check_scenarios(text, scenarios, label):
    return [f'{label}: missing {name} instruction ({phrase})'
            for name, phrases in scenarios.items() for phrase in phrases
            if phrase not in text]


def validate(root):
    errors = []
    texts = {}
    for path in INPUTS:
        try:
            texts[path] = (root / path).read_text(encoding='utf-8')
        except OSError as error:
            errors.append(f'{path}: {error}')
    if errors:
        return errors
    if (root / CANONICAL).read_bytes() != (root / PACKAGE).read_bytes():
        errors.append('packaged skill differs from canonical bytes')
    if texts['deploy/codex/skills/mmhmem-recap/SKILL.md'].replace('name: mmhmem-recap\n', '', 1) != texts['deploy/commands/codex/mmhmem-recap.md']:
        errors.append('Codex recap skill differs from legacy prompt guidance')
    if len(texts[CANONICAL].split()) * 1.3 > 200:
        errors.append('canonical skill exceeds 200-token estimate')
    for path in [CANONICAL, SERVER, *RECAPS]:
        text = texts[path]
        label = 'canonical' if path == CANONICAL else path
        if path == SERVER:
            match = re.search(r'const INSTRUCTIONS: &str = (.*?);\s*$', text, re.S)
            if not match:
                errors.append('server instructions assignment not found')
                continue
            text = match[1]
        text = normalized(text)
        errors.extend(check_scenarios(text, SCENARIOS, label))
        if path in RECAPS:
            errors.extend(check_scenarios(text, RECAP_SCENARIOS, label))
    return errors


def self_test(root):
    """Mutation fixtures prove missing triggers and stale packages are rejected."""
    scratch = root / 'tmp'
    scratch.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='checkpoint-guidance-', dir=scratch) as temp:
        fixture = Path(temp)
        for path in INPUTS:
            destination = fixture / path
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(root / path, destination)
        assert not validate(fixture), 'unmodified guidance fixture failed'
        for path in [CANONICAL, SERVER, *RECAPS]:
            original = (fixture / path).read_text(encoding='utf-8')
            scenarios = dict(SCENARIOS)
            if path in RECAPS:
                scenarios.update(RECAP_SCENARIOS)
            for scenario, phrases in scenarios.items():
                # Keep valid source around the mutation: a parse or parity
                # failure alone must not count as detecting a missing trigger.
                pattern = r'(?:\s|\\)*'.join(re.escape(word) for word in phrases[0].split())
                altered = re.sub(pattern, 'removed instruction', original, flags=re.I)
                (fixture / path).write_text(altered, encoding='utf-8')
                assert any(f'missing {scenario} instruction' in error
                           for error in validate(fixture)), f'{path}: missing {scenario} went undetected'
                (fixture / path).write_text(original, encoding='utf-8')
        # Restore exact bytes after text mutation (Windows newlines may differ).
        shutil.copyfile(root / CANONICAL, fixture / CANONICAL)
        (fixture / PACKAGE).write_bytes(b'divergent package')
        assert 'packaged skill differs from canonical bytes' in validate(fixture)
        with (fixture / CANONICAL).open('a', encoding='utf-8') as output:
            output.write(' extra' * 200)
        assert 'canonical skill exceeds 200-token estimate' in validate(fixture)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--self-test', action='store_true')
    args = parser.parse_args()
    errors = validate(ROOT)
    if errors:
        print('\n'.join(errors), file=sys.stderr)
        return 1
    if args.self_test:
        self_test(ROOT)
    print('PASS: checkpoint instruction scenarios, token budget and package parity')
    print('Evidence: static checks and mutation fixtures only; live agent compliance is untested.')
    return 0


if __name__ == '__main__':
    sys.exit(main())
