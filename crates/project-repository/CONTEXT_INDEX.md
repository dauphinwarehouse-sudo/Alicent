# Alicent context-index prototype

This module is the first isolated P4.2 slice for stable text chunks and bounded lexical retrieval. It lives beside the project repository so it can use the same audited SQLite and hashing dependencies without changing project format v5 yet.

## Guarantees

- UTF-8-safe chunks are bounded to 4 KiB and prefer paragraph/content-defined boundaries.
- Unchanged chunk text keeps its previous `chunk_id` across a localized document edit.
- Every update is revision-guarded; stale revisions and changed payloads for the same revision are rejected.
- SQLite FTS5 stores only indexed, non-excluded documents.
- Exclusion removes document text and FTS rows in one transaction.
- Search is literal, bounded to 50 results, and can independently exclude the active document.
- Empty and unchanged documents do not create unnecessary chunk rows.

## Current boundary

`src/context_index.rs` is compiled through `tests/context_index.rs` but is intentionally not exported from `alicent-project-repository` yet. The next integration PR must decide whether the index remains a disposable sidecar or becomes project schema v6, then connect save/archive/policy changes to a bounded background refresh queue.

Summaries, token-provider-specific estimation, embeddings, semantic ranking and provider disclosure remain out of scope for this slice.
