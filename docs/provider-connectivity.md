# Provider connectivity and compatibility

`alicent-provider-wire` supports privacy-preserving streaming connections to the official OpenAI
API and custom OpenAI-compatible endpoints.

## Supported contracts

| Contract | Endpoint appended to the configured base URL | Streaming support |
| --- | --- | --- |
| OpenAI Chat Completions | `chat/completions` | SSE text deltas, client function calls, finish reason and usage |
| OpenAI Responses | `responses` | SSE text deltas, client function calls, completion status and usage |

The request builder always sends `stream: true` and `store: false`. The transport accepts only
`text/event-stream`, disables redirects, bounds each chunk and the total response, and never treats
EOF as protocol completion. Chat requires a valid `finish_reason` and `[DONE]`; Responses requires
`response.completed`.

## Compatibility boundary

“OpenAI-compatible” means the endpoint preserves the selected request and SSE event contract.
Custom model identifiers and base paths are supported. Provider-specific authentication schemes,
query-string keys, legacy non-streaming JSON, WebSockets, multiple choices, audio, vision,
reasoning/signature blocks, server-hosted tools, resumable streams and automatic redirect following
are intentionally unsupported. Unknown output or event types fail closed instead of being silently
discarded.

Only HTTPS endpoints are accepted in production. Plain HTTP is available solely for an explicitly
enabled loopback address (`localhost`, `127.0.0.0/8` or `::1`) for local development and synthetic
tests. Embedded URL credentials, query strings and fragments are rejected.

## Timeouts, cancellation and retries

Connect, per-chunk idle and whole-request deadlines are independent and bounded. Cancellation
interrupts connection attempts, retry waits and active streams. A response is never retried after
stream bytes are exposed. At most three attempts are allowed, only for connect failures or explicit
`429`, `502`, `503` and `504` statuses before streaming starts. Numeric `Retry-After` is honored up
to the configured cap. Ambiguous timeouts and other POST failures are not retried to avoid duplicate
billable generations.

## Credentials and privacy

Persisted provider configuration contains endpoint, model and policy metadata only. API keys are
stored through `WindowsCredentialManager`, backed by Windows Credential Manager. There is no file,
database or log fallback. In-memory secrets zeroize on drop and have redacted debug formatting.
Public transport and vault errors expose allowlisted categories only; response bodies, headers,
manuscript content and credentials are never included.

Network transmission, custom endpoints and loopback HTTP each have separate privacy gates. Product
surfaces must disclose the selected provider's retention/training policy and obtain the user's
network consent before setting `allow_model_requests`.

## Tests

Contract and security tests use local loopback servers and synthetic credentials only. They cover
both endpoint contracts, strict Responses normalization, retry boundaries, redirect blocking,
timeouts, aborts and redaction. No generation request is made to a live provider in CI. Any future
live smoke test must be ignored by default, require an explicit environment opt-in, and use only a
non-billable metadata endpoint such as `GET /models`.