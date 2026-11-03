"""Selected harness preflight and commit bridge; plans stay in process memory."""
import argparse
import base64
import json
import os
from pathlib import Path
import re
import subprocess
import sys
sys.path.insert(0, str(Path(__file__).resolve().parent))
from manage_routing import plan, commit


def override(name, expected=None):
    value = os.environ.get(name)
    if value and (expected is None or os.path.abspath(value) != os.path.abspath(expected)):
        raise ValueError(f'{name} changes effective configuration; use default configuration or configure manually. No integrations were changed.')


def prepare(harness, home, project, global_scope, remove=False):
    home, project = Path(home), Path(project)
    base = home if global_scope else project
    if harness == 'claude':
        override('CLAUDE_CONFIG_DIR', home/'.claude')
        destination = base/'.claude/rules/mmhmem.md'
        if not remove:
            for file in (home/'.claude/settings.json', project/'.claude/settings.json', project/'.claude/settings.local.json'):
                if file.exists():
                    try:
                        excluded = json.loads(file.read_text(encoding='utf-8-sig')).get('claudeMdExcludes')
                    except (ValueError, OSError):
                        raise ValueError('Claude settings cannot be validated; review settings before onboarding.') from None
                    if excluded:
                        raise ValueError('claudeMdExcludes may exclude routing rules; review before onboarding.')
            for name in ('CLAUDE_CODE_SIMPLE', 'CLAUDE_CODE_DISABLE_PROJECT_INSTRUCTIONS'):
                if os.environ.get(name):
                    raise ValueError(f'{name} changes instruction loading; clear before onboarding.')
    else:
        override('OPENCODE_CONFIG_DIR', home/'.config/opencode')
        override('XDG_CONFIG_HOME', home/'.config')
        override('OPENCODE_CONFIG')
        override('OPENCODE_CONFIG_CONTENT')
        if not remove and not global_scope and os.environ.get('OPENCODE_DISABLE_PROJECT_CONFIG', '').lower() in ('1', 'true'):
            raise ValueError('OPENCODE_DISABLE_PROJECT_CONFIG disables project instructions.')
        destination = home/'.config/opencode/AGENTS.md' if global_scope else project/'AGENTS.md'
        if not remove and not destination.exists() and (destination.parent/'CLAUDE.md').exists():
            try:
                version = subprocess.run(['opencode', '--version'], capture_output=True, text=True, timeout=10).stdout.strip()
            except (OSError, subprocess.TimeoutExpired):
                version = ''
            if not re.match(r'^v?([2-9]|[1-9][0-9]+)\.', version):
                raise ValueError('Creating AGENTS.md could suppress existing CLAUDE.md; review OpenCode fallback manually.')
    canonical = b'' if remove else (Path(__file__).parent/'instructions/mmhmem-routing.md').read_bytes()
    return plan(destination, 'remove' if remove else 'install', canonical, harness == 'claude')


def encode(value):
    return {key: str(item) if key == 'path' else base64.b64encode(item).decode() if isinstance(item, bytes) else item for key, item in value.items()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=['prepare', 'commit'])
    parser.add_argument('--home', default=str(Path.home()))
    parser.add_argument('--project', default=os.getcwd())
    parser.add_argument('--global-scope', action='store_true')
    parser.add_argument('--remove', action='store_true')
    parser.add_argument('--dry-run', action='store_true')
    parser.add_argument('--agents', nargs='*', default=[])
    args = parser.parse_args()
    try:
        if args.operation == 'prepare':
            print(json.dumps([encode(prepare(agent, args.home, args.project, args.global_scope, args.remove)) for agent in args.agents]))
        else:
            for value in json.load(sys.stdin):
                value['path'] = Path(value['path'])
                for key in ('original', 'bytes'):
                    if value[key] is not None:
                        value[key] = base64.b64decode(value[key])
                try:
                    commit(value, args.dry_run)
                except (ValueError, OSError):
                    raise ValueError(f"Routing update failed at {value['path']}; earlier integration operations may have completed. Correct the error and rerun.") from None
                prefix = '[dry-run] ' if args.dry_run else ''
                print(f"[mmhmem] {prefix}routing instructions: {value['path']} ({value['outcome']})")
            print('[mmhmem] Start a fresh harness session to load updated instructions and MCP metadata.')
    except (ValueError, OSError) as error:
        parser.exit(1, f'[mmhmem] {error}\n')

if __name__ == '__main__':
    main()
