use std::path::Path;

use proteus_contracts::{
    contracts::{
        PROCESS_POLICY_EVALUATE_METHOD, PROCESS_POLICY_VISIBILITY_METHOD,
        ProcessPolicyEvaluateInput, ProcessPolicyResponse, ProcessPolicyVisibilityInput,
    },
    domain::{PolicyDecision, ToolCall, ToolSafety, ToolSpec, new_call_id},
};
use serde_json::json;

use super::{connect, invoke};

pub(super) fn exercise_policy_contexts(workspace: &Path) {
    let spec = ToolSpec::new(
        "read_file",
        "read",
        json!({"type": "object"}),
        ToolSafety::ReadOnly,
    );
    for (module_id, config, denied) in [
        ("allow_all", json!({}), false),
        (
            "codex_policy",
            json!({"deny": ["read_file"], "allow_sandboxed": ["shell"]}),
            true,
        ),
    ] {
        let policy = connect(workspace, "policy", module_id, config);
        let decision: ProcessPolicyResponse = invoke(
            &policy,
            PROCESS_POLICY_EVALUATE_METHOD,
            serde_json::to_value(ProcessPolicyEvaluateInput {
                call: ToolCall::new(new_call_id(), "read_file", json!({"path": "x"})),
                cwd: workspace.to_path_buf(),
                tool_spec: Some(spec.clone()),
                granted_permissions: Vec::new(),
            })
            .expect("policy input"),
        );
        if denied {
            assert!(matches!(decision.result, PolicyDecision::Deny { .. }));
        } else {
            assert!(matches!(decision.result, PolicyDecision::Allow));
            continue;
        }

        let visibility: ProcessPolicyResponse = invoke(
            &policy,
            PROCESS_POLICY_VISIBILITY_METHOD,
            serde_json::to_value(ProcessPolicyVisibilityInput {
                cwd: workspace.to_path_buf(),
                tool_spec: spec.clone(),
            })
            .expect("visibility input"),
        );
        assert!(matches!(visibility.result, PolicyDecision::Deny { .. }));

        for granted in [false, true] {
            let decision: ProcessPolicyResponse = invoke(
                &policy,
                PROCESS_POLICY_EVALUATE_METHOD,
                serde_json::to_value(ProcessPolicyEvaluateInput {
                    call: ToolCall::new(
                        new_call_id(),
                        "shell",
                        json!({"with_escalated_permissions": true}),
                    ),
                    cwd: workspace.to_path_buf(),
                    tool_spec: None,
                    granted_permissions: if granted {
                        vec!["escalated_exec".to_owned()]
                    } else {
                        Vec::new()
                    },
                })
                .expect("escalated policy input"),
            );
            if granted {
                assert!(matches!(decision.result, PolicyDecision::Allow));
            } else {
                assert!(matches!(decision.result, PolicyDecision::Ask { .. }));
            }
        }
    }
}
