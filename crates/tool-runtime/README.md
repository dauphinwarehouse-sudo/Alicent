# Alicent tool runtime contracts

`alicent-tool-runtime` is an isolated security boundary for future tools. It
does **not** execute tools or integrate with provider transports.

## Guarantees

- Tool names are unique and versioned.
- Inputs and outputs use a typed, strict JSON Schema subset. Object schemas
  always emit `"additionalProperties": false`.
- File paths, hosts, and named capabilities must match explicit permission
  scopes. Unsafe, relative, traversing, wildcard, or malformed values are
  denied.
- Only prompts marked as exclusively trusted-user and free of upstream
  injection signals can authorize a tool call.
- Destructive policies always require approval. Reversible policies explicitly
  describe rollback and may require approval.
- An approval is short-lived, one-time, and bound to SHA-256 of canonical JSON
  containing the tool name, version, and exact arguments.
- Authorization, approval, and denial decisions are available as structured
  audit events.

## Intended integration

1. Register immutable `ToolDefinition` values during application startup.
2. Build `PermissionContext` from trusted application state, never from model
   output.
3. Call `ApprovalEngine::evaluate`.
4. Display an `ApprovalChallenge` to the user when approval is required.
5. Pass the issued `ApprovalProof` only with the unchanged tool call.
6. Validate any eventual executor result with `ToolRegistry::validate_output`.

The caller must persist or forward `ApprovalEngine::journal()` if durable audit
retention is required. Execution, UI, repository access, and provider transport
remain deliberately out of scope.