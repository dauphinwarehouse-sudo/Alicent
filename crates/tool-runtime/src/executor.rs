use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    approval::{
        ApprovalChallenge, ApprovalEngine, ApprovalIssueError, AuditEvent, Decision, DenialReason,
        ToolCall,
    },
    hash::{canonical_json, sha256_hex},
    permissions::{PermissionContext, ResolvedPermission},
    registry::{OutputValidationError, ToolPolicy, ToolRegistry},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionBudget {
    pub max_executions: u64,
    pub max_input_bytes: u64,
    pub max_output_bytes: u64,
}

impl ExecutionBudget {
    pub const fn new(max_executions: u64, max_input_bytes: u64, max_output_bytes: u64) -> Self {
        Self {
            max_executions,
            max_input_bytes,
            max_output_bytes,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetUsage {
    pub executions: u64,
    pub input_bytes: u64,
    pub output_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetDimension {
    Executions,
    InputBytes,
    OutputBytes,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetExceeded {
    pub dimension: BudgetDimension,
    pub limit: u64,
    pub attempted: u64,
}

#[derive(Debug, Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Debug, Clone)]
pub struct ExecutionRequest {
    pub call_id: String,
    pub tool: String,
    pub arguments: Value,
    pub payload_hash: String,
    pub permissions: Vec<ResolvedPermission>,
    pub policy: ToolPolicy,
    pub cancellation: CancellationToken,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct ExecutorError {
    pub message: String,
}

impl ExecutorError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

pub trait ToolExecutor {
    fn execute(&mut self, request: ExecutionRequest) -> Result<Value, ExecutorError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExecutionRejection {
    Authorization { reason: DenialReason },
    Cancelled,
    BudgetExceeded { budget: BudgetExceeded },
    PayloadChanged,
    ExecutorFailed { message: String },
    InvalidOutput { message: String },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExecutionOutcome {
    ApprovalRequired {
        challenge: ApprovalChallenge,
    },
    Executed {
        output: Value,
        payload_hash: String,
    },
    Rejected {
        reason: ExecutionRejection,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionAuditOutcome {
    ApprovalRequired,
    Denied,
    Cancelled,
    BudgetExceeded,
    Started,
    Succeeded,
    Failed,
    InvalidOutput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionAuditEvent {
    pub sequence: u64,
    pub at_epoch_secs: u64,
    pub call_id: String,
    pub tool: String,
    pub payload_hash: Option<String>,
    pub outcome: ExecutionAuditOutcome,
    pub detail: Option<String>,
}

#[derive(Debug)]
pub struct ToolRuntime<E> {
    registry: ToolRegistry,
    approvals: ApprovalEngine,
    executor: E,
    budget: ExecutionBudget,
    usage: BudgetUsage,
    journal: Vec<ExecutionAuditEvent>,
    next_sequence: u64,
}

impl<E: ToolExecutor> ToolRuntime<E> {
    pub fn new(registry: ToolRegistry, executor: E, budget: ExecutionBudget) -> Self {
        Self {
            registry,
            approvals: ApprovalEngine::new(),
            executor,
            budget,
            usage: BudgetUsage::default(),
            journal: Vec::new(),
            next_sequence: 0,
        }
    }

    pub fn approve(
        &mut self,
        challenge: &ApprovalChallenge,
        approved_by: &str,
        now_epoch_secs: u64,
        ttl_secs: u64,
    ) -> Result<crate::ApprovalProof, ApprovalIssueError> {
        self.approvals
            .approve(challenge, approved_by, now_epoch_secs, ttl_secs)
    }

    pub fn execute(
        &mut self,
        call: &ToolCall,
        context: &PermissionContext,
        now_epoch_secs: u64,
        cancellation: &CancellationToken,
    ) -> ExecutionOutcome {
        if cancellation.is_cancelled() {
            return self.reject(
                call,
                None,
                ExecutionRejection::Cancelled,
                ExecutionAuditOutcome::Cancelled,
                now_epoch_secs,
            );
        }

        let input_bytes = canonical_json(&call.arguments).len() as u64;
        if let Some(exceeded) = checked_budget(
            self.usage.executions,
            1,
            self.budget.max_executions,
            BudgetDimension::Executions,
        )
        .or_else(|| {
            checked_budget(
                self.usage.input_bytes,
                input_bytes,
                self.budget.max_input_bytes,
                BudgetDimension::InputBytes,
            )
        }) {
            return self.reject_budget(call, None, exceeded, now_epoch_secs);
        }

        let decision = self
            .approvals
            .evaluate(&self.registry, call, context, now_epoch_secs);
        let (authorized_hash, permissions, policy) = match decision {
            Decision::ApprovalRequired { challenge } => {
                self.record(
                    call,
                    Some(challenge.payload_hash().into()),
                    ExecutionAuditOutcome::ApprovalRequired,
                    None,
                    now_epoch_secs,
                );
                return ExecutionOutcome::ApprovalRequired { challenge };
            }
            Decision::Denied { reason } => {
                return self.reject(
                    call,
                    None,
                    ExecutionRejection::Authorization { reason },
                    ExecutionAuditOutcome::Denied,
                    now_epoch_secs,
                );
            }
            Decision::Authorized {
                payload_hash,
                permissions,
                policy,
            } => (payload_hash, permissions, policy),
        };

        if cancellation.is_cancelled() {
            return self.reject(
                call,
                Some(authorized_hash),
                ExecutionRejection::Cancelled,
                ExecutionAuditOutcome::Cancelled,
                now_epoch_secs,
            );
        }

        // Re-fetch the registered version and re-hash the exact payload at the
        // last possible point before crossing the executor trust boundary.
        let Some(definition) = self.registry.get(&call.tool) else {
            return self.reject(
                call,
                Some(authorized_hash),
                ExecutionRejection::PayloadChanged,
                ExecutionAuditOutcome::Denied,
                now_epoch_secs,
            );
        };
        let execution_hash = payload_hash(&call.tool, definition.version, &call.arguments);
        if !constant_time_eq(authorized_hash.as_bytes(), execution_hash.as_bytes()) {
            return self.reject(
                call,
                Some(execution_hash),
                ExecutionRejection::PayloadChanged,
                ExecutionAuditOutcome::Denied,
                now_epoch_secs,
            );
        }

        self.usage.executions += 1;
        self.usage.input_bytes += input_bytes;
        self.record(
            call,
            Some(execution_hash.clone()),
            ExecutionAuditOutcome::Started,
            None,
            now_epoch_secs,
        );

        let request = ExecutionRequest {
            call_id: call.call_id.clone(),
            tool: call.tool.clone(),
            arguments: call.arguments.clone(),
            payload_hash: execution_hash.clone(),
            permissions,
            policy,
            cancellation: cancellation.clone(),
        };
        let output = match self.executor.execute(request) {
            Ok(output) => output,
            Err(error) => {
                return self.reject(
                    call,
                    Some(execution_hash),
                    ExecutionRejection::ExecutorFailed {
                        message: error.message.clone(),
                    },
                    ExecutionAuditOutcome::Failed,
                    now_epoch_secs,
                );
            }
        };

        if cancellation.is_cancelled() {
            return self.reject(
                call,
                Some(execution_hash),
                ExecutionRejection::Cancelled,
                ExecutionAuditOutcome::Cancelled,
                now_epoch_secs,
            );
        }

        if let Err(error) = self.registry.validate_output(&call.tool, &output) {
            let message = output_error_message(error);
            return self.reject(
                call,
                Some(execution_hash),
                ExecutionRejection::InvalidOutput {
                    message: message.clone(),
                },
                ExecutionAuditOutcome::InvalidOutput,
                now_epoch_secs,
            );
        }

        let output_bytes = canonical_json(&output).len() as u64;
        if let Some(exceeded) = checked_budget(
            self.usage.output_bytes,
            output_bytes,
            self.budget.max_output_bytes,
            BudgetDimension::OutputBytes,
        ) {
            return self.reject_budget(call, Some(execution_hash), exceeded, now_epoch_secs);
        }
        self.usage.output_bytes += output_bytes;
        self.record(
            call,
            Some(execution_hash.clone()),
            ExecutionAuditOutcome::Succeeded,
            None,
            now_epoch_secs,
        );
        ExecutionOutcome::Executed {
            output,
            payload_hash: execution_hash,
        }
    }

    pub fn usage(&self) -> BudgetUsage {
        self.usage
    }

    pub fn budget(&self) -> ExecutionBudget {
        self.budget
    }

    pub fn journal(&self) -> &[ExecutionAuditEvent] {
        &self.journal
    }

    pub fn approval_journal(&self) -> &[AuditEvent] {
        self.approvals.journal()
    }

    pub fn executor(&self) -> &E {
        &self.executor
    }

    pub fn executor_mut(&mut self) -> &mut E {
        &mut self.executor
    }

    fn reject_budget(
        &mut self,
        call: &ToolCall,
        payload_hash: Option<String>,
        budget: BudgetExceeded,
        now_epoch_secs: u64,
    ) -> ExecutionOutcome {
        self.reject(
            call,
            payload_hash,
            ExecutionRejection::BudgetExceeded { budget },
            ExecutionAuditOutcome::BudgetExceeded,
            now_epoch_secs,
        )
    }

    fn reject(
        &mut self,
        call: &ToolCall,
        payload_hash: Option<String>,
        reason: ExecutionRejection,
        outcome: ExecutionAuditOutcome,
        now_epoch_secs: u64,
    ) -> ExecutionOutcome {
        self.record(
            call,
            payload_hash,
            outcome,
            Some(format!("{reason:?}")),
            now_epoch_secs,
        );
        ExecutionOutcome::Rejected { reason }
    }

    fn record(
        &mut self,
        call: &ToolCall,
        payload_hash: Option<String>,
        outcome: ExecutionAuditOutcome,
        detail: Option<String>,
        now_epoch_secs: u64,
    ) {
        self.next_sequence += 1;
        self.journal.push(ExecutionAuditEvent {
            sequence: self.next_sequence,
            at_epoch_secs: now_epoch_secs,
            call_id: call.call_id.clone(),
            tool: call.tool.clone(),
            payload_hash,
            outcome,
            detail,
        });
    }
}

fn checked_budget(
    current: u64,
    increment: u64,
    limit: u64,
    dimension: BudgetDimension,
) -> Option<BudgetExceeded> {
    let attempted = current.checked_add(increment).unwrap_or(u64::MAX);
    (attempted > limit).then_some(BudgetExceeded {
        dimension,
        limit,
        attempted,
    })
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

fn output_error_message(error: OutputValidationError) -> String {
    error.to_string()
}

#[derive(Debug, Clone)]
pub struct MockExecutor {
    response: Result<Value, ExecutorError>,
    calls: Vec<ExecutionRequest>,
}

impl MockExecutor {
    pub fn succeeding(output: Value) -> Self {
        Self {
            response: Ok(output),
            calls: Vec::new(),
        }
    }

    pub fn failing(message: impl Into<String>) -> Self {
        Self {
            response: Err(ExecutorError::new(message)),
            calls: Vec::new(),
        }
    }

    pub fn calls(&self) -> &[ExecutionRequest] {
        &self.calls
    }

    pub fn set_response(&mut self, response: Result<Value, ExecutorError>) {
        self.response = response;
    }
}

impl ToolExecutor for MockExecutor {
    fn execute(&mut self, request: ExecutionRequest) -> Result<Value, ExecutorError> {
        self.calls.push(request);
        self.response.clone()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{
        JsonSchema, PermissionRequirement, PermissionScope, PromptSecurity, ToolDefinition,
    };

    fn registry() -> ToolRegistry {
        let mut registry = ToolRegistry::new();
        registry
            .register(ToolDefinition {
                name: "workspace.delete_file".into(),
                version: 1,
                description: "Delete a file".into(),
                input_schema: JsonSchema::object([("path", JsonSchema::string())], ["path"]),
                output_schema: JsonSchema::object(
                    [("deleted", JsonSchema::Boolean)],
                    ["deleted"],
                ),
                permissions: vec![PermissionRequirement::FileWrite {
                    path_pointer: "/path".into(),
                }],
                policy: ToolPolicy::Destructive {
                    impact: "Permanently removes a file".into(),
                },
            })
            .unwrap();
        registry
    }

    fn context() -> PermissionContext {
        PermissionContext {
            granted_scopes: vec![PermissionScope::FileWrite {
                root: "/workspace".into(),
            }],
            prompt: PromptSecurity::trusted_user(),
        }
    }

    fn call(path: &str) -> ToolCall {
        ToolCall {
            call_id: "call-1".into(),
            tool: "workspace.delete_file".into(),
            arguments: json!({"path": path}),
            approval: None,
        }
    }

    fn runtime() -> ToolRuntime<MockExecutor> {
        ToolRuntime::new(
            registry(),
            MockExecutor::succeeding(json!({"deleted": true})),
            ExecutionBudget::new(4, 4096, 4096),
        )
    }

    fn approve(
        runtime: &mut ToolRuntime<MockExecutor>,
        call: &ToolCall,
        now: u64,
        ttl: u64,
    ) -> crate::ApprovalProof {
        let challenge = match runtime.execute(call, &context(), now, &CancellationToken::new()) {
            ExecutionOutcome::ApprovalRequired { challenge } => challenge,
            other => panic!("expected challenge, got {other:?}"),
        };
        runtime
            .approve(&challenge, "user:alice", now, ttl)
            .unwrap()
    }

    #[test]
    fn approval_replay_never_reaches_executor() {
        let mut runtime = runtime();
        let mut call = call("/workspace/a.txt");
        call.approval = Some(approve(&mut runtime, &call, 100, 60));

        assert!(matches!(
            runtime.execute(&call, &context(), 101, &CancellationToken::new()),
            ExecutionOutcome::Executed { .. }
        ));
        assert!(matches!(
            runtime.execute(&call, &context(), 102, &CancellationToken::new()),
            ExecutionOutcome::Rejected {
                reason: ExecutionRejection::Authorization {
                    reason: DenialReason::ApprovalAlreadyUsed
                }
            }
        ));
        assert_eq!(runtime.executor().calls().len(), 1);
    }

    #[test]
    fn replaced_payload_never_reaches_executor() {
        let mut runtime = runtime();
        let original = call("/workspace/a.txt");
        let proof = approve(&mut runtime, &original, 100, 60);
        let mut replaced = call("/workspace/b.txt");
        replaced.approval = Some(proof);

        assert!(matches!(
            runtime.execute(&replaced, &context(), 101, &CancellationToken::new()),
            ExecutionOutcome::Rejected {
                reason: ExecutionRejection::Authorization {
                    reason: DenialReason::ApprovalPayloadMismatch
                }
            }
        ));
        assert!(runtime.executor().calls().is_empty());
    }

    #[test]
    fn expired_approval_never_reaches_executor() {
        let mut runtime = runtime();
        let mut call = call("/workspace/a.txt");
        call.approval = Some(approve(&mut runtime, &call, 100, 1));

        assert!(matches!(
            runtime.execute(&call, &context(), 102, &CancellationToken::new()),
            ExecutionOutcome::Rejected {
                reason: ExecutionRejection::Authorization {
                    reason: DenialReason::ApprovalExpired
                }
            }
        ));
        assert!(runtime.executor().calls().is_empty());
    }

    #[test]
    fn cancellation_and_budgets_fail_before_execution() {
        let mut runtime = runtime();
        let token = CancellationToken::new();
        token.cancel();
        assert!(matches!(
            runtime.execute(&call("/workspace/a.txt"), &context(), 1, &token),
            ExecutionOutcome::Rejected {
                reason: ExecutionRejection::Cancelled
            }
        ));
        assert!(runtime.executor().calls().is_empty());

        let mut constrained = ToolRuntime::new(
            registry(),
            MockExecutor::succeeding(json!({"deleted": true})),
            ExecutionBudget::new(0, 4096, 4096),
        );
        assert!(matches!(
            constrained.execute(
                &call("/workspace/a.txt"),
                &context(),
                1,
                &CancellationToken::new()
            ),
            ExecutionOutcome::Rejected {
                reason: ExecutionRejection::BudgetExceeded { .. }
            }
        ));
        assert!(constrained.executor().calls().is_empty());
    }

    #[test]
    fn invalid_output_is_not_returned_and_is_audited() {
        let mut runtime = ToolRuntime::new(
            registry(),
            MockExecutor::succeeding(json!({"deleted": true, "debug": "leak"})),
            ExecutionBudget::new(1, 4096, 4096),
        );
        let mut call = call("/workspace/a.txt");
        call.approval = Some(approve(&mut runtime, &call, 100, 60));
        assert!(matches!(
            runtime.execute(&call, &context(), 101, &CancellationToken::new()),
            ExecutionOutcome::Rejected {
                reason: ExecutionRejection::InvalidOutput { .. }
            }
        ));
        assert_eq!(
            runtime.journal().last().unwrap().outcome,
            ExecutionAuditOutcome::InvalidOutput
        );
    }
}
