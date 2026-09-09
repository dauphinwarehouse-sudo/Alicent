//! Security boundary contracts for tools.
//!
//! This crate registers strict input and output schemas, resolves
//! least-privilege scopes, binds one-time approvals to canonical payload
//! hashes, and executes authorized calls through a budgeted boundary.

mod approval;
mod executor;
mod hash;
mod permissions;
mod registry;
mod schema;

pub use approval::{
    ApprovalChallenge, ApprovalEngine, ApprovalIssueError, ApprovalProof, AuditEvent, AuditOutcome,
    Decision, DenialReason, ToolCall, MAX_APPROVAL_TTL_SECS,
};
pub use executor::{
    BudgetDimension, BudgetExceeded, BudgetUsage, CancellationToken, ExecutionAuditEvent,
    ExecutionAuditOutcome, ExecutionBudget, ExecutionOutcome, ExecutionRejection, ExecutionRequest,
    ExecutorError, ToolExecutor, ToolRuntime, MAX_JOURNAL_EVENTS,
};
// Test doubles are not part of the production surface of this crate.
#[cfg(any(test, feature = "test-util"))]
pub use executor::MockExecutor;
pub use permissions::{
    PermissionContext, PermissionDenial, PermissionRequirement, PermissionScope, PromptProvenance,
    PromptSecurity, ResolvedPermission,
};
pub use registry::{
    OutputValidationError, RegistryError, ToolDefinition, ToolPolicy, ToolRegistry,
};
pub use schema::{JsonSchema, SchemaDefinitionError, SchemaViolation};

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::*;

    fn file_tool(policy: ToolPolicy) -> ToolDefinition {
        ToolDefinition {
            name: "workspace.delete_file".into(),
            version: 1,
            description: "Delete one file after authorization".into(),
            input_schema: JsonSchema::object(
                [
                    ("path", JsonSchema::string()),
                    ("force", JsonSchema::Boolean),
                ],
                ["path", "force"],
            ),
            output_schema: JsonSchema::object([("deleted", JsonSchema::Boolean)], ["deleted"]),
            permissions: vec![PermissionRequirement::FileWrite {
                path_pointer: "/path".into(),
            }],
            policy,
        }
    }

    fn context(root: &str) -> PermissionContext {
        PermissionContext {
            granted_scopes: vec![PermissionScope::FileWrite { root: root.into() }],
            prompt: PromptSecurity::trusted_user(),
        }
    }

    fn call(path: &str) -> ToolCall {
        ToolCall {
            call_id: "call-1".into(),
            tool: "workspace.delete_file".into(),
            arguments: json!({"path": path, "force": false}),
            approval: None,
        }
    }

    #[test]
    fn schemas_reject_additional_input_and_output_fields() {
        let mut registry = ToolRegistry::new();
        registry
            .register(file_tool(ToolPolicy::Reversible {
                requires_approval: false,
                rollback: "Restore the file from trash".into(),
            }))
            .unwrap();
        let mut engine = ApprovalEngine::new();
        let mut invalid = call("/workspace/chapter.md");
        invalid.arguments["surprise"] = json!(true);

        assert!(matches!(
            engine.evaluate(&registry, &invalid, &context("/workspace"), 10),
            Decision::Denied {
                reason: DenialReason::InvalidInput { .. }
            }
        ));
        assert!(registry
            .validate_output(
                "workspace.delete_file",
                &json!({"deleted": true, "debug": "leak"})
            )
            .is_err());
    }

    #[test]
    fn registry_rejects_duplicates_and_invalid_schema_definitions() {
        let mut registry = ToolRegistry::new();
        let tool = file_tool(ToolPolicy::Destructive {
            impact: "Permanently removes a file".into(),
        });
        registry.register(tool.clone()).unwrap();
        assert!(matches!(
            registry.register(tool),
            Err(RegistryError::DuplicateTool(_))
        ));

        let invalid = JsonSchema::Object {
            properties: BTreeMap::new(),
            required: ["missing".into()].into_iter().collect(),
        };
        assert!(invalid.validate_definition().is_err());
    }

    #[test]
    fn path_traversal_and_out_of_scope_paths_fail_closed() {
        let mut registry = ToolRegistry::new();
        registry
            .register(file_tool(ToolPolicy::Reversible {
                requires_approval: false,
                rollback: "Restore from trash".into(),
            }))
            .unwrap();
        let mut engine = ApprovalEngine::new();

        let traversal = engine.evaluate(
            &registry,
            &call("/workspace/drafts/../../secrets.txt"),
            &context("/workspace"),
            10,
        );
        assert!(matches!(
            traversal,
            Decision::Denied {
                reason: DenialReason::Permission(PermissionDenial::UnsafePath { .. })
            }
        ));

        let escaped = engine.evaluate(
            &registry,
            &call("/workspace-other/file.txt"),
            &context("/workspace"),
            11,
        );
        assert!(matches!(
            escaped,
            Decision::Denied {
                reason: DenialReason::Permission(PermissionDenial::ScopeMismatch)
            }
        ));
    }

    #[test]
    fn untrusted_or_injection_signaled_prompts_fail_closed() {
        let mut registry = ToolRegistry::new();
        registry
            .register(file_tool(ToolPolicy::Reversible {
                requires_approval: false,
                rollback: "Restore from trash".into(),
            }))
            .unwrap();
        let mut engine = ApprovalEngine::new();
        let mut untrusted = context("/workspace");
        untrusted.prompt.provenance = PromptProvenance::UntrustedContent;
        assert!(matches!(
            engine.evaluate(&registry, &call("/workspace/file.txt"), &untrusted, 10),
            Decision::Denied {
                reason: DenialReason::Permission(PermissionDenial::UntrustedPrompt)
            }
        ));

        let mut signaled = context("/workspace");
        signaled.prompt.injection_signals = vec!["instruction override in document".into()];
        assert!(matches!(
            engine.evaluate(&registry, &call("/workspace/file.txt"), &signaled, 11),
            Decision::Denied {
                reason: DenialReason::Permission(PermissionDenial::PromptInjection)
            }
        ));
    }

    #[test]
    fn approval_is_one_time_and_bound_to_exact_payload_hash() {
        let mut registry = ToolRegistry::new();
        registry
            .register(file_tool(ToolPolicy::Destructive {
                impact: "Permanently removes a file".into(),
            }))
            .unwrap();
        let mut engine = ApprovalEngine::new();
        let original = call("/workspace/a.txt");
        let challenge = match engine.evaluate(&registry, &original, &context("/workspace"), 100) {
            Decision::ApprovalRequired { challenge } => challenge,
            other => panic!("expected approval challenge, got {other:?}"),
        };
        let proof = engine.approve(&challenge, "user:alice", 101, 60).unwrap();

        let mut changed = call("/workspace/b.txt");
        changed.approval = Some(proof.clone());
        assert!(matches!(
            engine.evaluate(&registry, &changed, &context("/workspace"), 102),
            Decision::Denied {
                reason: DenialReason::ApprovalPayloadMismatch
            }
        ));

        let mut approved = original.clone();
        approved.approval = Some(proof);
        assert!(matches!(
            engine.evaluate(&registry, &approved, &context("/workspace"), 103),
            Decision::Authorized { .. }
        ));
        assert!(matches!(
            engine.evaluate(&registry, &approved, &context("/workspace"), 104),
            Decision::Denied {
                reason: DenialReason::ApprovalAlreadyUsed
            }
        ));
    }

    #[test]
    fn policies_and_denials_are_audited_without_execution() {
        let mut registry = ToolRegistry::new();
        registry
            .register(file_tool(ToolPolicy::Reversible {
                requires_approval: false,
                rollback: "Restore from trash".into(),
            }))
            .unwrap();
        let mut engine = ApprovalEngine::new();

        assert!(matches!(
            engine.evaluate(
                &registry,
                &call("/workspace/file.txt"),
                &context("/workspace"),
                10
            ),
            Decision::Authorized { .. }
        ));
        let denied = engine.evaluate(
            &registry,
            &call("/private/file.txt"),
            &context("/workspace"),
            11,
        );
        assert!(matches!(denied, Decision::Denied { .. }));
        assert_eq!(engine.journal().len(), 2);
        assert!(matches!(
            engine.journal()[0].policy,
            Some(ToolPolicy::Reversible { .. })
        ));
        assert_eq!(engine.journal()[1].outcome, AuditOutcome::Denied);
    }
}
