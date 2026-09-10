# ADR 0014: Stable bounded chunks and disposable lexical context index

**Status:** accepted

## Context

ADR 0012 and ADR 0013 provide explicit references, persistent pins and a fail-closed exclusion flag. They still send whole selected documents and do not provide stable chunk identities, incremental invalidation or retrieval. Moving directly to embeddings would couple provider choice, project migrations and ranking before the local invariants are tested.

## Decision

Add an isolated repository-side prototype with these boundaries:

- split UTF-8 text into paragraph-aware, content-defined chunks targeting 2 KiB and never exceeding 4 KiB;
- derive deterministic IDs for new chunks and reuse prior IDs when identical chunk text survives a later document revision;
- store document revision, content hash, byte ranges, a rough token estimate and chunk text in SQLite;
- maintain a separate FTS5 table transactionally with chunk rows;
- skip byte-identical revisions, reject stale revisions and reject changed content presented with the same revision;
- delete all stored text and FTS rows immediately when a document is excluded;
- accept only literal bounded search and allow the caller to exclude the active document independently;
- keep the prototype outside project format v5 until save/archive/restore/policy invalidation and recovery behavior are specified together.

The implementation is compiled and exercised by integration tests, but is not exported as a production repository API in this PR.

## Consequences

Localized edits preserve unaffected chunk IDs, so later summaries or embeddings can be keyed by chunk identity instead of being recomputed for every document save. Lexical retrieval can be evaluated without network access or provider disclosure. The index can be deleted and rebuilt because project documents remain the source of truth.

A follow-up must choose sidecar versus schema-v6 storage, connect bounded background refresh to repository revisions, purge archive and exclusion changes fail-closed, and add crash/reopen/migration coverage. Summaries, semantic retrieval, embeddings and provider-specific token estimation remain future P4.2 work.
