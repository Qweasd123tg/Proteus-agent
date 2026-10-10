use proteus_contracts::{
    contracts::{ExecutionAttribution, ToolContext},
    domain::{ToolCall, ToolCallSurface, new_call_id, new_execution_id},
};
use proteus_core::core::{AgentControlSurface, RuntimeRegistry};
use serde_json::json;

use super::test_model;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn patch_tool_workdir_is_invocation_scoped_across_process_modules() {
    for module_id in ["direct_patch", "codex_patch"] {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        let subdir = workspace.join("subdir");
        let outside = root.path().join("outside");
        std::fs::create_dir_all(&subdir).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(workspace.join("proof.txt"), "root sentinel\n").unwrap();
        let mut config = test_model::config();
        config.agent_control.surface = AgentControlSurface::None;
        config.tools.enabled = vec!["apply_patch".to_owned()];
        config.components.insert(
            "patch-fixture".into(),
            serde_json::from_value(json!({
                "command": test_model::reference_module(),
                "exports": {"tool": {(module_id): {}}}
            }))
            .unwrap(),
        );
        let registry = RuntimeRegistry::from_config(&config, workspace.clone()).unwrap();
        let entry = registry.tools.entry("apply_patch").unwrap();
        let owner = entry.source.process_owner().expect("ordinary process tool");
        assert_eq!(owner.component_id, "patch-fixture");
        assert_eq!(owner.module_id, module_id);
        let tool = registry.tools.get("apply_patch").unwrap();
        let context = || {
            ToolContext::new(
                workspace.clone(),
                ExecutionAttribution::detached(new_execution_id()),
            )
        };
        let patch = "*** Begin Patch\n*** Add File: proof.txt\n+subdir content\n*** End Patch";
        for workdir in [json!("subdir"), json!(subdir)] {
            let call = ToolCall::new(
                new_call_id(),
                "apply_patch",
                json!({"patch": patch, "workdir": workdir}),
            );
            let result = tool.invoke(&call, context()).await.unwrap();
            assert!(result.ok, "{module_id}: {result:?}");
            assert_eq!(result.call_id, call.id);
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

        let freeform = ToolCall::new(new_call_id(), "apply_patch", json!({
            "input": "*** Begin Patch\n*** Add File: freeform.txt\n+freeform input\n*** End Patch",
            "workdir": "subdir"
        })).with_surface(ToolCallSurface::Freeform);
        let result = tool.invoke(&freeform, context()).await.unwrap();
        assert!(result.ok);
        assert_eq!(result.call_id, freeform.id);
        assert_eq!(
            std::fs::read_to_string(subdir.join("freeform.txt")).unwrap(),
            "freeform input\n"
        );
        assert!(!workspace.join("freeform.txt").exists());

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
                .expect_err("invalid workdir must fail before patch application");
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
