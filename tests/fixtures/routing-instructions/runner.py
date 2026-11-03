"""Shared golden runner. Adapters return (outcome, bytes-or-None); errors use outcome=error."""
import json
from pathlib import Path
from collections.abc import Callable

ROOT = Path(__file__).resolve().parent

def run_cases(adapter: Callable[[dict, bytes | None, bytes], tuple[str, bytes | None]]) -> int:
    manifest = json.loads((ROOT / 'cases.json').read_text(encoding='utf-8'))
    canonical = (ROOT / manifest['canonical']).read_bytes()
    for case in manifest['cases']:
        source = (ROOT / case['input']).read_bytes() if case['input'] else None
        expected = (ROOT / case['expected']).read_bytes() if case['expected'] else None
        outcome, actual = adapter(case, source, canonical)
        if outcome != case['outcome'] or actual != expected:
            raise AssertionError(f"{case['id']}: expected {case['outcome']} and exact bytes")
    return len(manifest['cases'])
