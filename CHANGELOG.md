# Changelog

## Unreleased — Ficbook-inspired UI

- Reworked the writing workspace with a brown frame, cream paper and ochre accents.
- Added revision-guarded document rename and idempotent scene/note duplication.
- Added active-only document/folder moves plus reversible schema-v3 subtree archive and restore.
- Added the isolated ProseMirror rich-text spike and ADR 0009.
- Added bounded Markdown/TXT document I/O, strict tool-runtime approvals, provider connectivity, and the story-graph prototype (ADR 0010).
- Added durable per-document AI context pins and exclusions in project schema v5.
- Added an isolated stable-chunk and SQLite FTS context-index prototype with incremental ID reuse and fail-closed exclusion removal (ADR 0014).
- Bound each tool approval to the call it was granted for, made approval identifiers opaque, pruned aged-out approvals and removed the panic paths from approval redemption.
- Tightened path scopes for Windows: drive-qualified paths compare case-insensitively, and reserved device names or segments Windows strips are rejected.
- Added a read-only tool policy that may opt into prompts mixing user instructions with document text, so read-only tools can run in an agent flow while everything that writes still requires a trusted-user prompt.
- Allowed a task waiting for approval to fail directly, instead of having to claim an execution it never started.
- Ran the CI reporting script from the base branch and bounded every job with a timeout.
- Preserved the installed Windows WebView2 and accessibility audit in CI.
- Added working section links, warm light/dark themes and a matching Alicent icon.
- Added responsive, contrast, focus-navigation and save-error regression tests.
- Preserved local storage, autosave, search and reversible version history.


## 0.1.0 — foundation branch, unreleased

- Added Rust domain and transactional SQLite repository with FTS5, versions and revision guards.
- Added Russian React/CodeMirror UI and Tauri Windows shell.
- Added serialized autosave, history diff and reversible version restore.
- Added provider/tool/runtime interfaces (not implemented AI features).
- Added unit, component, property, crash recovery and browser smoke tests.
- Added Windows prototype CI workflow, architecture, threat model and ADRs. Workflow activation succeeded after connection permissions were updated; Windows build verification is pending.

Not a production release. Unsigned installer, native Windows validation and full MVP remain pending.
