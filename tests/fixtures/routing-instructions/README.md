# Owned routing golden corpus

cases.json defines immutable raw-byte inputs and expected outputs for both helpers.
A null input means the target does not exist; a null expected output means no bytes
are returned (error, absent no-op, or dedicated-file deletion). Error adapters must
return error/null and separately assert the filesystem remains unchanged.

Use runner.py run_cases(adapter) or dot-source runner.ps1 and invoke
Invoke-RoutingGoldenCases -Adapter { ... }. Adapters invoke the real helper; they
must not derive results from expected files. Both runners compare outcome and exact
bytes. The corpus itself is validated with py -3.12 test_corpus.py from repo root.

Canonical whole-line markers wrap a prefix metadata comment. prefix=none owns no
preceding bytes; prefix=lf or prefix=crlf owns exactly that inserted preceding
newline. Validate its presence before removing it. Legacy blocks without metadata
own no preceding bytes. This makes appending to a file without a final newline
reversible without removing user whitespace. Dedicated nonempty unowned rules files
and dedicated frontmatter are conflicts. General AGENTS files retain unrelated text.

Golden output can be passed from one helper directly as input to the other helper
for repeat/update/remove verification. Files use .bin so Git must not normalize
newlines or encoding. Filesystem failures, reparse/symlink targets, backup rotation,
concurrency, and dry-run side effects require helper integration tests separately.
