use super::super::AgentAppServer;
use super::*;

#[tokio::test]
async fn directory_save_has_explicit_precedence_over_late_fragments() {
    let dir = tempfile::tempdir().unwrap();
    tokio::fs::write(dir.path().join("z-settings.toml"), "[permissions]\nmode='normal'\n[modules]\nworkflow='old'\n[module_config.workflow.old]\nvalue=1\n[tools]\nenabled=['old']\n").await.unwrap();
    let mut config = AppConfig::default();
    config.permissions.mode = PermissionMode::Plan;
    config.tools.enabled = vec!["new".into()];
    let target = config_builder_target_path(Some(dir.path())).unwrap();
    persist_config_builder(&target, &config).await.unwrap();
    let loaded = AppConfig::load(Some(dir.path())).await.unwrap();
    assert_eq!(loaded.permissions.mode, PermissionMode::Plan);
    assert_eq!(loaded.tools.enabled, config.tools.enabled);
    assert_eq!(loaded.modules.workflow, None);
    assert!(loaded.module_config.is_empty());
}

#[tokio::test]
async fn concurrent_saves_publish_the_same_profile_as_disk() {
    let home = tempfile::tempdir().unwrap();
    let source = home.path().join("config.toml");
    let server = AgentAppServer::launch(
        crate::test_model::config(),
        home.path().into(),
        Some(&source),
    )
    .await
    .unwrap();
    let mut tasks = Vec::new();
    for mode in [
        PermissionMode::Plan,
        PermissionMode::Auto,
        PermissionMode::Normal,
        PermissionMode::Plan,
    ] {
        let server = server.clone();
        tasks.push(tokio::spawn(async move {
            server
                .set_config_builder(
                    BTreeMap::new(),
                    None,
                    BTreeMap::new(),
                    None,
                    None,
                    Some(mode),
                )
                .await
        }));
    }
    for task in tasks {
        task.await.unwrap().unwrap();
    }
    let persisted = AppConfig::load(Some(&source)).await.unwrap();
    assert_eq!(server.permission_mode().await, persisted.permissions.mode);
    assert_eq!(
        server.config.read().await.permissions.mode,
        persisted.permissions.mode
    );
    server.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn atomic_save_preserves_the_profile_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.toml");
    let alias = dir.path().join("alias.toml");
    tokio::fs::write(&source, "[permissions]\nmode='normal'\n")
        .await
        .unwrap();
    std::os::unix::fs::symlink(&source, &alias).unwrap();
    let mut config = AppConfig::default();
    config.permissions.mode = PermissionMode::Plan;
    let target = config_builder_target_path(Some(&alias)).unwrap();
    persist_config_builder(&target, &config).await.unwrap();
    assert!(
        std::fs::symlink_metadata(&alias)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        AppConfig::load(Some(&source))
            .await
            .unwrap()
            .permissions
            .mode,
        PermissionMode::Plan
    );
}
