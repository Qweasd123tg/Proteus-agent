//! Real-process evidence for the reference patcher's all-or-nothing preflight
//! and strict internal hunk syntax.

use std::{path::Path, time::Duration};

use proteus_contracts::{
    contracts::{PROCESS_PATCH_APPLY_METHOD, ProcessPatchInput, ProcessPatchResponse},
    domain::Patch,
};
use proteus_module_protocol::{
    ProcessComponentBinding, ProcessExportBinding, current_process_contract_authority,
    v3::{ComponentBroker, ComponentBrokerOptions, InvocationTerminal},
};
use proteus_process_host::ProcessSpec;
use serde_json::json;

const TIMEOUT: Duration = Duration::from_secs(5);

fn broker(workspace: &Path) -> ComponentBroker {
    let version = current_process_contract_authority("patch")
        .expect("patch authority")
        .contract_version;
    let export =
        ProcessExportBinding::new("patch", "direct", version, json!({})).expect("patch export");
    let binding = ProcessComponentBinding::new("reference-patch", [export]).unwrap();
    ComponentBroker::connect(
        ProcessSpec::new(env!("CARGO_BIN_EXE_proteus-reference-worker")).cwd(workspace),
        binding,
        ComponentBrokerOptions::default(),
    )
    .expect("patch worker")
}

async fn apply(broker: &ComponentBroker, cwd: &Path, patch: &str) -> InvocationTerminal {
    broker
        .invoke(
            &proteus_contracts::contracts::ProcessComponentExportRef::new("patch", "direct"),
            PROCESS_PATCH_APPLY_METHOD,
            serde_json::to_value(ProcessPatchInput {
                patch: Patch::new(patch),
                cwd: cwd.to_path_buf(),
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
    let broker = broker(workspace.path());

    let partial = apply(
        &broker,
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
        workspace.path(),
        "*** Begin Patch\n*** Update File: sample.txt\n@@\n-old\n+valid\n*** End Patch",
    )
    .await;
    let InvocationTerminal::Success(value) = valid else {
        panic!("worker did not recover after rejected patches: {valid:?}");
    };
    let response: ProcessPatchResponse = serde_json::from_value(value).unwrap();
    assert!(response.result.ok);
    assert_eq!(std::fs::read_to_string(sample).unwrap(), "valid\n");
}
