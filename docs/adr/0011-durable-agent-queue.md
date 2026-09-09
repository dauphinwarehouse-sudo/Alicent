# ADR 0011: Durable agent queue and at-most-once tool dispatch

- Status: accepted for the first P3.2 vertical slice
- Date: 2026-09-09

## Context

P3.2 needs agent work to survive application restarts and bound cumulative work across retries. The guarded tool executor validates, authorizes, approves and budgets one in-memory execution session, but it cannot decide whether a tool call seen after a crash already crossed the side-effect boundary.

Exactly-once effects cannot be promised for arbitrary filesystems or external services without cooperation from those systems. Retrying an ambiguous call is unsafe.

## Decision

Add an additive agent queue schema to the existing project SQLite database. It uses the repository's WAL, `synchronous=FULL`, foreign-key checks and `BEGIN IMMEDIATE` write discipline, so queued work is covered by the existing backup path without changing the manuscript schema version.

The store owns:

- custom-agent definitions and named execution profiles;
- immutable profile-budget snapshots on each task;
- idempotent enqueue and lifecycle commands keyed by caller-supplied command IDs and payload hashes;
- idempotent claims with bounded leases;
- persisted cumulative step, tool-call, input, output and cost usage;
- append-only task audit events containing stable reason codes, not prompts or executor messages;
- a tool-call ledger keyed by task and call ID.

A worker must durably record a tool call as `dispatched` before invoking the guarded executor's side-effecting implementation. Only that first transition returns permission to invoke it. Reopening the project never makes `dispatched` retryable: an interrupted task is paused with `uncertain_tool_effect`, and resume is rejected until an operator explicitly records whether the effect happened. This is at-most-once dispatch and deliberately prefers a recoverable pause over duplicate side effects.

Expired leases recover to `paused`; explicit resume returns an ordinary paused task to `queued`. Terminal tasks never restart silently. Cancellation is durable. Budget reservations, lifecycle transitions and audit records share transactions.

This PR deliberately stops at the domain and project-repository vertical slice. It does not add desktop UI, provider calls or a model worker.

## Consequences

- Two processes can claim different tasks without lost writes; one task cannot have two live claims.
- A crash before `dispatched` is retryable. A crash after it is not automatically retryable.
- Existing guarded-executor budgets remain defense in depth; persisted profile budgets are authoritative across restarts.
- Agent rows are included in normal SQLite backups.
- Desktop controls, profile editing, agent deletion, provider scheduling and operator-facing ambiguous-effect resolution remain later work. Tasks retain their original limits when profile management is added.
