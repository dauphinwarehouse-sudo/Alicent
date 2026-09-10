# Alicent context index

The context index is part of project schema v6. Project documents remain the source of truth; the stored chunks and FTS rows are a rebuildable local projection.

## Guarantees

- Only active, non-excluded scenes and notes are eligible. Folders, archived documents and excluded documents are never indexed.
- UTF-8-safe chunks are bounded to 4 KiB and prefer paragraph/content-defined boundaries.
- Unchanged chunk text keeps its previous `chunk_id` across localized edits.
- Each chunk and FTS row carries the source document revision.
- Save, version restore and checkpoint restore immediately hide stale FTS rows; bounded refresh rebuilds them from the current revision.
- Rename and metadata-only revisions update the indexed title and revision without rechunking.
- Archive and AI-context exclusion purge stored chunks and FTS rows in the same transaction as the policy change.
- Refresh is limited to 32 documents per transaction. Retrieval is literal, limited to 50 chunks and always excludes the current document.
- Schema validation rejects ineligible, future-revision, orphaned, duplicated or incorrectly fresh index rows.
- Migration from v5 creates and validates a standalone pre-migration backup before installing v6.

## Scope

This slice is local and lexical only. It does not add embeddings, summaries, provider calls, network access or UI behavior.
