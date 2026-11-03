# mmhmem file reading

For ordinary repository inspection, when mmhmem is available and authorized,
use `smart_read` before a full native or shell read. Reuse the exact project identity.
For edits or verification, request exact source with `offset` and `limit`, or a
bounded native line range. Summaries are not exact evidence.

If mmhmem is unavailable, continue with authorized native reads. If a call fails
or its worker is unavailable, do not retry the same failing request; use a bounded,
authorized native read or targeted slice. Respect permissions and allowed roots.
Explicit user tool preferences prevail. Keep native search, build, test, and edit
workflows, and follow existing checkpoint instructions.

Do not claim savings from a connection or an attempted read; report only recorded
results. Cache reuse depends on matching content and cache identity, including the
question. This guidance does not guarantee model compliance.
