# Requirements Document

## Project Description (Input)

`mmhmem stats` shows few tokens saved although the owner runs many agents
in focused sessions over the same code bases. The audit log of the owner's
machine (2026-11-10) explains why:

- The summary cache is keyed by file path, content hash and the free-text
  `question` the agent passes. Agents word the question freely: 144 cache
  rows carry 105 distinct questions, so identical content was summarised
  again 22 times, mostly by another agent hours later. Only 13 of 157
  summary requests were cache hits. Warm-up fills the cache under one
  question and therefore rarely helps another agent.
- The read-routing hook redirects whole-file reads of large files, but any
  read with an offset or limit passes regardless of its size. 273 of 404
  redirects were followed within minutes by a native ranged read of the
  same file, and the redirect text itself suggests that fallback. Only 34
  redirects became `smart_read` summaries.
- A first-time summary costs more worker tokens than it saves on first use
  (0.67 avoided frontier tokens per worker input token); savings come only
  from re-use, which the two points above prevent. The median summarised
  file has 310 lines; at the 150-line threshold many summaries save little.
- The file cache has no hit counter, no size cap and no visibility in
  `mmhmem stats`, so none of this was visible without reading the audit
  log.

The owner chose, from a menu on 2026-11-10: a content-only cache key with
custom questions as extra answers, closing the ranged-read escape, cache
and redirect visibility in the stats, per-project warm-up, and raising the
threshold to 300 lines.

## Introduction

This specification makes the read-routing savings real for a user who runs
several agents over the same repositories: one cached summary per file
content that every agent and the warm-up share, a hook that no longer lets
large ranged reads through, a threshold where summaries pay off, a warm-up
that covers a whole repository in one command, and statistics that show the
cache hit rate and how often a redirect is followed.

## Boundary Context

- **In scope:** the summary cache's key, columns, cap and migration; how a
  custom `question` is answered and cached; the hook's treatment of ranged
  reads and its redirect texts for every harness; the default threshold and
  its documentation; the `mmhmem stats` columns; a per-repository warm-up
  command and its mention in the playbook; the prune command's compatibility
  with the new key.
- **Out of scope:** the worker model and its prompts; the `shunt_content`
  tool's own cache beyond sharing its table; harness installation (which
  hook files are written is `reliable-installers` / `antigravity-native-setup`);
  a new hook event type; cross-machine caches.
- **Adjacent expectations:** `read-routing-hooks` keeps owning the hook
  protocol per harness and the fail-open rule; `corpus-warmup-prune` keeps
  owning the `shunt_warmup` tool's batch contract and the prune rules, which
  this spec adapts to the new key; the statistics honesty discipline stays
  absolute (no synthetic rows, estimates labelled).

## Requirements

### Requirement 1: One cached summary per file content

**Objective:** As a user running several agents on the same repositories, I
want a file summarised once and reused by every agent, so that worker calls
stop repeating for unchanged content.

#### Acceptance Criteria

1. When a large file is read through `smart_read` without a question, or with the default question, the service shall serve the cached structural summary for that file content if one exists, whatever path, project or agent produced it.
2. When no cached structural summary exists for that content, the service shall produce one with the worker model and cache it under the content hash alone.
3. When the same content appears under another path (a copy, a worktree, a moved file), the service shall serve the cached summary without a worker call.
4. The service shall not expire a cached summary by age; a summary stays valid until its content changes or it is evicted by the size cap of Requirement 5.
5. When the store is opened by a version with the new key, summaries cached under the old key shall be carried over so that no worker call is repeated for content that was already summarised.

### Requirement 2: Custom questions are extra answers, never a cache miss for the summary

**Objective:** As an agent with a specific question about a file, I want an
answer to my question when I ask for one, without my wording hiding the
shared summary from me or from other agents.

#### Acceptance Criteria

1. When a `question` other than the default is given and a cached answer to that exact question exists for the content, the service shall serve that answer.
2. When a `question` other than the default is given, no answer to it is cached, and a structural summary for the content is cached, the service shall serve the structural summary and state in one line that the question was not applied and how to ask for a question-specific answer.
3. When the caller explicitly asks for a question-specific answer, the service shall produce it with the worker model, cache it under the content hash and the question, and leave the structural summary in place.
4. The tool description and the playbook shall tell agents to omit the question for the shared summary and to ask a question only when the summary is not enough.
5. Where a question-specific answer is cached, it shall count as a cache hit in the statistics like a structural summary.

### Requirement 3: Ranged reads of large spans are redirected

**Objective:** As a user who turned the hook on, I want large reads redirected
even when the agent passes an offset and a limit, so that the redirect cannot
be sidestepped by re-reading the whole file as a range.

#### Acceptance Criteria

1. When a read names a range whose span is at or above the threshold, the hook shall treat it as a whole-file read and apply the same decision rules as for a read without a range.
2. When a read names a range with no end (read to the end of the file) and the remaining lines from its start are at or above the threshold, the hook shall treat it as a whole-file read.
3. When a read names a range whose span is below the threshold, the hook shall let it pass.
4. The redirect text for every harness shall point first to `smart_read` for the summary and shall describe the ranged fallback as a range below the threshold around the lines needed, naming the threshold.
5. The hook shall keep failing open: a range it cannot interpret shall pass.
6. Where the harness sends exact line ranges that `smart_read` can serve as a slice, the redirect text shall say that a slice through `smart_read` is also available.

### Requirement 4: Threshold where summaries pay off

**Objective:** As an operator, I want the default threshold set where a
summary saves more than it costs, so that the worker is not spent on files a
slice would serve.

#### Acceptance Criteria

1. The default minimum file length for summarisation and for the hook's redirect shall be 300 lines.
2. The documented default, the installer's and the playbook's wording, and the project steering shall state 300 lines, and the environment variable shall keep overriding it.
3. When the threshold changes, existing cached summaries of files between the old and the new threshold shall keep being served if asked for, and shall not be removed.

### Requirement 5: Cache and redirect visibility

**Objective:** As an operator, I want to see whether the cache and the hook
are working, so that a regression like this one is visible without reading
the audit log.

#### Acceptance Criteria

1. The summary cache shall record, per entry, how often it was served and when it was last served.
2. The summary cache shall have a configurable size cap in rows with the same default as the content cache, and shall evict the least recently served entries beyond it; each eviction shall be audited.
3. `mmhmem stats` shall show a cache hit rate per row: served summaries divided by all summary requests (served plus produced), labelled as a rate.
4. `mmhmem stats` shall show redirect follow-through per row: the share of hook redirects followed within a short, fixed window by a `smart_read` call for the same file, labelled as a rate and computed from the same records as the existing activity counts.
5. Both rates shall stay out of the totals row when they cannot be aggregated meaningfully, and the hidden-column rules of the table shall apply to them as to the other activity columns.
6. The statistics shall remain honest: no synthetic rows, and every estimate labelled as an estimate.

### Requirement 6: Per-repository warm-up

**Objective:** As a user starting agents on a repository, I want one command
that warms the cache for the whole repository, so that every agent's first
read of a large file is a hit.

#### Acceptance Criteria

1. The mmhmem binary shall provide a `warmup` subcommand that takes a repository root (default: the current folder) and pushes every text file at or above the threshold under it through the same warm-up path as the `shunt_warmup` tool, in batches, with the default question.
2. The command shall skip files the security rule bypasses, files outside the allowed roots, non-text files and files ignored by the repository's version control when the root is a repository.
3. When a file's content is already cached, the command shall count it as a hit and make no worker call, so a second run costs nothing.
4. The command shall support a dry run that lists what would be summarised with the estimated number of worker calls, and a cap on the number of files per run with a clear message when the cap is reached.
5. The command shall report the outcome in one summary: files examined, hits, summaries produced, skipped by reason, failures, and worker tokens spent.
6. If no worker model is configured and any file would need a summary, then the command shall say so and exit without changes.
7. The playbook shall mention the command in one line without exceeding its token cap.

### Requirement 7: Prune stays correct with the new key

**Objective:** As an operator, I want `mmhmem prune` and `shunt_prune` to keep
removing stale rows, so that the cache does not carry dead weight after
refactors.

#### Acceptance Criteria

1. When a cached summary's last known file no longer exists, or its current content no longer matches the cached content, prune shall treat the row as stale as it does today.
2. When a cached summary's content still exists at any path prune knows for it, prune shall keep the row.
3. Prune's dry run, grouped report and audit records shall keep their current form.
