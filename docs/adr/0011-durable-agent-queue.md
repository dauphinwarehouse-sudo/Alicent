# ADR 0011: Durable agent queue and at-most-once tool dispatch

- Status: accepted for the first P3.2 vertical slice
- Date: 2026-09-09

## Context

P3.2 needs agent work to survive application restarts, bound cumulative work across retries and expose a small custom-agent workflow in the desktop UI. The guarded tool executor already validates, authorizes, approves and budgets one in-memory execution session, but it cannot decide whether a tool call seen after a crash has already crossed the side-effect boundary.

Exactly-once effects cannot be promised for arbitrary filesystems or external services without cooperation from those systems. Retrying an ambiguous call is unsafe.

## Decision

Add `alicent-agent-runtime`, backed by a project-local `agent-queue.sqlite3` database configured with WAL, foreign keys, `synchronous=FULL` and `BEGIN IMMEDIATE` writes.

The store owns:

- custom-agent definitions and named execution profiles;
- immutable profile-budget snapshots on each task;
- idempotent enqueue and lifecycle commands, keyed by caller-supplied command IDs and payload hashes;
- idempotent claims with bounded leases;
- persisted cumulative step, tool-call, input, output and cost usage;
- append-only task audit events containing stable reason codes, not prompts or executor messages;
- a tool-call ledger keyed by task and call ID.

A worker must durably move a tool call from `prepared` to `dispatched` before invoking the guarded executor's inner side-effecting implementation. Only that first transition returns permission to invoke it. Reopening the store never turns `dispatched` back into `prepared`: an interrupted task is paused with `uncertain_tool_effect`, and automatic resume is rejected until an operator explicitly records whether the effect happened. This is at-most-once dispatch. It deliberately prefers a recoverable pause over duplicate side effects.

Expired task leases are recovered to `paused`; an explicit resume returns an ordinary paused task to `queued`. Terminal tasks never restart silently. Cancellation is durable and checked again before dispatch. Budget reservations and lifecycle transitions occur in the same transactions as their audit records.

The desktop slice lists profiles and custom agents, creates an agent, enqueues a task, and exposes cancel/resume controls and persisted usage. It does not make provider calls or run a background worker; those integrations remain separate work.

## Consequences

- Two processes can claim different tasks without lost writes; one task cannot have two live claims.
- A crash before `dispatched` is retryable. A crash after it is not automatically retryable.
- Existing in-memory executor budgets remain defense in depth; persisted profile budgets are authoritative across restarts.
- The sidecar keeps P3.2 isolated from the manuscript schema and migration chain. Existing project backup/restore does not yet copy this sidecar; backup integration is follow-up work and is called out in the pull request.
- Editing profiles and deleting agents are intentionally outside this first slice. Tasks retain their original limits even when later profile management is added.
