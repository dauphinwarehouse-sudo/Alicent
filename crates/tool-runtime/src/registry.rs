use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    permissions::PermissionRequirement,
    schema::{JsonSchema, SchemaDefinitionError, SchemaViolation},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolPolicy {
    Reversible {
        requires_approval: bool,
        rollback: String,
    },
    Destructive {
        impact: String,
    },
}

impl ToolPolicy {
    pub fn requires_approval(&self) -> bool {
        match self {
            Self::Reversible {
                requires_approval, ..
            } => *requires_approval,
            Self::Destructive { .. } => true,
        }
    }

    fn validate(&self) -> Result<(), RegistryError> {
        let explanation = match self {
            Self::Reversible { rollback, .. } => rollback,
            Self::Destructive { impact } => impact,
        };
        if explanation.trim().is_empty() || explanation.chars().any(char::is_control) {
            Err(RegistryError::InvalidPolicy)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub version: u32,
    pub description: String,
    pub input_schema: JsonSchema,
    pub output_schema: JsonSchema,
    #[serde(default)]
    pub permissions: Vec<PermissionRequirement>,
    pub policy: ToolPolicy,
}

impl ToolDefinition {
    fn validate(&self) -> Result<(), RegistryError> {
        if self.version == 0 || !valid_tool_name(&self.name) {
            return Err(RegistryError::InvalidToolName);
        }
        if self.description.trim().is_empty() || self.description.chars().any(char::is_control) {
            return Err(RegistryError::InvalidDescription);
        }
        self.input_schema.validate_definition()?;
        self.output_schema.validate_definition()?;
        self.policy.validate()?;

        let mut seen = BTreeSet::new();
        for permission in &self.permissions {
            permission
                .validate_definition()
                .map_err(RegistryError::InvalidPermission)?;
            let encoded =
                serde_json::to_string(permission).expect("permission requirements serialize");
            if !seen.insert(encoded) {
                return Err(RegistryError::DuplicatePermission);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, ToolDefinition>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, tool: ToolDefinition) -> Result<(), RegistryError> {
        tool.validate()?;
        if self.tools.contains_key(&tool.name) {
            return Err(RegistryError::DuplicateTool(tool.name));
        }
        self.tools.insert(tool.name.clone(), tool);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&ToolDefinition> {
        self.tools.get(name)
    }

    pub fn definitions(&self) -> impl Iterator<Item = &ToolDefinition> {
        self.tools.values()
    }

    pub fn validate_output(&self, name: &str, output: &Value) -> Result<(), OutputValidationError> {
        let tool = self
            .get(name)
            .ok_or_else(|| OutputValidationError::UnknownTool(name.into()))?;
        tool.output_schema
            .validate(output)
            .map_err(OutputValidationError::Schema)
    }
}

fn valid_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 80
        && name.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || byte == b'.'
                || byte == b'_'
                || byte == b'-'
        })
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("tool name or version is invalid")]
    InvalidToolName,
    #[error("tool description is invalid")]
    InvalidDescription,
    #[error("tool policy is invalid")]
    InvalidPolicy,
    #[error("permission requirement is invalid: {0}")]
    InvalidPermission(String),
    #[error("permission requirement is duplicated")]
    DuplicatePermission,
    #[error("tool `{0}` is already registered")]
    DuplicateTool(String),
    #[error(transparent)]
    InvalidSchema(#[from] SchemaDefinitionError),
}

#[derive(Debug, thiserror::Error)]
pub enum OutputValidationError {
    #[error("unknown tool `{0}`")]
    UnknownTool(String),
    #[error(transparent)]
    Schema(#[from] SchemaViolation),
}
