/** Core editor, recovery and agent-runtime contracts. */
export type {
  ProviderGenerationEvent,
  ProviderGenerationPort,
  ProviderGenerationRequest,
  ProviderGenerationResult,
} from "./provider-generation";
export type UUID = string;
export interface Project {
  id: UUID;
  title: string;
  schema_version: number;
  created_at: string;
}
export type DocumentKind = "folder" | "scene" | "note";
export interface DocumentSummary {
  id: UUID;
  parent_id: UUID | null;
  title: string;
  kind: DocumentKind;
  revision: number;
  updated_at: string;
}
export interface Document extends DocumentSummary {
  content: string;
}
export interface ArchiveDocument {
  command_id: UUID;
  document_id: UUID;
  expected_revision: number;
}
export interface ArchiveReceipt {
  command_id: UUID;
  document_id: UUID;
  affected_count: number;
}
export interface ArchivedDocument extends DocumentSummary {
  archived_at: string;
  affected_count: number;
}
export interface SaveDocument {
  command_id: UUID;
  document_id: UUID;
  expected_revision: number;
  content: string;
}
export interface MoveDocument {
  command_id: UUID;
  document_id: UUID;
  parent_id: UUID | null;
  expected_revision: number;
}
export interface VersionSummary {
  revision: number;
  created_at: string;
  actor: string;
}
export interface BackupInfo {
  path: string;
  bytes: number;
  schema_version: number;
}
export interface Checkpoint {
  id: UUID;
  name: string;
  created_at: string;
  document_count: number;
}
export interface CheckpointDocument {
  id: UUID;
  title: string;
  current_revision: number;
  target_revision: number;
  changed: boolean;
}
export interface CheckpointPreview {
  checkpoint: Checkpoint;
  project_revision: number;
  changed_count: number;
  newer_document_count: number;
  documents: CheckpointDocument[];
  has_more: boolean;
}
export interface CheckpointRestore {
  checkpoint_id: UUID;
  changed_count: number;
  operation_id: UUID;
  undo_checkpoint_id: UUID | null;
}
export interface ProjectPort {
  backupProject(): Promise<BackupInfo | null>;
  restoreBackup(): Promise<Project | null>;
  cancelRecovery(): Promise<void>;
  createCheckpoint(id: UUID, name: string): Promise<Checkpoint>;
  checkpoints(offset?: number): Promise<Checkpoint[]>;
  checkpointPreview(id: UUID, offset?: number): Promise<CheckpointPreview>;
  restoreCheckpoint(
    id: UUID,
    expectedRevision: number,
    commandId: UUID,
  ): Promise<CheckpointRestore>;
  createProject(title: string): Promise<Project | null>;
  openProject(): Promise<Project | null>;
  list(parent: UUID | null, offset?: number): Promise<DocumentSummary[]>;
  createDocument(
    title: string,
    kind: DocumentKind,
    parent: UUID | null,
    commandId?: UUID,
  ): Promise<Document>;
  renameDocument(
    id: UUID,
    title: string,
    expectedRevision: number,
    commandId: UUID,
  ): Promise<Document>;
  duplicateDocument(
    id: UUID,
    title: string,
    parent: UUID | null,
    commandId: UUID,
  ): Promise<Document>;
  archived(offset?: number): Promise<ArchivedDocument[]>;
  archiveDocument(command: ArchiveDocument): Promise<ArchiveReceipt>;
  restoreArchived(command: ArchiveDocument): Promise<ArchiveReceipt>;
  moveDocument(command: MoveDocument): Promise<Document>;
  read(id: UUID): Promise<Document>;
  save(command: SaveDocument): Promise<Document>;
  search(query: string): Promise<DocumentSummary[]>;
  versions(id: UUID, offset?: number): Promise<VersionSummary[]>;
  versionContent(id: UUID, revision: number): Promise<string>;
  restore(
    id: UUID,
    revision: number,
    expectedRevision: number,
    commandId: UUID,
  ): Promise<Document>;
}

export type ProviderProtocol =
  | "openai-chat"
  | "openai-responses"
  | "anthropic-messages";
export interface ProviderConfig {
  id: UUID;
  protocol: ProviderProtocol;
  baseUrl: string;
  model: string;
  secretRef: string; // Opaque OS-vault reference, NEVER a key.
  contextWindow: number;
  maxOutputTokens: number;
  timeoutMs: number;
}
export interface Capabilities {
  streaming: boolean;
  tools: boolean;
  vision: boolean;
  reasoning: boolean;
}
export type ModelMessage =
  | { role: "system" | "user"; text: string }
  | { role: "assistant"; text: string; toolCalls?: ToolCall[] }
  | {
      role: "tool";
      callId: string;
      name: string;
      output: unknown;
      isError: boolean;
    };
export interface ToolCall {
  id: string;
  name: string;
  arguments: unknown;
}
export type ModelEvent =
  | { type: "text_delta"; text: string }
  | { type: "reasoning_summary"; text: string } // Only content explicitly supplied by provider.
  | { type: "tool_call"; call: ToolCall } // Emit only after complete argument parsing/validation.
  | { type: "usage"; inputTokens: number; outputTokens: number }
  | { type: "done"; reason: "stop" | "tool_calls" | "length" }
  | { type: "error"; code: string; retryable: boolean };
export interface ProviderAdapter {
  capabilities(
    config: ProviderConfig,
    signal: AbortSignal,
  ): Promise<Capabilities>;
  stream(
    config: ProviderConfig,
    messages: ModelMessage[],
    tools: ToolDefinition[],
    signal: AbortSignal,
  ): AsyncIterable<ModelEvent>;
}
export type RiskLevel =
  | "read"
  | "write_reversible"
  | "destructive"
  | "external";
export interface ToolDefinition {
  name: string;
  description: string;
  input_schema: Readonly<Record<string, unknown>>;
  output_schema: Readonly<Record<string, unknown>>;
  risk_level: RiskLevel;
  requires_confirmation: boolean;
  timeout_ms: number;
  permission_scope: "document" | "project" | "network";
}
export interface ToolContext {
  projectId: UUID;
  taskId: UUID;
  commandId: UUID;
  actorId: UUID;
  allowedDocumentIds: ReadonlySet<UUID>;
  signal: AbortSignal;
}
export interface Tool {
  definition: ToolDefinition;
  execute(input: unknown, context: ToolContext): Promise<unknown>;
}
export type TaskStatus =
  | "queued"
  | "planning"
  | "waiting_for_approval"
  | "running"
  | "paused"
  | "completed"
  | "failed"
  | "cancelled";
export interface AgentTask {
  id: UUID;
  projectId: UUID;
  agentId: UUID;
  status: TaskStatus;
  budget: { maxSteps: number; deadline: string; maxCostUsd: number };
}
export interface AgentRuntime {
  enqueue(request: {
    projectId: UUID;
    agentId: UUID;
    text: string;
    budget: AgentTask["budget"];
  }): Promise<AgentTask>;
  cancel(taskId: UUID): Promise<void>;
  resume(taskId: UUID): Promise<void>;
}
/** Design-time fail-closed policy. Runtime must bind approval to exact command + payload hash. */
export function needsApproval(
  tool: ToolDefinition,
  autoReversibleWrites: boolean,
): boolean {
  return (
    tool.requires_confirmation ||
    tool.risk_level === "destructive" ||
    tool.risk_level === "external" ||
    (tool.risk_level === "write_reversible" && !autoReversibleWrites)
  );
}
