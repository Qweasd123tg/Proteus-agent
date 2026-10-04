use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};

use crate::core::config_store_root;

use super::{AppServerHandle, ConfigBuilderState, config_builder::config_builder_target_path};
pub use proteus_contracts::app_protocol::config_builder::{ConfigHistory, ConfigRevision};

const KEPT_REVISIONS: usize = 50;

impl AppServerHandle {
    pub async fn config_history(&self) -> Result<ConfigHistory> {
        let Some(config_path) = self.config_path.as_deref() else {
            return Ok(ConfigHistory::default());
        };
        Ok(ConfigHistory {
            revisions: read_revisions(&config_history_dir(config_path), KEPT_REVISIONS).await?,
        })
    }
}

/// History belongs to the profile store beside sessions, not to the profile
/// directory: `configs` may be a symlink into a repository.
pub(super) fn config_history_dir(config_path: &Path) -> PathBuf {
    let root = config_store_root(config_path);
    let target = config_builder_target_path(Some(config_path)).unwrap_or_default();
    let relative = target
        .strip_prefix(&root)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| target.file_name().map(PathBuf::from).unwrap_or_default());
    root.join("config-history").join(relative)
}

/// Keeps the state a save is about to replace, unless it is already the
/// newest revision.
pub(super) async fn record_replaced_state(dir: &Path, state: ConfigBuilderState) -> Result<()> {
    let latest = read_revisions(dir, 1).await?.pop();
    if latest.as_ref().is_some_and(|latest| latest.state == state) {
        return Ok(());
    }
    tokio::fs::create_dir_all(dir)
        .await
        .with_context(|| format!("failed to create {}", dir.display()))?;
    let now = u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())?;
    // Ids order revisions, so saves within one millisecond still advance.
    let replaced_at_ms = latest.map_or(now, |latest| now.max(latest.replaced_at_ms + 1));
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let id = format!("{replaced_at_ms:013}-{}", &suffix[..8]);
    let revision = ConfigRevision {
        id: id.clone(),
        replaced_at_ms,
        state,
    };
    let temporary = dir.join(format!(".{id}.tmp"));
    tokio::fs::write(&temporary, serde_json::to_vec_pretty(&revision)?).await?;
    tokio::fs::rename(&temporary, dir.join(format!("{id}.json"))).await?;
    for stale in revision_files(dir).await?.into_iter().skip(KEPT_REVISIONS) {
        tokio::fs::remove_file(&stale).await?;
    }
    Ok(())
}

async fn read_revisions(dir: &Path, limit: usize) -> Result<Vec<ConfigRevision>> {
    let mut revisions = Vec::new();
    for path in revision_files(dir).await?.into_iter().take(limit) {
        let bytes = tokio::fs::read(&path).await?;
        let revision: ConfigRevision = serde_json::from_slice(&bytes)
            .with_context(|| format!("invalid config revision {}", path.display()))?;
        anyhow::ensure!(
            path.file_stem().and_then(|stem| stem.to_str()) == Some(revision.id.as_str()),
            "config revision id does not match its file {}",
            path.display()
        );
        revisions.push(revision);
    }
    Ok(revisions)
}

/// Revision files, newest first: ids start with zero-padded milliseconds.
async fn revision_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", dir.display()));
        }
    };
    let mut files = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            files.push(path);
        }
    }
    files.sort_unstable_by(|left, right| right.cmp(left));
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(mode: &str) -> ConfigBuilderState {
        ConfigBuilderState {
            permission_mode: mode.to_owned(),
            ..ConfigBuilderState::default()
        }
    }

    #[tokio::test]
    async fn revisions_are_newest_first_deduplicated_and_bounded() {
        let dir = tempfile::tempdir().unwrap();
        record_replaced_state(dir.path(), state("normal"))
            .await
            .unwrap();
        record_replaced_state(dir.path(), state("normal"))
            .await
            .unwrap();
        assert_eq!(read_revisions(dir.path(), 10).await.unwrap().len(), 1);
        for index in 0..KEPT_REVISIONS + 2 {
            record_replaced_state(dir.path(), state(&format!("mode-{index}")))
                .await
                .unwrap();
        }
        let revisions = read_revisions(dir.path(), usize::MAX).await.unwrap();
        assert_eq!(revisions.len(), KEPT_REVISIONS);
        assert_eq!(
            revisions[0].state.permission_mode,
            format!("mode-{}", KEPT_REVISIONS + 1)
        );
        assert!(revisions.windows(2).all(|pair| pair[0].id > pair[1].id));
    }

    #[test]
    fn history_follows_the_profile_store_layout() {
        let dir = tempfile::tempdir().unwrap();
        let configs = dir.path().join("configs");
        std::fs::create_dir(&configs).unwrap();
        assert_eq!(
            config_history_dir(&configs.join("codex.config.toml")),
            dir.path().join("config-history/configs/codex.config.toml")
        );
        let profile = dir.path().join("work");
        std::fs::create_dir(&profile).unwrap();
        assert_eq!(
            config_history_dir(&profile),
            dir.path().join("config-history/work/config-builder.toml")
        );
    }
}
