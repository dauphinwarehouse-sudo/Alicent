use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    approval::{
        ApprovalChallenge, ApprovalEngine, ApprovalIssueError, AuditEvent, Decision, DenialReason,
        ToolCall,
    },
    hash::{canonical_json, constant_time_eq, payload_hash},
    permissions::{PermissionContext, ResolvedPermission},
    registry::{OutputValidationError, ToolPolicy, ToolRegistry},
};

/// Audit journals live in memory, so a long-lived session must not be able to
/// grow them without bound. Oldest events are evicted first and counted.
pub const MAX_JOURNAL_EVENTS: usize = 4096;

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

/// Cancellation is cooperative: the runtime checks the token around the call,
/// but an executor that blocks forever also blocks the runtime. Implementations
/// must observe `request.cancellation` and enforce their own timeout.
pub trait ToolExecutor {
    fn execute(&mut self, request: ExecutionRequest) -> Result<Value, ExecutorError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExecutionRejection {
    Authorization {
        reason: DenialReason,
    },
    /// The tool disappeared from the registry between authorization and
    /// execution. Distinct from a payload swap, which is an attack signal.
    ToolUnavailable,
    /// `side_effects_applied` is true when the executor already ran and its
    /// result was discarded, so the caller must treat the tool as executed.
    Cancelled {
        side_effects_applied: bool,
    },
    BudgetExceeded {
        budget: BudgetExceeded,
        side_effects_applied: bool,
    },
    PayloadChanged,
    ExecutorFailed {
        message: String,
    },
    InvalidOutput {
        message: String,
    },
}

impl ExecutionRejection {
    /// True when the tool already ran, regardless of what the caller receives.
    pub fn side_effects_applied(&self) -> bool {
        match self {
            Self::Cancelled {
                side_effects_applied,
            }
            | Self::BudgetExceeded {
                side_effects_applied,
                ..
            } => *side_effects_applied,
            Self::InvalidOutput { .. } | Self::ExecutorFailed { .. } => true,
            Self::Authorization { .. } | Self::ToolUnavailable | Self::PayloadChanged => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExecutionOutcome {
    ApprovalRequired { challenge: ApprovalChallenge },
    Executed { output: Value, payload_hash: String },
    Rejected { reason: ExecutionRejection },
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
    /// True when the executor ran for this call, even if the runtime rejected
    /// the result afterwards. Never infer this from `outcome` alone.
    pub side_effects_applied: bool,
    /// Stable, non-sensitive reason code. Raw executor and validation messages
    /// are deliberately kept out of the journal.
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
    dropped_journal_events: u64,
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
            dropped_journal_events: 0,
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
                ExecutionRejection::Cancelled {
                    side_effects_applied: false,
                },
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
            return self.reject_budget(call, None, exceeded, false, now_epoch_secs);
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
                    false,
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
                ExecutionRejection::Cancelled {
                    side_effects_applied: false,
                },
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
                ExecutionRejection::ToolUnavailable,
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
            false,
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

        // Everything below happens after the tool already ran: the result can
        // be withheld, but the audit trail must still say it was executed.
        if cancellation.is_cancelled() {
            return self.reject(
                call,
                Some(execution_hash),
                ExecutionRejection::Cancelled {
                    side_effects_applied: true,
                },
                ExecutionAuditOutcome::Cancelled,
                now_epoch_secs,
            );
        }

        if let Err(error) = self.registry.validate_output(&call.tool, &output) {
            let message = output_error_message(error);
            return self.reject(
                call,
                Some(execution_hash),
                ExecutionRejection::InvalidOutput { message },
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
            return self.reject_budget(call, Some(execution_hash), exceeded, true, now_epoch_secs);
        }
        self.usage.output_bytes += output_bytes;
        self.record(
            call,
            Some(execution_hash.clone()),
            ExecutionAuditOutcome::Succeeded,
            true,
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

    /// Number of execution audit events evicted because the journal reached
    /// `MAX_JOURNAL_EVENTS`. A non-zero value means the journal is incomplete
    /// and must be treated as such when it is exported.
    pub fn dropped_journal_events(&self) -> u64 {
        self.dropped_journal_events
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
        side_effects_applied: bool,
        now_epoch_secs: u64,
    ) -> ExecutionOutcome {
        self.reject(
            call,
            payload_hash,
            ExecutionRejection::BudgetExceeded {
                budget,
                side_effects_applied,
            },
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
        let side_effects_applied = reason.side_effects_applied();
        self.record(
            call,
            payload_hash,
            outcome,
            side_effects_applied,
            Some(rejection_detail(&reason)),
            now_epoch_secs,
        );
        ExecutionOutcome::Rejected { reason }
    }

    fn record(
        &mut self,
        call: &ToolCall,
        payload_hash: Option<String>,
        outcome: ExecutionAuditOutcome,
        side_effects_applied: bool,
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
            side_effects_applied,
            detail,
        });
        if self.journal.len() > MAX_JOURNAL_EVENTS {
            let overflow = self.journal.len() - MAX_JOURNAL_EVENTS;
            self.journal.drain(..overflow);
            self.dropped_journal_events += overflow as u64;
        }
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

/// Stable reason codes for the journal. Executor and schema messages can carry
/// paths, arguments or provider text, so they never reach the audit trail.
fn rejection_detail(reason: &ExecutionRejection) -> String {
    match reason {
        ExecutionRejection::Authorization { reason } => {
            format!("authorization:{}", denial_code(reason))
        }
        ExecutionRejection::ToolUnavailable => "tool_unavailable".into(),
        ExecutionRejection::Cancelled {
            side_effects_applied,
        } => format!("cancelled:side_effects={side_effects_applied}"),
        ExecutionRejection::BudgetExceeded { budget, .. } => {
            format!("budget_exceeded:{:?}", budget.dimension)
        }
        ExecutionRejection::PayloadChanged => "payload_changed".into(),
        ExecutionRejection::ExecutorFailed { .. } => "executor_failed".into(),
        ExecutionRejection::InvalidOutput { .. } => "invalid_output".into(),
    }
}

fn denial_code(reason: &DenialReason) -> &'static str {
    match reason {
        DenialReason::UnknownTool => "unknown_tool",
        DenialReason::InvalidCallId => "invalid_call_id",
        DenialReason::InvalidInput { .. } => "invalid_input",
        DenialReason::Permission(_) => "permission",
        DenialReason::MissingApproval => "missing_approval",
        DenialReason::InvalidApproval => "invalid_approval",
        DenialReason::ApprovalPayloadMismatch => "approval_payload_mismatch",
        DenialReason::ApprovalExpired => "approval_expired",
        DenialReason::ApprovalAlreadyUsed => "approval_already_used",
    }
}

fn output_error_message(error: OutputValidationError) -> String {
    error.to_string()
}

#[cfg(any(test, feature = "test-util"))]
#[derive(Debug, Clone)]
pub struct MockExecutor {
    response: Result<Value, ExecutorError>,
    calls: Vec<ExecutionRequest>,
}

#[cfg(any(test, feature = "test-util"))]
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

#[cfg(any(test, feature = "test-util"))]
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
                output_schema: JsonSchema::object([("deleted", JsonSchema::Boolean)], ["deleted"]),
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
        runtime.approve(&challenge, "user:alice", now, ttl).unwrap()
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
                reason: ExecutionRejection::Cancelled {
                    side_effects_applied: false
                }
            }
        ));
        assert!(runtime.executor().calls().is_empty());
        assert!(!runtime.journal().last().unwrap().side_effects_applied);

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
                reason: ExecutionRejection::BudgetExceeded {
                    side_effects_applied: false,
                    ..
                }
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
        let event = runtime.journal().last().unwrap();
        assert_eq!(event.outcome, ExecutionAuditOutcome::InvalidOutput);
        assert!(event.side_effects_applied);
        assert_eq!(event.detail.as_deref(), Some("invalid_output"));
    }

    #[test]
    fn output_budget_rejection_records_applied_side_effects() {
        let mut runtime = ToolRuntime::new(
            registry(),
            MockExecutor::succeeding(json!({"deleted": true})),
            ExecutionBudget::new(1, 4096, 1),
        );
        let mut call = call("/workspace/a.txt");
        call.approval = Some(approve(&mut runtime, &call, 100, 60));

        let outcome = runtime.execute(&call, &context(), 101, &CancellationToken::new());
        assert!(matches!(
            outcome,
            ExecutionOutcome::Rejected {
                reason: ExecutionRejection::BudgetExceeded {
                    side_effects_applied: true,
                    ..
                }
            }
        ));
        assert_eq!(runtime.executor().calls().len(), 1);
        let event = runtime.journal().last().unwrap();
        assert_eq!(event.outcome, ExecutionAuditOutcome::BudgetExceeded);
        assert!(event.side_effects_applied);
    }

    #[test]
    fn journal_detail_never_contains_executor_messages() {
        let mut runtime = ToolRuntime::new(
            registry(),
            MockExecutor::failing("/secret/path exploded"),
            ExecutionBudget::new(1, 4096, 4096),
        );
        let mut call = call("/workspace/a.txt");
        call.approval = Some(approve(&mut runtime, &call, 100, 60));

        assert!(matches!(
            runtime.execute(&call, &context(), 101, &CancellationToken::new()),
            ExecutionOutcome::Rejected {
                reason: ExecutionRejection::ExecutorFailed { .. }
            }
        ));
        let event = runtime.journal().last().unwrap();
        assert_eq!(event.detail.as_deref(), Some("executor_failed"));
        assert!(event.side_effects_applied);
    }
}
