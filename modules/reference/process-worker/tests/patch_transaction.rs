//! Real-process patch/v1 substitution with distinct direct/Codex algorithms.

use std::{path::Path, time::Duration};

use proteus_contracts::{
    contracts::{
        ExecutionAttribution, PROCESS_PATCH_APPLY_METHOD, ProcessPatchInput, ProcessPatchResponse,
        ToolContext,
    },
    domain::{Patch, ToolCall, new_call_id, new_execution_id},
};
use proteus_core::core::{AgentControlSurface, AppConfig, RuntimeRegistry};
use proteus_module_protocol::{
    ProcessComponentBinding, ProcessExportBinding, current_process_contract_authority,
    v3::{ComponentBroker, ComponentBrokerOptions, InvocationTerminal},
};
use proteus_process_host::ProcessSpec;
use serde_json::json;

const TIMEOUT: Duration = Duration::from_secs(5);

fn broker(workspace: &Path, module_id: &str) -> ComponentBroker {
    let version = current_process_contract_authority("patch")
        .expect("patch authority")
        .contract_version;
    let export =
        ProcessExportBinding::new("patch", module_id, version, json!({})).expect("patch export");
    let binding = ProcessComponentBinding::new("reference-patch", [export]).unwrap();
    ComponentBroker::connect(
        ProcessSpec::new(env!("CARGO_BIN_EXE_proteus-reference-worker")).cwd(workspace),
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
            &proteus_contracts::contracts::ProcessComponentExportRef::new("patch", module_id),
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
    let broker = broker(workspace.path(), "direct");

    let partial = apply(
        &broker,
        "direct",
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
        "direct",
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
        "direct",
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn patch_slot_substitution_keeps_contract_and_selects_module_semantics() {
    for module_id in ["direct", "codex"] {
        let workspace = tempfile::tempdir().unwrap();
        let broker = broker(workspace.path(), module_id);
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
        let response: ProcessPatchResponse = serde_json::from_value(value).unwrap();
        assert!(response.result.ok);
        let anchored = apply(&broker, module_id, workspace.path(),
            "*** Begin Patch\n*** Update File: f\n@@ last\n-old\n+new\n*** End of File\n*** End Patch").await;
        if module_id == "direct" {
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn patch_tool_workdir_is_invocation_scoped_across_process_modules() {
    for module_id in ["direct", "codex"] {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        let subdir = workspace.join("subdir");
        let outside = root.path().join("outside");
        std::fs::create_dir_all(&subdir).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(workspace.join("proof.txt"), "root sentinel\n").unwrap();
        let mut config = AppConfig::default();
        config.agent_control.surface = AgentControlSurface::None;
        config.modules.patch = Some(module_id.to_owned());
        config.tools.enabled = vec!["apply_patch".to_owned()];
        config.components.insert(
            "patch-fixture".into(),
            serde_json::from_value(json!({
                "command": env!("CARGO_BIN_EXE_proteus-reference-worker"),
                "exports": {"model": {"fake": {}}, "patch": {(module_id): {}}}
            }))
            .unwrap(),
        );
        config
            .module_config
            .entry("model".into())
            .or_default()
            .insert("fake".into(), json!({"implementation": "fake"}));
        let registry = RuntimeRegistry::from_config(&config, workspace.clone()).unwrap();
        let tool = registry.tools.get("apply_patch").unwrap();
        let context = || {
            ToolContext::new(
                workspace.clone(),
                ExecutionAttribution::detached(new_execution_id()),
            )
        };
        let patch = "*** Begin Patch\n*** Add File: proof.txt\n+subdir content\n*** End Patch";
        for workdir in [json!("subdir"), json!(subdir)] {
            let result = tool
                .invoke(
                    &ToolCall::new(
                        new_call_id(),
                        "apply_patch",
                        json!({"patch": patch, "workdir": workdir}),
                    ),
                    context(),
                )
                .await
                .unwrap();
            assert!(result.ok, "{module_id}: {result:?}");
            assert_eq!(
                std::fs::read_to_string(subdir.join("proof.txt")).unwrap(),
                "subdir content\n"
            );
            assert_eq!(
                std::fs::read_to_string(workspace.join("proof.txt")).unwrap(),
                "root sentinel\n"
            );
            std::fs::remove_file(subdir.join("proof.txt")).unwrap();
        }

        // The slot contract also resolves relative cwd independently of host process cwd.
        let result = registry.patch.apply(
            Patch::new("*** Begin Patch\n*** Add File: contract.txt\n+relative slot cwd\n*** End Patch"),
            Path::new("subdir"),
        ).await.unwrap();
        assert!(result.ok);
        assert!(subdir.join("contract.txt").exists());
        assert!(!workspace.join("contract.txt").exists());

        let default = tool.invoke(
            &ToolCall::new(new_call_id(), "apply_patch", json!({
                "patch": "*** Begin Patch\n*** Add File: default.txt\n+workspace default\n*** End Patch"
            })),
            context(),
        ).await.unwrap();
        assert!(default.ok);
        assert!(workspace.join("default.txt").exists());
        assert!(!subdir.join("default.txt").exists());

        let mut invalid = vec![
            (json!("missing"), "resolve patch workdir"),
            (json!("proof.txt"), "patch workdir must be a directory"),
            (json!("../outside"), "patch workdir escapes workspace"),
            (json!(outside), "patch workdir escapes workspace"),
            (json!(42), "requires string arg 'workdir'"),
        ];
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, workspace.join("outside-link")).unwrap();
            invalid.push((json!("outside-link"), "patch workdir escapes workspace"));
        }
        for (workdir, message) in invalid {
            let error = tool
                .invoke(
                    &ToolCall::new(
                        new_call_id(),
                        "apply_patch",
                        json!({"patch": patch, "workdir": workdir}),
                    ),
                    context(),
                )
                .await
                .expect_err("invalid workdir must fail before patch invocation");
            assert!(
                error.to_string().contains(message),
                "{module_id}: {error:#}"
            );
        }
        assert!(!outside.join("proof.txt").exists());
        assert!(!subdir.join("proof.txt").exists());
        assert_eq!(
            std::fs::read_to_string(workspace.join("proof.txt")).unwrap(),
            "root sentinel\n"
        );
    }
}
