# ADR 0012: Explicit bounded project context for editor generation

**Status:** accepted

## Context

The first AI editor slice could revise only the active document. The product specification requires relevant multi-document context without loading the whole project into memory or sending it to a provider. Stable chunks, summaries and semantic retrieval are not implemented yet.

## Decision

Add an explicit intermediate context slice:

- the user selects up to eight scenes or notes from the currently loaded document list;
- only selected documents are read, one at a time, when generation starts;
- the renderer rejects stale, duplicate, folder and over-limit selections and caps reference text at 512 KiB;
- the native provider boundary independently rejects more than eight references, duplicate/current-document IDs and a combined request larger than 2 MiB;
- the provider prompt identifies the active document as the only edit target and reference documents as read-only context;
- the UI states exactly how many documents will be sent.

This preserves lazy project access and explicit network disclosure. Context is not inferred from unrelated documents, and a reference can never become the replacement target.

## Consequences

Authors can maintain continuity across nearby scenes before the semantic context engine exists. Selection is intentionally temporary and limited to the current list. Persistent pins, document-level AI exclusion, stable chunks, summaries, token estimation, semantic retrieval and background indexing remain P4.2 work.

This slice must not be described as completion of the context engine.