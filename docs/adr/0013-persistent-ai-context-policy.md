# ADR 0013: Persistent per-document AI context policy

**Status:** accepted

## Context

ADR 0012 added explicit bounded references for one editor request. Selection disappeared when the author changed documents or reopened a project, and there was no persistent way to prevent a sensitive note from entering model context.

## Decision

Persist two mutually exclusive document flags in project schema v5:

- `ai_context_pinned` makes an active scene or note an automatically selected reference;
- `ai_context_excluded` removes a document from reference candidates and prevents it from being the generation target;
- no more than eight active documents may be pinned across the project;
- policy changes use revision guards, operation journaling and idempotent receipts;
- archiving clears pins, while exclusion survives archive/restore;
- duplication inherits exclusion but never a pin.

The renderer merges pinned documents with the currently visible list, de-duplicates IDs, and keeps manual per-request selection possible. It validates exclusion both from the summary and from the freshly read document. Existing native request limits remain the final independent boundary.

## Consequences

Authors get durable, explicit control without semantic retrieval or whole-project loading. Pinning is a convenience default, not permission to edit a reference. Exclusion is enforced for the current editor flow, but future tools and agent runtime must independently authorize every document ID.

Stable chunks, summaries, incremental invalidation, token estimation and semantic retrieval remain future P4.2 work.
