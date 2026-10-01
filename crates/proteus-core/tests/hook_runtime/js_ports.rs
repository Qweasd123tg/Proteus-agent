use super::*;

fn js_component(config: &mut AppConfig, entries: &[(&str, &str, serde_json::Value)]) {
    let mut exports = serde_json::Map::new();
    config.modules.hooks.clear();
    for (id, entry, settings) in entries {
        exports.insert((*id).into(), json!({}));
        config.modules.hooks.push((*id).into());
        config.module_config.entry("hook".into()).or_default().insert(
            (*id).into(),
            json!({
                "entry": workspace_file(&format!("examples/modules/hook-process/entries/{entry}")),
                "settings": settings,
            }),
        );
    }
    config.components.insert(
        "js-hooks".into(),
        serde_json::from_value(json!({
            "command": "node",
            "args": [workspace_file("examples/modules/hook-process/worker.mjs")],
            "exports": {"hook": exports},
        }))
        .unwrap(),
    );
}

#[tokio::test]
async fn ported_js_and_ts_handlers_compose_veto_and_replay_through_the_existing_slot() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("probe.txt"), "x".repeat(2000)).unwrap();
    let mut config = config().await;
    config.tools.enabled = vec!["read_file".into(), "apply_patch".into()];
    js_component(
        &mut config,
        &[
            ("ported-pi", "pi.ts", json!({})),
            ("ported-opencode", "opencode.mjs", json!({})),
        ],
    );
    let runtime = AgentRuntime::builder(config.clone(), workspace.path().into())
        .with_config_path(Some(&workspace.path().join("config.toml")))
        .build_async()
        .await
        .unwrap();
    runtime.run("read_file probe.txt".into()).await.unwrap();
    assert!(
        records(&runtime).iter().any(|record| match &record.entry {
            JournalEntry::ToolResultRecorded(tool) =>
                tool.result.ok
                    && tool.result.output == format!("{}\nПроверено hook.", "x".repeat(1200)),
            _ => false,
        })
    );
    assert_replay(&runtime, &config).await;
    runtime.run("apply_patch".into()).await.unwrap();
    assert!(!workspace.path().join("smoke.txt").exists());
    assert_replay(&runtime, &config).await;
}

#[tokio::test]
async fn ported_codex_claude_command_veto_prevents_effect_and_replays() {
    let workspace = tempfile::tempdir().unwrap();
    let mut config = config().await;
    config.tools.enabled = vec!["apply_patch".into()];
    js_component(
        &mut config,
        &[(
            "ported-command",
            "command.mjs",
            json!({
                "command": "python3",
                "args": [workspace_file("examples/modules/hook-process/entries/deny-edit.py")],
            }),
        )],
    );
    let runtime = AgentRuntime::builder(config.clone(), workspace.path().into())
        .with_config_path(Some(&workspace.path().join("config.toml")))
        .build_async()
        .await
        .unwrap();
    runtime.run("apply_patch".into()).await.unwrap();
    assert!(!workspace.path().join("smoke.txt").exists());
    assert!(records(&runtime).iter().any(|record| match &record.entry {
        JournalEntry::HookInvoked(trace) => matches!(&trace.output,
            Some(HookEvent::BeforeTool { blocked: Some(reason), .. })
            if reason.contains("Изменение файлов запрещено")),
        _ => false,
    }));
    assert_replay(&runtime, &config).await;
}
