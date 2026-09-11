//! Pure domain and command contracts. No database, desktop or HTTP dependencies.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const SCHEMA_VERSION: i64 = 5;
pub const MAX_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("Название должно содержать от 1 до 200 символов")]
    InvalidTitle,
    #[error("Документ превышает ограничение прототипа: 8 МиБ")]
    DocumentTooLarge,
    #[error("Недопустимый переход состояния задачи")]
    InvalidTransition,
    #[error("Документ не может быть одновременно исключён и закреплён для ИИ")]
    InvalidAiContext,
}

pub fn validate_title(title: &str) -> Result<(), DomainError> {
    if title.trim().is_empty() || title.chars().count() > 200 || title.chars().any(char::is_control)
    {
        Err(DomainError::InvalidTitle)
    } else {
        Ok(())
    }
}

pub fn validate_content(content: &str) -> Result<(), DomainError> {
    if content.len() > MAX_DOCUMENT_BYTES {
        Err(DomainError::DocumentTooLarge)
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DocumentKind {
    Folder,
    Scene,
    Note,
}
impl DocumentKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Folder => "folder",
            Self::Scene => "scene",
            Self::Note => "note",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: Uuid,
    pub title: String,
    pub schema_version: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentSummary {
    pub id: Uuid,
    pub parent_id: Option<Uuid>,
    pub title: String,
    pub kind: DocumentKind,
    pub revision: i64,
    pub updated_at: String,
    pub ai_context_excluded: bool,
    pub ai_context_pinned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    #[serde(flatten)]
    pub summary: DocumentSummary,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveDocument {
    pub command_id: Uuid,
    pub document_id: Uuid,
    pub expected_revision: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveReceipt {
    pub command_id: Uuid,
    pub document_id: Uuid,
    pub affected_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchivedDocument {
    #[serde(flatten)]
    pub summary: DocumentSummary,
    pub archived_at: String,
    pub affected_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveDocument {
    pub command_id: Uuid,
    pub document_id: Uuid,
    pub expected_revision: i64,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveDocument {
    pub command_id: Uuid,
    pub document_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub expected_revision: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetDocumentAiContext {
    pub command_id: Uuid,
    pub document_id: Uuid,
    pub expected_revision: i64,
    pub excluded: bool,
    pub pinned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionSummary {
    pub revision: i64,
    pub created_at: String,
    pub actor: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Queued,
    Planning,
    WaitingForApproval,
    Running,
    Paused,
    Completed,
    Failed,
    Cancelled,
}
impl TaskStatus {
    pub fn transition(self, next: Self) -> Result<Self, DomainError> {
        use TaskStatus::*;
        // WaitingForApproval reaches Failed directly: a task can break while
        // the approval prompt is open, and routing it through Running to
        // record that would claim an execution that never happened.
        let valid = matches!(
            (self, next),
            (Queued, Planning | Cancelled)
                | (Planning, Running | WaitingForApproval | Failed | Cancelled)
                | (WaitingForApproval, Running | Failed | Cancelled)
                | (
                    Running,
                    WaitingForApproval | Paused | Completed | Failed | Cancelled
                )
                | (Paused, Queued | Running | Cancelled)
        );
        if valid {
            Ok(next)
        } else {
            Err(DomainError::InvalidTransition)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn titles_accept_cyrillic_not_control_characters() {
        assert!(validate_title("Последняя глава").is_ok());
        assert!(validate_title("   ").is_err());
        assert!(validate_title("Глава\n1").is_err());
        assert!(validate_title(&"я".repeat(201)).is_err());
    }
    #[test]
    fn terminal_tasks_cannot_restart_silently() {
        for state in [
            TaskStatus::Completed,
            TaskStatus::Failed,
            TaskStatus::Cancelled,
        ] {
            assert!(state.transition(TaskStatus::Running).is_err());
        }
        assert_eq!(
            TaskStatus::Running.transition(TaskStatus::Paused).unwrap(),
            TaskStatus::Paused
        );
    }
    #[test]
    fn tasks_waiting_for_approval_can_fail() {
        let status = TaskStatus::WaitingForApproval;
        assert_eq!(
            status.transition(TaskStatus::Failed).unwrap(),
            TaskStatus::Failed
        );
        // Still no shortcut into a success or a pause it never entered.
        assert!(status.transition(TaskStatus::Completed).is_err());
        assert!(status.transition(TaskStatus::Paused).is_err());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupInfo {
    pub path: String,
    pub bytes: u64,
    pub schema_version: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub id: Uuid,
    pub name: String,
    pub created_at: String,
    pub document_count: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointDocument {
    pub id: Uuid,
    pub title: String,
    pub current_revision: i64,
    pub target_revision: i64,
    pub changed: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointPreview {
    pub checkpoint: Checkpoint,
    pub project_revision: i64,
    pub changed_count: i64,
    pub newer_document_count: i64,
    pub documents: Vec<CheckpointDocument>,
    pub has_more: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointRestore {
    pub undo_checkpoint_id: Option<Uuid>,
    pub checkpoint_id: Uuid,
    pub changed_count: usize,
    pub operation_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionProfile {
    pub id: String,
    pub name: String,
    pub max_steps: i64,
    pub max_tool_calls: i64,
    pub max_input_bytes: i64,
    pub max_output_bytes: i64,
    pub max_cost_microusd: i64,
    pub max_runtime_secs: i64,
    pub lease_secs: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomAgent {
    pub id: Uuid,
    pub name: String,
    pub instructions: String,
    pub profile_id: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskBudget {
    pub max_steps: i64,
    pub max_tool_calls: i64,
    pub max_input_bytes: i64,
    pub max_output_bytes: i64,
    pub max_cost_microusd: i64,
    pub deadline_at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskUsage {
    pub steps: i64,
    pub tool_calls: i64,
    pub input_bytes: i64,
    pub output_bytes: i64,
    pub cost_microusd: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTask {
    pub id: Uuid,
    pub project_id: Uuid,
    pub agent_id: Uuid,
    pub profile_id: String,
    pub status: TaskStatus,
    pub prompt: String,
    pub budget: TaskBudget,
    pub usage: TaskUsage,
    pub failure_code: Option<String>,
    pub attempt: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTaskClaim {
    pub task: AgentTask,
    pub lease_token: String,
    pub lease_expires_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateAgentCommand {
    pub command_id: Uuid,
    pub agent_id: Uuid,
    pub name: String,
    pub instructions: String,
    pub profile_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnqueueAgentTask {
    pub command_id: Uuid,
    pub task_id: Uuid,
    pub agent_id: Uuid,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTaskCommand {
    pub command_id: Uuid,
    pub task_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDispatchDecision {
    pub execute: bool,
    pub reason: Option<String>,
}
