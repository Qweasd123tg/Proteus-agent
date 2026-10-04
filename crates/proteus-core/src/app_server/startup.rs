use std::path::{Path, PathBuf};

use super::{AgentAppServer, AppServerHandle};
use crate::core::AppConfig;
use anyhow::Result;

pub(super) async fn launch_selected(
    config: AppConfig,
    cwd: PathBuf,
    config_path: Option<&Path>,
    resume: Option<PathBuf>,
    fresh: bool,
) -> Result<AppServerHandle> {
    anyhow::ensure!(
        !(fresh && resume.is_some()),
        "fresh session conflicts with explicit resume"
    );
    if let Some(session_dir) = resume {
        AgentAppServer::launch_resumed(config, cwd, config_path, session_dir).await
    } else if fresh {
        AgentAppServer::launch(config, cwd, config_path).await
    } else {
        AgentAppServer::launch_or_resume_latest(config, cwd, config_path).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fresh_start_ignores_previous_history_while_default_resumes() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let config = crate::test_model::config();
        let original = launch_selected(
            config.clone(),
            dir.path().to_owned(),
            Some(&config_path),
            None,
            true,
        )
        .await
        .unwrap();
        original
            .send_user_message("previous task".into())
            .await
            .unwrap();
        let old_id = original.session_id();
        let resumed = launch_selected(
            config.clone(),
            dir.path().to_owned(),
            Some(&config_path),
            None,
            false,
        )
        .await
        .unwrap();
        assert_eq!(resumed.session_id(), old_id);
        assert!(!resumed.runtime.history().await.is_empty());
        let fresh = launch_selected(
            config.clone(),
            dir.path().to_owned(),
            Some(&config_path),
            None,
            true,
        )
        .await
        .unwrap();
        assert_ne!(fresh.session_id(), old_id);
        assert!(fresh.runtime.history().await.is_empty());
        assert!(
            launch_selected(
                config,
                dir.path().to_owned(),
                Some(&config_path),
                original.session_dir_path(),
                true
            )
            .await
            .is_err()
        );
    }
}
