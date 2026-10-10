//! Real-process tool/v5 patch transactions with distinct direct/Codex algorithms.

use std::{path::Path, time::Duration};

use proteus_contracts::{
    contracts::{
        ExecutionAttribution, PROCESS_TOOL_INVOKE_METHOD, PROCESS_TOOL_LIST_METHOD,
        ProcessToolInvokeInput, ProcessToolInvokeResponse, ProcessToolListResponse,
    },
    domain::{ToolCall, ToolSafety, new_call_id, new_execution_id},
};
use proteus_module_protocol::{
    ProcessComponentBinding, ProcessExportBinding, current_process_contract_authority,
    v3::{ComponentBroker, ComponentBrokerOptions, InvocationTerminal},
};
use proteus_process_host::ProcessSpec;
use serde_json::json;

const TIMEOUT: Duration = Duration::from_secs(5);

fn broker(workspace: &Path, module_id: &str) -> ComponentBroker {
    let version = current_process_contract_authority("tool")
        .expect("tool authority")
        .contract_version;
    let export = ProcessExportBinding::new("tool", module_id, version, json!({}))
        .expect("patch tool export");
    let binding = ProcessComponentBinding::new("reference-patch", [export]).unwrap();
    ComponentBroker::connect(
        ProcessSpec::new(env!("CARGO_BIN_EXE_proteus-reference-module")).cwd(workspace),
        binding,
        ComponentBrokerOptions::default(),
    )
    .expect("patch worker")
}

async fn apply(
    broker: &ComponentBroker,
    module_id: &str,
    cwd: &Path,
    patch: &str,
) -> InvocationTerminal {
    broker
        .invoke(
            &proteus_contracts::contracts::ProcessComponentExportRef::new("tool", module_id),
            PROCESS_TOOL_INVOKE_METHOD,
            serde_json::to_value(ProcessToolInvokeInput {
                call: ToolCall::new(new_call_id(), "apply_patch", json!({"patch": patch})),
                cwd: cwd.to_path_buf(),
                attribution: ExecutionAttribution::detached(new_execution_id()),
                skills: Default::default(),
            })
            .unwrap(),
            TIMEOUT,
        )
        .await
        .expect("patch invocation")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn process_patcher_rejects_invalid_full_plan_without_partial_changes() {
    let workspace = tempfile::tempdir().unwrap();
    let sample = workspace.path().join("sample.txt");
    let repeated = workspace.path().join("repeated.txt");
    std::fs::write(&sample, "old\n").unwrap();
    std::fs::write(&repeated, "old\nseparator\nold\n").unwrap();
    let broker = broker(workspace.path(), "direct_patch");

    let partial = apply(
        &broker,
        "direct_patch",
        workspace.path(),
        "*** Begin Patch\n*** Update File: sample.txt\n@@\n-old\n+changed\n*** Update File: missing.txt\n@@\n-old\n+changed\n*** End Patch",
    )
    .await;
    assert!(
        matches!(partial, InvocationTerminal::ModuleError(ref error)
            if error.message.contains("missing.txt")),
        "{partial:?}"
    );
    assert_eq!(std::fs::read_to_string(&sample).unwrap(), "old\n");

    let positional = apply(
        &broker,
        "direct_patch",
        workspace.path(),
        "*** Begin Patch\n*** Update File: repeated.txt\n@@ -3,1 +3,1 @@\n-old\n+changed\n*** End Patch",
    )
    .await;
    assert!(
        matches!(positional, InvocationTerminal::ModuleError(ref error)
            if error.message.contains("non-bare")),
        "{positional:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&repeated).unwrap(),
        "old\nseparator\nold\n"
    );

    let valid = apply(
        &broker,
        "direct_patch",
        workspace.path(),
        "*** Begin Patch\n*** Update File: sample.txt\n@@\n-old\n+valid\n*** End Patch",
    )
    .await;
    let InvocationTerminal::Success(value) = valid else {
        panic!("worker did not recover after rejected patches: {valid:?}");
    };
    let response: ProcessToolInvokeResponse = serde_json::from_value(value).unwrap();
    assert!(response.result.ok);
    assert_eq!(std::fs::read_to_string(sample).unwrap(), "valid\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn patch_tool_substitution_keeps_contract_and_selects_module_semantics() {
    for module_id in ["direct_patch", "codex_patch"] {
        let workspace = tempfile::tempdir().unwrap();
        let broker = broker(workspace.path(), module_id);
        let listed = broker
            .invoke(
                &proteus_contracts::contracts::ProcessComponentExportRef::new("tool", module_id),
                PROCESS_TOOL_LIST_METHOD,
                serde_json::Value::Null,
                TIMEOUT,
            )
            .await
            .unwrap();
        let InvocationTerminal::Success(value) = listed else {
            panic!("{module_id}: {listed:?}");
        };
        let response: ProcessToolListResponse = serde_json::from_value(value).unwrap();
        assert_eq!(response.result.len(), 1);
        let definition = &response.result[0];
        assert!(definition.model_visible);
        assert_eq!(definition.spec.name, "apply_patch");
        assert_eq!(definition.spec.safety, ToolSafety::WritesFiles);
        assert!(!definition.spec.supports_parallel_tool_calls);
        assert_eq!(
            definition.spec.metadata["approval"]["cache_scopes"],
            json!(["workspace_write"])
        );
        let target = workspace.path().join("f");
        let created = apply(
            &broker,
            module_id,
            workspace.path(),
            "*** Begin Patch\n*** Add File: f\n+first\n+old\n+last\n+old\n*** End Patch",
        )
        .await;
        let InvocationTerminal::Success(value) = created else {
            panic!("{module_id}: {created:?}");
        };
        let response: ProcessToolInvokeResponse = serde_json::from_value(value).unwrap();
        assert!(response.result.ok);
        let anchored = apply(&broker, module_id, workspace.path(),
            "*** Begin Patch\n*** Update File: f\n@@ last\n-old\n+new\n*** End of File\n*** End Patch").await;
        if module_id == "direct_patch" {
            assert!(
                matches!(anchored, InvocationTerminal::ModuleError(ref error) if error.message.contains("non-bare")),
                "{anchored:?}"
            );
            assert_eq!(
                std::fs::read_to_string(&target).unwrap(),
                "first\nold\nlast\nold\n"
            );
        } else {
            assert!(
                matches!(anchored, InvocationTerminal::Success(_)),
                "{anchored:?}"
            );
            assert_eq!(
                std::fs::read_to_string(&target).unwrap(),
                "first\nold\nlast\nnew\n"
            );
            let rejected = apply(&broker, module_id, workspace.path(),
                "*** Begin Patch\n*** Add File: untouched\n+new\n*** Update File: f\n@@ absent\n-old\n+bad\n*** End Patch").await;
            assert!(
                matches!(rejected, InvocationTerminal::ModuleError(ref error) if error.message.contains("Failed to find context")),
                "{rejected:?}"
            );
            assert!(!workspace.path().join("untouched").exists());
        }
        // Reuse the same process after an algorithm-level error.
        let recovered = apply(
            &broker,
            module_id,
            workspace.path(),
            "*** Begin Patch\n*** Delete File: f\n*** End Patch",
        )
        .await;
        assert!(
            matches!(recovered, InvocationTerminal::Success(_)),
            "{recovered:?}"
        );
        assert!(!target.exists());
    }
}
