use anyhow::{Context, Result, bail};
use std::{
    io::ErrorKind,
    path::{Component, Path, PathBuf},
};

/// Resolve an existing ancestor without creating missing session directories.
/// The resulting identity is stable when those directories are materialized.
pub(super) fn canonicalize_unmaterialized_path(path: &Path) -> Result<PathBuf> {
    let absolute = std::path::absolute(path)
        .with_context(|| format!("failed to resolve absolute path {}", path.display()))?;
    for ancestor in absolute.ancestors() {
        match std::fs::canonicalize(ancestor) {
            Ok(mut canonical) => {
                let suffix = absolute.strip_prefix(ancestor)?;
                for component in suffix.components() {
                    match component {
                        Component::Normal(name) => canonical.push(name),
                        Component::ParentDir => {
                            canonical.pop();
                        }
                        Component::CurDir => {}
                        _ => bail!("unexpected absolute suffix in {}", absolute.display()),
                    }
                }
                return Ok(canonical);
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to canonicalize session ancestor {}",
                        ancestor.display()
                    )
                });
            }
        }
    }
    bail!(
        "no existing ancestor for session path {}",
        absolute.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        core::SessionStore,
        domain::{new_session_id, new_thread_id},
        model_standard::{CanonicalMessage, MessageRole},
    };

    #[cfg(unix)]
    #[tokio::test]
    async fn symlink_alias_and_real_path_share_identity_before_and_after_materialization() {
        let root = tempfile::tempdir().expect("root");
        let real = root.path().join("real");
        let alias = root.path().join("alias");
        std::fs::create_dir(&real).expect("real root");
        std::os::unix::fs::symlink(&real, &alias).expect("alias");
        let workspace = tempfile::tempdir().expect("workspace");
        let session_id = new_session_id();
        let via_alias =
            SessionStore::new(&alias, workspace.path(), session_id).expect("alias store");
        let via_real = SessionStore::new(&real, workspace.path(), session_id).expect("real store");
        assert_eq!(via_alias.session_dir(), via_real.session_dir());
        assert!(std::sync::Arc::ptr_eq(&via_alias.writer, &via_real.writer));
        assert!(
            !real.join("sessions").exists(),
            "identity lookup must remain read only"
        );
        let lexical = alias.join(via_alias.session_dir().strip_prefix(&real).expect("suffix"));
        let before =
            crate::core::canonicalize_session_dir_path(lexical.clone()).expect("key before");
        via_alias
            .append_history(
                new_thread_id(),
                None,
                &[CanonicalMessage::text(MessageRole::User, "first")],
            )
            .await
            .expect("append");
        let reopened = SessionStore::open(lexical.clone()).expect("reopen alias");
        assert!(std::sync::Arc::ptr_eq(&via_alias.writer, &reopened.writer));
        assert_eq!(
            before,
            crate::core::canonicalize_session_dir_path(lexical).expect("key after")
        );
        assert_eq!(reopened.load_messages().expect("history").len(), 1);
    }

    #[test]
    fn relative_config_publishes_absolute_session_dir_without_materialization() {
        let root = tempfile::tempdir_in(std::env::current_dir().expect("cwd")).expect("root");
        let relative = root
            .path()
            .strip_prefix(std::env::current_dir().expect("cwd"))
            .expect("relative")
            .to_path_buf();
        let workspace = tempfile::tempdir().expect("workspace");
        let store =
            SessionStore::new(&relative, workspace.path(), new_session_id()).expect("store");
        assert!(store.session_dir().is_absolute());
        assert!(!store.session_dir().exists());
        assert_eq!(
            crate::core::canonicalize_session_dir_path(store.session_dir().to_path_buf())
                .expect("HTTP key"),
            store.session_dir()
        );
    }

    #[test]
    fn missing_suffix_normalizes_parent_components() {
        let root = tempfile::tempdir().expect("root");
        assert_eq!(
            canonicalize_unmaterialized_path(&root.path().join("missing/../session"))
                .expect("path"),
            root.path().join("session")
        );
        assert!(!root.path().join("missing").exists());
    }
}
