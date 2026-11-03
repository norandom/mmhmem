#!/usr/bin/env python3
"""Show, install, remove and report mmhmem's read-routing hook entry (Linux and macOS).

Usage: python3 manage_hooks.py <show|install|remove|status> --harness claude --binary <abs mmhmem>
       [--settings-path FILE] [--managed-settings-path FILE] [--dry-run]

Edits are textual splices on a position-tracking strict (RFC 8259) JSON parse, so every
byte outside mmhmem's own entry stays as it was and removal undoes an install byte for
byte. Python 3.8+ standard library only.
"""
import argparse
import getpass
import json
import os
import plistlib
import posixpath
import re
import shlex
import stat
import sys
import tempfile


class HelperError(Exception):
    pass


# --- position-tracking JSON parser --------------------------------------------------
# Nodes: kind o (object), a (array), s, n, b, z (null); start / exclusive end offsets.
class Node(object):
    __slots__ = ('kind', 'start', 'end', 'value', 'members', 'items', 'counts')

    def __init__(self, kind, start, end=0, value=None):
        self.kind, self.start, self.end, self.value = kind, start, end, value
        self.members, self.items, self.counts = [], [], {}


class Member(object):
    __slots__ = ('key', 'start', 'end', 'value')

    def __init__(self, key, start, end, value):
        self.key, self.start, self.end, self.value = key, start, end, value


STRING_TOKEN = re.compile(r'"(?:[^"\\\x00-\x1f]|\\(?:["\\/bfnrt]|u[0-9a-fA-F]{4}))*"')
NUMBER_TOKEN = re.compile(r'-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?')
LITERALS = (('true', 'b', True), ('false', 'b', False), ('null', 'z', None))


class Parser(object):
    def __init__(self, text):
        self.text, self.pos = text, 0

    def space(self):
        text, pos = self.text, self.pos
        while pos < len(text) and text[pos] in ' \t\r\n':
            pos += 1
        self.pos = pos

    def peek(self):
        return self.text[self.pos] if self.pos < len(self.text) else ''

    def string(self):
        match = STRING_TOKEN.match(self.text, self.pos)
        if not match:
            raise ValueError('invalid string at offset %d' % self.pos)
        node = Node('s', self.pos, match.end(), json.loads(match.group(0)))
        self.pos = match.end()
        return node

    def value(self):
        self.space()
        if self.pos >= len(self.text):
            raise ValueError('unexpected end of file')
        start, char = self.pos, self.text[self.pos]
        if char == '{':
            node = Node('o', start)
            self.pos += 1
            self.space()
            if self.peek() == '}':
                self.pos += 1
            else:
                while True:
                    self.space()
                    if self.peek() != '"':
                        raise ValueError('expected a key at offset %d' % self.pos)
                    key = self.string()
                    self.space()
                    if self.peek() != ':':
                        raise ValueError("expected ':' at offset %d" % self.pos)
                    self.pos += 1
                    value = self.value()
                    node.counts[key.value] = node.counts.get(key.value, 0) + 1
                    node.members.append(Member(key.value, key.start, value.end, value))
                    self.space()
                    if self.peek() == ',':
                        self.pos += 1
                        continue
                    if self.peek() == '}':
                        self.pos += 1
                        break
                    raise ValueError("expected ',' or '}' at offset %d" % self.pos)
            node.end = self.pos
            return node
        if char == '[':
            node = Node('a', start)
            self.pos += 1
            self.space()
            if self.peek() == ']':
                self.pos += 1
            else:
                while True:
                    node.items.append(self.value())
                    self.space()
                    if self.peek() == ',':
                        self.pos += 1
                        continue
                    if self.peek() == ']':
                        self.pos += 1
                        break
                    raise ValueError("expected ',' or ']' at offset %d" % self.pos)
            node.end = self.pos
            return node
        if char == '"':
            return self.string()
        for literal, kind, value in LITERALS:
            if self.text.startswith(literal, start):
                self.pos += len(literal)
                return Node(kind, start, self.pos, value)
        match = NUMBER_TOKEN.match(self.text, start)
        if not match:
            raise ValueError('unexpected character at offset %d' % start)
        self.pos = match.end()
        return Node('n', start, self.pos, match.group(0))


def parse_json(text):
    parser = Parser(text)
    try:
        root = parser.value()
    except RecursionError:
        raise ValueError('nesting too deep')
    parser.space()
    if parser.pos != len(text):
        raise ValueError('unexpected content at offset %d' % parser.pos)
    return root


def member(node, key):
    """Last member wins, as in JSON.parse; callers that edit a key refuse duplicates."""
    if node is None or node.kind != 'o':
        return None
    for item in reversed(node.members):
        if item.key == key:
            return item
    return None


def assert_single_key(node, key, where):
    if node.counts.get(key, 0) > 1:
        raise ValueError("duplicate key '%s' in %s" % (key, where))


def plain(node):
    """The parsed value as Python data (last duplicate wins)."""
    if node.kind == 'o':
        return dict((item.key, plain(item.value)) for item in node.members)
    if node.kind == 'a':
        return [plain(item) for item in node.items]
    return node.value


# --- JSON writer matching JSON.stringify(value, null, unit) ------------------------
class Style(object):
    def __init__(self, newline='\n', unit='  ', pretty=True):
        self.newline, self.unit, self.pretty = newline, unit, pretty
        self.colon = ': ' if pretty else ':'


ESCAPES = {'"': '\\"', '\\': '\\\\', '\b': '\\b', '\f': '\\f', '\n': '\\n', '\r': '\\r', '\t': '\\t'}


def json_string(value):
    out = []
    for char in value:
        if char in ESCAPES:
            out.append(ESCAPES[char])
        elif ord(char) < 32:
            out.append('\\u%04x' % ord(char))
        else:
            out.append(char)
    return '"' + ''.join(out) + '"'


def json_text(value, indent, style):
    if isinstance(value, dict):
        if not value:
            return '{}'
        inner = indent + style.unit
        parts = [json_string(key) + style.colon + json_text(item, inner, style) for key, item in value.items()]
        if not style.pretty:
            return '{' + ','.join(parts) + '}'
        return '{' + style.newline + inner + (',' + style.newline + inner).join(parts) + style.newline + indent + '}'
    if isinstance(value, list):
        if not value:
            return '[]'
        inner = indent + style.unit
        parts = [json_text(item, inner, style) for item in value]
        if not style.pretty:
            return '[' + ','.join(parts) + ']'
        return '[' + style.newline + inner + (',' + style.newline + inner).join(parts) + style.newline + indent + ']'
    if isinstance(value, str):
        return json_string(value)
    if isinstance(value, bool):
        return 'true' if value else 'false'
    return str(value)


# --- text splices that keep the surrounding formatting ---------------------------------
def elements(container):
    return container.members if container.kind == 'o' else container.items


def line_indent(text, offset):
    line_start = text.rfind('\n', 0, offset) + 1 if offset > 0 else 0
    return re.match(r'[ \t]*', text[line_start:]).group(0)


def file_style(text):
    newline = '\r\n' if '\r\n' in text else '\n'
    unit = '  '
    match = re.search(r'[\{\[][ \t]*\r?\n([ \t]+)\S', text)
    if match:
        unit = match.group(1)[len(line_indent(text, match.start())):] or '  '
    return Style(newline, unit, '\n' in text)


def add_element(text, container, key, value, style):
    """Appends an element (an object member when key is set) after the container's last element."""
    items = elements(container)
    if items:
        last = items[-1]
        if len(items) >= 2:
            separator = text[items[-2].end:last.start]
        else:
            separator = ',' + text[container.start + 1:last.start]
        pretty = '\n' in separator
        indent = separator[separator.rfind('\n') + 1:] if pretty else ''
        local = Style(style.newline, style.unit, pretty)
        body = json_text(value, indent, local)
        if key is not None:
            body = json_string(key) + local.colon + body
        return text[:last.end] + separator + body + text[last.end:]
    own = line_indent(text, container.start)
    pretty = style.pretty
    indent = own + style.unit if pretty else ''
    local = Style(style.newline, style.unit, pretty)
    body = json_text(value, indent, local)
    if key is not None:
        body = json_string(key) + local.colon + body
    if pretty:
        body = style.newline + indent + body + style.newline + own
    return text[:container.start + 1] + body + text[container.end - 1:]


def remove_element(text, container, index):
    """Removes one element with the separator that add_element would have added for it."""
    items = elements(container)
    if len(items) == 1:
        return text[:container.start + 1] + text[container.end - 1:]
    if index > 0:
        cut_from, cut_to = items[index - 1].end, items[index].end
    else:
        cut_from, cut_to = items[0].start, items[1].start
    return text[:cut_from] + text[cut_to:]


# --- harness definitions (codex and opencode are added by later tasks) ---------------------
HARNESSES = {
    'claude': {
        'label': 'Claude Code',
        'matcher': 'Read|Bash|PowerShell',
        'args': ['hook', 'claude'],
        'timeout': 10,
        'approval': 'Claude Code holds back hooks until you accept the workspace trust dialog in an '
                    'interactive session; review the entry with /hooks.',
        'sees': "Before each Read, Bash or PowerShell tool call, Claude Code passes the hook the tool name and "
                "its input: the file path with any offset and limit for Read, or the command text for Bash and "
                "PowerShell. The hook looks only at that file's size and line count; it declines (pointing the "
                "agent to mmhmem's smart_read) only full reads of large files that are not security-critical, and "
                "only while an mmhmem worker is configured. Every other call, and any hook error or timeout, "
                "proceeds unchanged. It never returns or records file content, never reads secrets such as the "
                "worker API key, and makes no network connections.",
    },
}

# Managed settings locations: code.claude.com/docs/en/managed-settings ("Where each mechanism
# stores the policy"): /Library/Application Support/ClaudeCode/ on macOS, /etc/claude-code/ on
# Linux and WSL, each with an optional managed-settings.d/ drop-in directory.
MANAGED_DIRS = {'darwin': '/Library/Application Support/ClaudeCode'}
MANAGED_DIR_DEFAULT = '/etc/claude-code'
PREFERENCE_DOMAIN = 'com.anthropic.claudecode'


def default_managed_path(platform=None):
    platform = platform or sys.platform
    return posixpath.join(MANAGED_DIRS.get(platform, MANAGED_DIR_DEFAULT), 'managed-settings.json')


def managed_preference_paths(platform=None, user=None):
    """macOS configuration-profile preferences for the com.anthropic.claudecode domain.

    The docs name the domain, not a file; MDM-installed profiles are materialised under
    /Library/Managed Preferences (device-wide and per user), readable without privileges.
    """
    if (platform or sys.platform) != 'darwin':
        return []
    base = '/Library/Managed Preferences'
    paths = [posixpath.join(base, PREFERENCE_DOMAIN + '.plist')]
    if user:
        paths.append(posixpath.join(base, user, PREFERENCE_DOMAIN + '.plist'))
    return paths


def user_config_dir():
    configured = os.environ.get('CLAUDE_CONFIG_DIR')
    if configured:
        return os.path.abspath(configured)
    return os.path.join(os.environ.get('HOME') or os.path.expanduser('~'), '.claude')


def make_entry(definition, binary):
    handler = {'type': 'command', 'command': binary, 'args': list(definition['args']),
               'timeout': definition['timeout']}
    return {'matcher': definition['matcher'], 'hooks': [handler]}


def is_own_handler(handler, definition):
    if handler.kind != 'o':
        return False
    command, args = member(handler, 'command'), member(handler, 'args')
    if command is None or command.value.kind != 's' or args is None or args.value.kind != 'a':
        return False
    name = re.split(r'[\\/]', command.value.value)[-1].lower()
    if name not in ('mmhmem', 'mmhmem.exe'):
        return False
    items = args.value.items
    return len(items) == len(definition['args']) and all(
        item.kind == 's' and item.value == expected for item, expected in zip(items, definition['args']))


class Own(object):
    def __init__(self):
        self.hooks = self.pre = None
        self.found = []  # (group index, handlers array node, handler index, handler node)


def find_own(root, definition):
    """The hook containers and every identified handler; raises when the shape is not editable."""
    if root.kind != 'o':
        raise ValueError('settings root is not a JSON object')
    assert_single_key(root, 'hooks', 'settings')
    own = Own()
    hooks = member(root, 'hooks')
    if hooks is None:
        return own
    if hooks.value.kind != 'o':
        raise ValueError('"hooks" is not an object')
    assert_single_key(hooks.value, 'PreToolUse', 'hooks')
    own.hooks = hooks
    pre = member(hooks.value, 'PreToolUse')
    if pre is None:
        return own
    if pre.value.kind != 'a':
        raise ValueError('"hooks.PreToolUse" is not an array')
    own.pre = pre
    for group_index, group in enumerate(pre.value.items):
        handlers = member(group, 'hooks')
        if handlers is None or handlers.value.kind != 'a':
            continue
        for index, handler in enumerate(handlers.value.items):
            if is_own_handler(handler, definition):
                own.found.append((group_index, handlers.value, index, handler))
    return own


def remove_own_handler(text, definition, which):
    """Removes the first or last identified handler, then its group, the PreToolUse array and
    the hooks object as each becomes empty."""
    root = parse_json(text)
    own = find_own(root, definition)
    group_index, handlers, index, _ = own.found[0] if which == 'first' else own.found[-1]
    if len(handlers.items) > 1:
        return remove_element(text, handlers, index)
    pre = own.pre.value
    if len(pre.items) > 1:
        return remove_element(text, pre, group_index)
    hooks = own.hooks.value
    if len(hooks.members) > 1:
        return remove_element(text, hooks, hooks.members.index(own.pre))
    return remove_element(text, root, root.members.index(own.hooks))


# --- settings file I/O ---------------------------------------------------------------------
POSIX = os.name == 'posix'
# Top-level links an OS creates itself: macOS /var, /tmp, /etc -> /private/...; merged-/usr
# layouts link /bin, /sbin and /lib* into /usr. root's own HOME on macOS is /var/root.
SYSTEM_LINKS = frozenset(('var', 'tmp', 'etc', 'bin', 'sbin', 'lib', 'lib32', 'lib64', 'libx32'))


def _is_link(path):
    return os.path.islink(path)


def _link_owner(path):
    return os.lstat(path).st_uid


def _euid():
    return os.geteuid()


def system_link(path):
    """True for a root-owned parent link that this user cannot have planted.

    A non-root user cannot create root-owned links, so those are system layout. When the
    helper runs as root, every link it meets is root-owned, so only the top-level system
    layout links in SYSTEM_LINKS are accepted."""
    if not POSIX or _link_owner(path) != 0:
        return False
    if _euid() != 0:
        return True
    parent = os.path.dirname(path)
    return os.path.dirname(parent) == parent and os.path.basename(path) in SYSTEM_LINKS


def assert_plain_path(path):
    """Refuses a symlinked target or a symlinked parent directory (see system_link)."""
    current = os.path.abspath(path)
    first = True
    while True:
        if _is_link(current) and (first or not system_link(current)):
            raise HelperError('%s is a symbolic link; edit the hook entry manually. Nothing was changed.'
                              % current)
        parent = os.path.dirname(current)
        if parent == current:
            return
        current, first = parent, False


class Settings(object):
    def __init__(self, path):
        self.path = path
        self.exists = os.path.isfile(path)
        self.raw = None
        self.mode = None
        self.bom = ''
        self.text = ''
        self.root = None


def read_settings(path):
    state = Settings(path)
    if state.exists:
        with open(path, 'rb') as handle:
            state.raw = handle.read()
        state.mode = stat.S_IMODE(os.stat(path).st_mode)
        try:
            text = state.raw.decode('utf-8')
        except UnicodeDecodeError:
            raise ValueError('%s is not valid UTF-8' % path)
        if text.startswith('\ufeff'):
            state.bom, text = '\ufeff', text[1:]
        state.text = text
    if not state.text.strip(' \t\r\n'):
        return state
    try:
        state.root = parse_json(state.text)
    except ValueError as error:
        raise ValueError('%s is not valid JSON (%s)' % (path, error))
    return state


def fsync_directory(directory):
    if os.name != 'posix':
        return
    descriptor = os.open(directory, os.O_RDONLY)
    try:
        os.fsync(descriptor)
    except OSError:
        pass
    finally:
        os.close(descriptor)


def write_private(path, data, mode, owner):
    """Writes a new file in place of path through a same-directory staging file."""
    directory = os.path.dirname(path)
    descriptor, stage = tempfile.mkstemp(prefix='.mmhmem-', suffix='.tmp', dir=directory)
    try:
        with os.fdopen(descriptor, 'wb') as handle:
            handle.write(data)
            handle.flush()
            os.fsync(handle.fileno())
        os.chmod(stage, mode)
        if owner and hasattr(os, 'chown') and os.geteuid() == 0:
            os.chown(stage, owner[0], owner[1])
        os.replace(stage, path)
    finally:
        if os.path.exists(stage):
            os.unlink(stage)


def save_settings(state, text):
    path = state.path
    assert_plain_path(path)
    backup = path + '.mmhmem.bak'
    if os.path.islink(backup):
        raise HelperError('%s is a symbolic link; nothing was written' % backup)
    data = (state.bom + text).encode('utf-8')
    directory = os.path.dirname(path)
    if not os.path.isdir(directory):
        os.makedirs(directory, 0o700)
    assert_plain_path(path)
    changed = os.path.isfile(path) != state.exists
    if not changed and state.exists:
        with open(path, 'rb') as handle:
            changed = handle.read() != state.raw
    if changed:
        raise HelperError('%s changed while mmhmem was editing it; nothing was written' % path)
    owner = None
    if state.exists:
        info = os.stat(path)
        owner = (info.st_uid, info.st_gid)
        write_private(backup, state.raw, state.mode, owner)
    # A new file is private: user settings may carry env values and credentials helpers.
    write_private(path, data, state.mode if state.exists else 0o600, owner)
    fsync_directory(directory)


# --- policy ------------------------------------------------------------------------------
def policy_block(data, source, managed):
    if not isinstance(data, dict):
        return None
    if data.get('disableAllHooks') is True:
        return 'disableAllHooks: true in %s' % source
    if not managed:
        return None
    if data.get('allowManagedHooksOnly') is True:
        return 'allowManagedHooksOnly: true in %s' % source
    if 'strictPluginOnlyCustomization' in data:
        strict = data['strictPluginOnlyCustomization']
        if strict is True:
            return 'strictPluginOnlyCustomization: true in %s' % source
        if isinstance(strict, list) and 'hooks' in [item for item in strict if isinstance(item, str)]:
            return 'strictPluginOnlyCustomization includes "hooks" in %s' % source
        # Claude Code treats a value that is neither a boolean nor an array as locking every surface.
        if not isinstance(strict, (bool, list)):
            return 'strictPluginOnlyCustomization (non-boolean value) in %s' % source
    return None


def _read_bytes(path):
    with open(path, 'rb') as handle:
        return handle.read()


def _list_dir(path):
    return os.listdir(path)


def note_denied(path):
    """Claude Code starts without a managed source the OS denies it and records that in /status."""
    sys.stderr.write('[mmhmem] note: managed settings %s cannot be read by this user (permission denied); '
                     'Claude Code also runs without it, so it was not checked.\n' % path)


def managed_files(managed_path):
    files = [managed_path] if os.path.isfile(managed_path) else []
    drop_ins = os.path.join(os.path.dirname(managed_path), 'managed-settings.d')
    if os.path.isdir(drop_ins):
        try:
            names = sorted(_list_dir(drop_ins))
        except PermissionError:
            note_denied(drop_ins)
            names = []
        # Claude Code ignores hidden files and files that do not end in .json.
        files += [os.path.join(drop_ins, name) for name in names
                  if name.endswith('.json') and not name.startswith('.')
                  and os.path.isfile(os.path.join(drop_ins, name))]
    return files


def managed_blocks(managed_path, preference_paths):
    blocks = []
    for path in managed_files(managed_path):
        source = 'managed settings %s' % path
        try:
            text = _read_bytes(path).decode('utf-8')
        except PermissionError:
            note_denied(path)
            continue
        except (OSError, UnicodeDecodeError):
            blocks.append('%s cannot be parsed; Claude Code refuses to start until it is fixed' % source)
            continue
        text = text[1:] if text.startswith('\ufeff') else text
        if not text.strip(' \t\r\n'):
            continue  # an empty managed settings file counts as {}
        try:
            data = plain(parse_json(text))
        except ValueError:
            blocks.append('%s cannot be parsed; Claude Code refuses to start until it is fixed' % source)
            continue
        if not isinstance(data, dict):
            blocks.append('%s is not a JSON object; Claude Code refuses to start until it is fixed' % source)
            continue
        block = policy_block(data, source, True)
        if block:
            blocks.append(block)
    for path in preference_paths:
        if not os.path.isfile(path):
            continue
        source = 'managed preferences %s' % path
        try:
            data = plistlib.loads(_read_bytes(path))
        except PermissionError:
            note_denied(path)
            continue
        except Exception:  # plistlib raises several types for malformed input
            blocks.append('%s cannot be parsed; Claude Code refuses to start until it is fixed' % source)
            continue
        if not isinstance(data, dict):
            blocks.append('%s is not a dictionary; Claude Code refuses to start until it is fixed' % source)
            continue
        block = policy_block(data, source, True)
        if block:
            blocks.append(block)
    return blocks


# --- commands ------------------------------------------------------------------------------
def fail(message):
    sys.stderr.write('[mmhmem] %s\n' % message)
    sys.stderr.flush()
    return 1


def say(text):
    sys.stdout.write(text + '\n')


def current_user():
    try:
        return getpass.getuser()
    except Exception:
        return None


def run(options):
    definition = HARNESSES[options.harness]
    label = definition['label']
    user_dir = user_config_dir()
    target = os.path.abspath(options.settings_path) if options.settings_path else os.path.join(user_dir, 'settings.json')
    if options.managed_settings_path:
        managed_path = os.path.abspath(options.managed_settings_path)
        preferences = []  # an explicit managed path replaces every OS policy source
    else:
        managed_path = default_managed_path()
        preferences = managed_preference_paths(sys.platform, current_user())
    prefix = '[dry-run] ' if options.dry_run else ''

    # User scope only: a project's .claude directory or a local settings file is never a target.
    target_dir = os.path.dirname(target)
    if os.path.basename(target).lower() == 'settings.local.json' or (
            os.path.basename(target_dir).lower() == '.claude'
            and os.path.normcase(target_dir.rstrip('/\\')) != os.path.normcase(user_dir.rstrip('/\\'))):
        return fail('%s looks like project settings; mmhmem only adds hooks to the user settings file. '
                    'Nothing was changed.' % target)

    binary = options.binary
    if options.command in ('show', 'install'):
        if not binary:
            return fail('--binary <absolute path to mmhmem> is required.')
        if not binary.startswith('/'):
            return fail('--binary must be an absolute path: %s' % binary)
        binary = posixpath.normpath(binary)
        if posixpath.basename(binary) not in ('mmhmem', 'mmhmem.exe'):
            return fail('--binary must point to mmhmem: %s' % binary)
        try:
            binary.encode('utf-8')
        except UnicodeEncodeError:
            return fail('--binary is not valid UTF-8: %r' % binary)
    entry = make_entry(definition, binary) if binary else None
    remove_command = ' '.join(shlex.quote(part) for part in (
        ['python3', os.path.abspath(__file__), 'remove', '--harness', options.harness]
        + (['--settings-path', target] if options.settings_path else [])))

    try:
        state = read_settings(target)
    except (ValueError, OSError) as error:
        return fail('%s. Nothing was changed.' % error)
    try:
        own = find_own(state.root, definition) if state.root is not None else Own()
    except ValueError as error:
        return fail('%s cannot be edited safely: %s. Nothing was changed.' % (target, error))

    def blocks():
        found = []
        user_block = policy_block(plain(state.root) if state.root is not None else None,
                                  'user settings %s' % target, False)
        if user_block:
            found.append(user_block)
        return found + managed_blocks(managed_path, preferences)

    if options.command == 'show':
        say('[mmhmem] %s read-routing hook' % label)
        say('File: %s' % target)
        say('Entry appended to hooks.PreToolUse:')
        say(json_text(entry, '', Style()))
        say('What it sees and decides: %s' % definition['sees'])
        say('Remove it with: %s' % remove_command)
        return 0

    if options.command == 'status':
        found_blocks = blocks()
        present = bool(own.found)
        if found_blocks:
            note = ' (entry present in the settings file)' if present else ''
            say('%s: policy-blocked by %s%s' % (options.harness, '; '.join(found_blocks), note))
        elif present:
            say('%s: installed in %s (%s)' % (options.harness, target,
                                              member(own.found[0][3], 'command').value.value))
        else:
            say('%s: not installed (%s)' % (options.harness, target))
        return 0

    text = state.text
    if options.command == 'install':
        found_blocks = blocks()
        if found_blocks:
            return fail('%s hooks are disabled or restricted by policy: %s. The hook was not added.'
                        % (label, '; '.join(found_blocks)))
        message = ''
        if not own.found:
            if state.root is None:
                text = json_text({'hooks': {'PreToolUse': [entry]}}, '', Style()) + '\n'
            else:
                style = file_style(text)
                if own.pre is not None:
                    text = add_element(text, own.pre.value, None, entry, style)
                elif own.hooks is not None:
                    text = add_element(text, own.hooks.value, 'PreToolUse', [entry], style)
                else:
                    text = add_element(text, state.root, 'hooks', {'PreToolUse': [entry]}, style)
            message = 'added the read-routing hook to %s' % target
        else:
            command = member(own.found[0][3], 'command').value
            if command.value != binary:
                text = text[:command.start] + json_string(binary) + text[command.end:]
                message = "replaced the hook's binary path %s with %s in %s" % (command.value, binary, target)
            # Duplicates (from manual edits) are removed so exactly one entry remains.
            while len(find_own(parse_json(text), definition).found) > 1:
                text = remove_own_handler(text, definition, 'last')
        if text == state.text:
            say('[mmhmem] %s hook already installed in %s (unchanged)' % (label, target))
            return 0
        message = message or 'removed duplicate read-routing hook entries from %s' % target
        if not options.dry_run:
            try:
                save_settings(state, text)
            except (HelperError, OSError) as error:
                return fail(str(error))
        say('[mmhmem] %s%s' % (prefix, message))
        if state.exists and not options.dry_run:
            say('[mmhmem] backup: %s.mmhmem.bak' % target)
        say('[mmhmem] %s' % definition['approval'])
        say('[mmhmem] Remove it with: %s' % remove_command)
        return 0

    # remove
    if state.root is not None:
        while find_own(parse_json(text), definition).found:
            text = remove_own_handler(text, definition, 'first')
    if text == state.text:
        say('[mmhmem] no %s read-routing hook in %s (unchanged)' % (label, target))
        return 0
    if not options.dry_run:
        try:
            save_settings(state, text)
        except (HelperError, OSError) as error:
            return fail(str(error))
    say('[mmhmem] %sremoved the read-routing hook from %s' % (prefix, target))
    if not options.dry_run:
        say('[mmhmem] backup: %s.mmhmem.bak' % target)
    return 0


def main(argv=None):
    parser = argparse.ArgumentParser(description="Manage mmhmem's read-routing hook entry.")
    parser.add_argument('command', choices=['show', 'install', 'remove', 'status'])
    parser.add_argument('--harness', choices=sorted(HARNESSES), default='claude')
    parser.add_argument('--binary')
    parser.add_argument('--settings-path')
    parser.add_argument('--managed-settings-path')
    parser.add_argument('--dry-run', action='store_true')
    return run(parser.parse_args(argv))


if __name__ == '__main__':
    sys.exit(main())
