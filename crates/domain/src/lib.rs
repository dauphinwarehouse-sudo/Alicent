//! Pure domain and command contracts. No database, desktop or HTTP dependencies.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const SCHEMA_VERSION: i64 = 1;
pub const MAX_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("Название должно содержать от 1 до 200 символов")]
    InvalidTitle,
    #[error("Документ превышает ограничение прототипа: 8 МиБ")]
    DocumentTooLarge,
    #[error("Недопустимый переход состояния задачи")]
    InvalidTransition,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    #[serde(flatten)]
    pub summary: DocumentSummary,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveDocument {
    pub command_id: Uuid,
    pub document_id: Uuid,
    pub expected_revision: i64,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionSummary {
    pub revision: i64,
    pub created_at: String,
    pub actor: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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
        let valid = matches!(
            (self, next),
            (Queued, Planning | Cancelled)
                | (Planning, Running | WaitingForApproval | Failed | Cancelled)
                | (WaitingForApproval, Running | Cancelled)
                | (
                    Running,
                    WaitingForApproval | Paused | Completed | Failed | Cancelled
                )
                | (Paused, Running | Cancelled)
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
}
