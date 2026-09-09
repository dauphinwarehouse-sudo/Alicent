use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    hash::{canonical_json, sha256_hex},
    permissions::{authorize, PermissionContext, PermissionDenial, ResolvedPermission},
    registry::{ToolPolicy, ToolRegistry},
    schema::SchemaViolation,
};

pub const MAX_APPROVAL_TTL_SECS: u64 = 24 * 60 * 60;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub call_id: String,
    pub tool: String,
    pub arguments: Value,
    pub approval: Option<ApprovalProof>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalProof {
    pub approval_id: String,
    pub payload_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Decision {
    Authorized {
        payload_hash: String,
        permissions: Vec<ResolvedPermission>,
        policy: ToolPolicy,
    },
    ApprovalRequired {
        challenge: ApprovalChallenge,
    },
    Denied {
        reason: DenialReason,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApprovalChallenge {
    call_id: String,
    tool: String,
    tool_version: u32,
    payload_hash: String,
    permissions: Vec<ResolvedPermission>,
    policy: ToolPolicy,
}

impl ApprovalChallenge {
    pub fn call_id(&self) -> &str {
        &self.call_id
    }

    pub fn tool(&self) -> &str {
        &self.tool
    }

    pub fn payload_hash(&self) -> &str {
        &self.payload_hash
    }

    pub fn permissions(&self) -> &[ResolvedPermission] {
        &self.permissions
    }

    pub fn policy(&self) -> &ToolPolicy {
        &self.policy
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DenialReason {
    UnknownTool,
    InvalidCallId,
    InvalidInput { path: String, message: String },
    Permission(PermissionDenial),
    MissingApproval,
    InvalidApproval,
    ApprovalPayloadMismatch,
    ApprovalExpired,
    ApprovalAlreadyUsed,
}

impl From<SchemaViolation> for DenialReason {
    fn from(value: SchemaViolation) -> Self {
        Self::InvalidInput {
            path: value.path,
            message: value.message,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditOutcome {
    ApprovalRequired,
    ApprovalGranted,
    Authorized,
    Denied,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditEvent {
    pub sequence: u64,
    pub at_epoch_secs: u64,
    pub call_id: String,
    pub tool: String,
    pub payload_hash: Option<String>,
    pub policy: Option<ToolPolicy>,
    pub outcome: AuditOutcome,
    pub reason: Option<DenialReason>,
    pub actor: Option<String>,
}

#[derive(Debug)]
struct ApprovalRecord {
    tool: String,
    tool_version: u32,
    payload_hash: String,
    approved_by: String,
    expires_at: u64,
    consumed: bool,
}

#[derive(Debug, Default)]
pub struct ApprovalEngine {
    approvals: HashMap<String, ApprovalRecord>,
    journal: Vec<AuditEvent>,
    next_approval_id: u64,
    next_sequence: u64,
}

impl ApprovalEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn evaluate(
        &mut self,
        registry: &ToolRegistry,
        call: &ToolCall,
        context: &PermissionContext,
        now_epoch_secs: u64,
    ) -> Decision {
        let Some(tool) = registry.get(&call.tool) else {
            return self.deny(call, None, None, DenialReason::UnknownTool, now_epoch_secs);
        };
        if call.call_id.trim().is_empty()
            || call.call_id.len() > 128
            || call.call_id.chars().any(char::is_control)
        {
            return self.deny(
                call,
                None,
                Some(tool.policy.clone()),
                DenialReason::InvalidCallId,
                now_epoch_secs,
            );
        }
        if let Err(error) = tool.input_schema.validate(&call.arguments) {
            return self.deny(
                call,
                None,
                Some(tool.policy.clone()),
                error.into(),
                now_epoch_secs,
            );
        }
        let permissions = match authorize(&call.arguments, &tool.permissions, context) {
            Ok(permissions) => permissions,
            Err(error) => {
                return self.deny(
                    call,
                    None,
                    Some(tool.policy.clone()),
                    DenialReason::Permission(error),
                    now_epoch_secs,
                )
            }
        };
        let payload_hash = payload_hash(&call.tool, tool.version, &call.arguments);

        if !tool.policy.requires_approval() {
            self.record(AuditEvent {
                sequence: 0,
                at_epoch_secs: now_epoch_secs,
                call_id: call.call_id.clone(),
                tool: call.tool.clone(),
                payload_hash: Some(payload_hash.clone()),
                policy: Some(tool.policy.clone()),
                outcome: AuditOutcome::Authorized,
                reason: None,
                actor: None,
            });
            return Decision::Authorized {
                payload_hash,
                permissions,
                policy: tool.policy.clone(),
            };
        }

        let Some(proof) = &call.approval else {
            let challenge = ApprovalChallenge {
                call_id: call.call_id.clone(),
                tool: call.tool.clone(),
                tool_version: tool.version,
                payload_hash: payload_hash.clone(),
                permissions,
                policy: tool.policy.clone(),
            };
            self.record(AuditEvent {
                sequence: 0,
                at_epoch_secs: now_epoch_secs,
                call_id: call.call_id.clone(),
                tool: call.tool.clone(),
                payload_hash: Some(payload_hash),
                policy: Some(tool.policy.clone()),
                outcome: AuditOutcome::ApprovalRequired,
                reason: None,
                actor: None,
            });
            return Decision::ApprovalRequired { challenge };
        };

        if !constant_time_eq(proof.payload_hash.as_bytes(), payload_hash.as_bytes()) {
            return self.deny(
                call,
                Some(payload_hash),
                Some(tool.policy.clone()),
                DenialReason::ApprovalPayloadMismatch,
                now_epoch_secs,
            );
        }

        let validation = self.approvals.get(&proof.approval_id).map(|record| {
            if record.consumed {
                Err(DenialReason::ApprovalAlreadyUsed)
            } else if record.expires_at < now_epoch_secs {
                Err(DenialReason::ApprovalExpired)
            } else if record.tool != call.tool
                || record.tool_version != tool.version
                || !constant_time_eq(record.payload_hash.as_bytes(), payload_hash.as_bytes())
            {
                Err(DenialReason::ApprovalPayloadMismatch)
            } else {
                Ok(record.approved_by.clone())
            }
        });
        let approved_by = match validation {
            Some(Ok(actor)) => actor,
            Some(Err(reason)) => {
                return self.deny(
                    call,
                    Some(payload_hash),
                    Some(tool.policy.clone()),
                    reason,
                    now_epoch_secs,
                )
            }
            None => {
                return self.deny(
                    call,
                    Some(payload_hash),
                    Some(tool.policy.clone()),
                    DenialReason::InvalidApproval,
                    now_epoch_secs,
                )
            }
        };

        self.approvals
            .get_mut(&proof.approval_id)
            .expect("approval was validated")
            .consumed = true;
        self.record(AuditEvent {
            sequence: 0,
            at_epoch_secs: now_epoch_secs,
            call_id: call.call_id.clone(),
            tool: call.tool.clone(),
            payload_hash: Some(payload_hash.clone()),
            policy: Some(tool.policy.clone()),
            outcome: AuditOutcome::Authorized,
            reason: None,
            actor: Some(approved_by),
        });
        Decision::Authorized {
            payload_hash,
            permissions,
            policy: tool.policy.clone(),
        }
    }

    pub fn approve(
        &mut self,
        challenge: &ApprovalChallenge,
        approved_by: &str,
        now_epoch_secs: u64,
        ttl_secs: u64,
    ) -> Result<ApprovalProof, ApprovalIssueError> {
        if approved_by.trim().is_empty() || approved_by.chars().any(char::is_control) {
            return Err(ApprovalIssueError::InvalidActor);
        }
        if ttl_secs == 0 || ttl_secs > MAX_APPROVAL_TTL_SECS {
            return Err(ApprovalIssueError::InvalidTtl);
        }
        let expires_at = now_epoch_secs
            .checked_add(ttl_secs)
            .ok_or(ApprovalIssueError::InvalidTtl)?;
        self.next_approval_id += 1;
        let id = format!(
            "approval-{}-{}",
            self.next_approval_id,
            &challenge.payload_hash[..12]
        );
        self.approvals.insert(
            id.clone(),
            ApprovalRecord {
                tool: challenge.tool.clone(),
                tool_version: challenge.tool_version,
                payload_hash: challenge.payload_hash.clone(),
                approved_by: approved_by.into(),
                expires_at,
                consumed: false,
            },
        );
        self.record(AuditEvent {
            sequence: 0,
            at_epoch_secs: now_epoch_secs,
            call_id: challenge.call_id.clone(),
            tool: challenge.tool.clone(),
            payload_hash: Some(challenge.payload_hash.clone()),
            policy: Some(challenge.policy.clone()),
            outcome: AuditOutcome::ApprovalGranted,
            reason: None,
            actor: Some(approved_by.into()),
        });
        Ok(ApprovalProof {
            approval_id: id,
            payload_hash: challenge.payload_hash.clone(),
        })
    }

    pub fn journal(&self) -> &[AuditEvent] {
        &self.journal
    }

    fn deny(
        &mut self,
        call: &ToolCall,
        payload_hash: Option<String>,
        policy: Option<ToolPolicy>,
        reason: DenialReason,
        now_epoch_secs: u64,
    ) -> Decision {
        self.record(AuditEvent {
            sequence: 0,
            at_epoch_secs: now_epoch_secs,
            call_id: call.call_id.clone(),
            tool: call.tool.clone(),
            payload_hash,
            policy,
            outcome: AuditOutcome::Denied,
            reason: Some(reason.clone()),
            actor: None,
        });
        Decision::Denied { reason }
    }

    fn record(&mut self, mut event: AuditEvent) {
        self.next_sequence += 1;
        event.sequence = self.next_sequence;
        self.journal.push(event);
    }
}

fn payload_hash(tool: &str, version: u32, arguments: &Value) -> String {
    sha256_hex(&canonical_json(&json!({
        "tool": tool,
        "version": version,
        "arguments": arguments,
    })))
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApprovalIssueError {
    #[error("approving actor is invalid")]
    InvalidActor,
    #[error("approval TTL must be between 1 second and 24 hours")]
    InvalidTtl,
}
