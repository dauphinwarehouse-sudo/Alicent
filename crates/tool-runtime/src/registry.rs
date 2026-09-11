use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    permissions::{PermissionRequirement, PromptTrustRequirement},
    schema::{JsonSchema, SchemaDefinitionError, SchemaViolation},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolPolicy {
    /// A tool that only reads. Because it cannot change anything, it is the
    /// one policy allowed to run on a prompt that mixes the user's words with
    /// document text, which is what an agent flow always produces.
    ReadOnly {
        /// What the tool reads, for the audit trail and the approval UI.
        reads: String,
        /// Opt-in, so a read-only tool that is still sensitive keeps the
        /// strict rule. Absent in older definitions, hence the serde default.
        #[serde(default)]
        accepts_mixed_prompt: bool,
    },
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
            Self::ReadOnly { .. } => false,
            Self::Reversible {
                requires_approval, ..
            } => *requires_approval,
            Self::Destructive { .. } => true,
        }
    }

    /// Whether a prompt that mixes the user's instruction with document text
    /// may drive this tool. Anything that can write keeps demanding an
    /// exclusively user-authored prompt, and a read-only tool has to opt in.
    pub fn prompt_trust(&self) -> PromptTrustRequirement {
        let accepts_mixed = match self {
            Self::ReadOnly {
                accepts_mixed_prompt,
                ..
            } => *accepts_mixed_prompt,
            Self::Reversible { .. } | Self::Destructive { .. } => false,
        };
        if accepts_mixed {
            PromptTrustRequirement::MixedAllowed
        } else {
            PromptTrustRequirement::TrustedUserOnly
        }
    }

    fn validate(&self) -> Result<(), RegistryError> {
        let explanation = match self {
            Self::ReadOnly { reads, .. } => reads,
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

        // A read-only policy is the only one that can run on a prompt carrying
        // document text, so it must not be able to reach anything that leaves
        // the machine or changes it. Otherwise a writing tool could be
        // relabelled as read-only to get past the provenance gate.
        if matches!(self.policy, ToolPolicy::ReadOnly { .. })
            && self.permissions.iter().any(is_write_permission)
        {
            return Err(RegistryError::InvalidReadOnlyPolicy);
        }
        Ok(())
    }
}

fn is_write_permission(permission: &PermissionRequirement) -> bool {
    matches!(
        permission,
        PermissionRequirement::FileWrite { .. } | PermissionRequirement::NetworkHost { .. }
    )
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
    #[error("a read-only tool cannot request write or network permissions")]
    InvalidReadOnlyPolicy,
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
