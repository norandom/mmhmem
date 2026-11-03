"""Byte-preserving owned routing sections; CLI plans and commits share this API."""
import argparse
import json
import os
from pathlib import Path
import re
import tempfile

START = '<!-- mmhmem:routing:start -->'
END = '<!-- mmhmem:routing:end -->'

def assert_path(path):
    for item in (path, *path.parents):
        if item.is_symlink() or (hasattr(item, 'is_junction') and item.is_junction()):
            raise ValueError(f'Symlink/reparse routing path: {item}')

def section(text):
    first = last = None
    prefix = 'none'
    metadata = False
    for match in re.finditer(r'^.*mmhmem:routing:.*(?:\r?\n|$)', text, re.M):
        line = match.group().rstrip('\r\n')
        if line == START and first is None and last is None:
            first = match
        elif line == END and first is not None and last is None:
            last = match
        elif re.fullmatch(r'<!-- mmhmem:routing:prefix=(none|lf|crlf) -->', line) and first is not None and last is None and not metadata:
            prefix = re.fullmatch(r'<!-- mmhmem:routing:prefix=(none|lf|crlf) -->', line)[1]
            metadata = True
        else:
            raise ValueError('Ambiguous routing markers or prefix metadata')
    if (first is None) != (last is None):
        raise ValueError('Unmatched routing marker')
    if first is None:
        return None
    begin = first.start()
    owned = {'none': '', 'lf': '\n', 'crlf': '\r\n'}[prefix]
    if owned:
        if begin < len(owned) or text[begin-len(owned):begin] != owned:
            raise ValueError('Routing prefix does not match preceding bytes')
        if prefix == 'lf' and begin >= 2 and text[begin-2] == '\r':
            raise ValueError('Routing LF prefix conflicts with CRLF')
        begin -= len(owned)
    return begin, last.end(), prefix

def plan(path, operation, canonical=b'', dedicated=False):
    path = Path(os.path.abspath(path))
    assert_path(path)
    if operation not in ('install', 'remove'):
        raise ValueError('Invalid routing operation')
    exists = path.exists()
    original = path.read_bytes() if exists else None
    text = original.decode('utf-8') if exists else ''
    bom = '\ufeff' if text.startswith('\ufeff') else ''
    text = text[len(bom):]
    if dedicated and re.match(r'^---(?:\r?\n|$)', text):
        raise ValueError('Dedicated routing frontmatter needs manual review')
    owned = section(text)
    if dedicated and exists and text and owned is None:
        raise ValueError('Unowned dedicated routing file')
    newline = '\r\n' if '\r\n' in text else '\n'
    result = text
    if operation == 'remove':
        if owned:
            result = text[:owned[0]] + text[owned[1]:]
    else:
        body = canonical.decode('utf-8').rstrip('\r\n').replace('\r\n', '\n').replace('\n', newline)
        if 'mmhmem:routing:' in body:
            raise ValueError('Canonical instructions contain ownership markers')
        prefix = owned[2] if owned else 'none'
        if not owned and text and not text.endswith('\n'):
            prefix = 'crlf' if newline == '\r\n' else 'lf'
        separator = {'none': '', 'lf': '\n', 'crlf': '\r\n'}[prefix]
        block = separator + START + newline + f'<!-- mmhmem:routing:prefix={prefix} -->' + newline + body + newline + END + newline
        result = text[:owned[0]] + block + text[owned[1]:] if owned else text + block
    data = None
    outcome = 'changed'
    if operation == 'remove' and not exists:
        outcome = 'unchanged'
    elif operation == 'remove' and dedicated and not result and owned:
        outcome = 'delete'
    else:
        data = (bom + result).encode('utf-8')
        if original == data:
            outcome = 'unchanged'
    return dict(path=path, exists=exists, original=original, outcome=outcome, bytes=data)

def recheck(value):
    path = value['path']
    assert_path(path)
    if path.exists() != value['exists'] or (path.exists() and path.read_bytes() != value['original']):
        raise ValueError('Routing target changed after preflight')

def commit(value, dry_run=False, before_commit=None, fail_at=''):
    if dry_run or value['outcome'] == 'unchanged':
        return
    if before_commit:
        before_commit()
    recheck(value)
    path = value['path']
    path.parent.mkdir(parents=True, exist_ok=True)
    stage = None
    try:
        if fail_at == 'backup':
            raise OSError('Injected backup failure')
        if value['exists']:
            backup = Path(str(path) + '.mmhmem.bak')
            assert_path(backup)
            stage = write_stage(path.parent, value['original'])
            os.replace(stage, backup)
            stage = None
        if fail_at == 'stage':
            raise OSError('Injected stage failure')
        if value['outcome'] != 'delete':
            stage = write_stage(path.parent, value['bytes'])
        if fail_at == 'replace':
            raise OSError('Injected replace failure')
        recheck(value)
        if value['outcome'] == 'delete':
            path.unlink()
        elif value['exists']:
            os.replace(stage, path)
        else:
            os.link(stage, path)  # Atomic no-clobber creation on the same filesystem.
    finally:
        if stage is not None and stage.exists():
            stage.unlink()

def write_stage(directory, data):
    descriptor, name = tempfile.mkstemp(prefix='.mmhmem-', suffix='.tmp', dir=directory)
    path = Path(name)
    try:
        with os.fdopen(descriptor, 'wb') as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
    except BaseException:
        path.unlink(missing_ok=True)
        raise
    return path

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=['install', 'remove'])
    parser.add_argument('path', type=Path)
    parser.add_argument('--canonical', type=Path)
    parser.add_argument('--dedicated', action='store_true')
    parser.add_argument('--dry-run', action='store_true')
    parser.add_argument('--plan-only', action='store_true')
    args = parser.parse_args()
    value = plan(args.path, args.operation, args.canonical.read_bytes() if args.canonical else b'', args.dedicated)
    if not args.plan_only:
        commit(value, args.dry_run)
    print(json.dumps({'path': str(value['path']), 'outcome': value['outcome']}))

if __name__ == '__main__':
    main()
