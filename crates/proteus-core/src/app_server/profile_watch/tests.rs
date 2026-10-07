use super::*;
use crate::{
    app_server::AgentAppServer,
    app_server::AppServerEvent,
    contracts::CancellationToken,
    domain::{ToolCall, new_call_id},
};

async fn next_reload(events: &mut tokio::sync::broadcast::Receiver<AppServerEvent>) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if matches!(
                events.recv().await.unwrap(),
                AppServerEvent::ModulesReloaded { .. }
            ) {
                break;
            }
        }
    })
    .await
    .unwrap();
}

async fn next_error(events: &mut tokio::sync::broadcast::Receiver<AppServerEvent>) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if matches!(
                events.recv().await.unwrap(),
                AppServerEvent::ProfileReloadStatus { error: Some(_) }
            ) {
                break;
            }
        }
    })
    .await
    .unwrap();
}

async fn skill(handle: &AppServerHandle) -> crate::domain::ToolResult {
    handle
        .runtime
        .execute_tool(
            ToolCall::new(new_call_id(), "skill", serde_json::json!({"name":"review"})),
            CancellationToken::new(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn file_and_ui_updates_reach_other_open_sessions_and_bad_source_preserves_the_epoch() {
    let workspace = tempfile::tempdir().unwrap();
    let configs = tempfile::tempdir().unwrap();
    let path = configs.path().join("config.toml");
    let fragment = configs.path().join("selection.toml");
    std::fs::create_dir(workspace.path().join(".git")).unwrap();
    let skills = workspace.path().join(".proteus/skills/review");
    std::fs::create_dir_all(&skills).unwrap();
    std::fs::write(
        skills.join("SKILL.md"),
        "---\nname: review\ndescription: Test\n---\nInstructions.\n",
    )
    .unwrap();
    let mut config = crate::test_model::config();
    config.components.insert("capabilities".into(),serde_json::from_value(serde_json::json!({
        "command":crate::test_model::reference_module(),"exports":{"tool":{"reference.tools":{}},"context_provider":{"skills":{}},"policy":{"allow_all":{}}}
    })).unwrap());
    config.modules.policy = Some("allow_all".into());
    config.tools.enabled = vec!["skill".into()];
    let mut document = toml::Value::try_from(&config).unwrap();
    document.as_table_mut().unwrap().remove("modules");
    document.as_table_mut().unwrap().remove("addons");
    let source = format!(
        "include = 'selection.toml'\n{}",
        toml::to_string_pretty(&document).unwrap()
    );
    std::fs::write(&path, &source).unwrap();
    let selection = "[modules]\npolicy='allow_all'\n";
    std::fs::write(&fragment, selection).unwrap();
    let first = AgentAppServer::launch(
        AppConfig::load(Some(&path)).await.unwrap(),
        workspace.path().to_path_buf(),
        Some(&path),
    )
    .await
    .unwrap();
    let second = AgentAppServer::launch(
        AppConfig::load(Some(&path)).await.unwrap(),
        workspace.path().to_path_buf(),
        Some(&path),
    )
    .await
    .unwrap();
    let mut first_events = first.subscribe();
    let mut second_events = second.subscribe();
    assert!(skill(&first).await.ok);
    assert!(skill(&second).await.ok);
    // Includes are read through the same parser, not a special watcher schema.
    std::fs::write(
        &fragment,
        format!("{selection}[addons]\ndisabled_skills=['review']\n"),
    )
    .unwrap();
    tokio::join!(
        next_reload(&mut first_events),
        next_reload(&mut second_events)
    );
    assert!(!skill(&first).await.ok);
    assert!(!skill(&second).await.ok);
    let epoch = first.runtime.module_epoch().await;
    std::fs::write(&fragment, "[modules]\npolicy='not-registered'\n").unwrap();
    tokio::join!(
        next_error(&mut first_events),
        next_error(&mut second_events)
    );
    assert_eq!(first.runtime.module_epoch().await, epoch);
    assert!(
        !skill(&first).await.ok,
        "bad profile must not undo the last valid availability"
    );
    assert!(first.addons_snapshot().await.reload_error.is_some());
    std::fs::write(&fragment, selection).unwrap();
    tokio::join!(
        next_reload(&mut first_events),
        next_reload(&mut second_events)
    );
    assert!(skill(&first).await.ok);
    assert!(first.addons_snapshot().await.reload_error.is_none());
    let mut update = first.addons_snapshot().await.settings;
    update.addons.disabled_skills = vec!["review".into()];
    first.set_addons(update).await.unwrap();
    next_reload(&mut second_events).await;
    assert!(
        !skill(&second).await.ok,
        "UI save must propagate through the profile to another session"
    );
    first.shutdown().await;
    second.shutdown().await;
}

#[tokio::test]
async fn watcher_does_not_keep_a_dropped_handle_alive() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let handle = AgentAppServer::launch(
        crate::test_model::config(),
        dir.path().to_path_buf(),
        Some(&path),
    )
    .await
    .unwrap();
    let owner = std::sync::Arc::downgrade(&handle.inner);
    drop(handle);
    tokio::time::timeout(Duration::from_secs(2), async {
        while owner.upgrade().is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
