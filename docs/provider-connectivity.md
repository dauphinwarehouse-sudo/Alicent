# Provider connectivity and compatibility

`alicent-provider-wire` provides privacy-preserving, bounded connectivity for the official OpenAI
and Anthropic APIs and explicitly enabled HTTPS-compatible endpoints.

## Supported contracts

| Contract | Endpoint appended to the configured base URL | Streaming support |
| --- | --- | --- |
| OpenAI Chat Completions | `chat/completions` | SSE text deltas, client function calls, finish reason and usage |
| OpenAI Responses | `responses` | SSE text deltas, client function calls, completion status and usage |
| Anthropic Messages | `messages` | SSE text deltas, client tool use, stop reason and usage |

OpenAI requests require `stream: true` and `store: false`. Anthropic requests require `stream: true`,
omit the unsupported `store` field, and use `x-api-key` plus a fixed `anthropic-version` header.
Provider-specific credentials are marked sensitive and are never included in public errors or logs.

The transport accepts only `text/event-stream`, disables redirects, bounds each chunk and the total
response, and never treats EOF as protocol completion. The non-generation connection check uses a
bounded `GET /models` metadata request; it never sends a generation payload or returns the response
body to the renderer.

## Compatibility boundary

“OpenAI-compatible” or “Anthropic-compatible” means the endpoint preserves the selected request,
authentication and SSE contract. Custom model identifiers and base paths are supported in balanced
privacy mode. Query-string keys, legacy non-streaming JSON, WebSockets, multiple choices, audio,
vision, server-hosted tools, resumable streams and automatic redirects are unsupported.

Production accepts HTTPS only. Plain HTTP loopback remains available solely to Rust fixture tests
that construct an explicit transport policy; the renderer capability handshake reports it as disabled
in production. Embedded URL credentials, query strings and fragments are rejected.

## Timeouts, cancellation and retries

Connect, per-chunk idle and whole-request deadlines are independent and bounded. Cancellation
interrupts connection attempts, retry waits and active streams. A response is never retried after
stream bytes are exposed. At most three attempts are allowed, only for connect failures or explicit
`429`, `502`, `503` and `504` statuses before streaming starts. Numeric `Retry-After` is capped.
Ambiguous POST failures are not retried to avoid duplicate billable generations.

## Credentials, settings and public errors

API keys are stored only through `WindowsCredentialManager`, backed by Windows Credential Manager.
There is no file, database or log fallback. In-memory secrets zeroize on drop and have redacted debug
formatting. The settings file contains only provider, endpoint, model and privacy metadata.

The renderer first performs a versioned native capability handshake. Native commands return only
fixed, allowlisted error codes. Stored credentials are represented in renderer snapshots by a boolean
and are never returned. Strict privacy accepts only the official endpoint; balanced privacy permits
custom HTTPS endpoints. The connection check itself is the explicit user action authorizing its one
non-generation network request; saving settings does not use the network.

## Tests

Contract and security tests use local loopback servers, mock vaults and synthetic credentials only.
They cover both providers' authentication, endpoint and body invariants, bounded metadata probes,
redirect blocking, timeout/abort behavior, persistence and redaction. CI performs no live or billable
provider call and stores no real key.
