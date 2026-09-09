use std::path::Path;

#[cfg(test)]
use proteus_contracts::{
    domain::{new_session_id, new_thread_id},
    model_standard::{CanonicalMessage, MessageRole},
};
use proteus_core::core::{
    SessionStore, config_store_root, list_session_summaries, list_workspace_session_summaries,
};

use super::DoctorFindings;

#[derive(Clone, Copy)]
pub(crate) enum SessionScope {
    Workspace,
    All,
}

pub(super) fn check_session_storage(
    findings: &mut DoctorFindings,
    effective_config: Option<&Path>,
    cwd: &Path,
    scope: SessionScope,
) {
    let Some(config_path) = effective_config else {
        findings.ok("session storage: disabled without a config path");
        return;
    };

    let config_root = config_store_root(config_path);
    let (label, summaries) = match scope {
        SessionScope::Workspace => (
            format!("workspace {}", cwd.display()),
            list_workspace_session_summaries(&config_root, cwd),
        ),
        SessionScope::All => (
            "all workspaces".to_owned(),
            list_session_summaries(&config_root),
        ),
    };
    if matches!(scope, SessionScope::Workspace) {
        findings.ok(
            "session storage scope: current workspace; use doctor --all-sessions for a full audit",
        );
    }
    match summaries {
        Ok(summaries) => {
            let mut message_count = 0_usize;
            for summary in &summaries {
                let messages = SessionStore::open(summary.session_dir.clone())
                    .and_then(|store| store.load_messages());
                match messages {
                    Ok(messages) => message_count += messages.len(),
                    Err(error) => {
                        findings.error(format!(
                            "session storage ({label}) history failed to load: {error:#}"
                        ));
                        return;
                    }
                }
            }
            findings.ok(format!(
                "session storage ({label}): {} compatible persisted sessions, {} message records",
                summaries.len(),
                message_count
            ));
        }
        Err(error) => findings.error(format!("session storage ({label}): {error:#}")),
    }
}

#[cfg(test)]
mod tests {
    use proteus_core::core::encode_workspace_path;

    use super::*;

    #[tokio::test]
    async fn unrelated_old_session_only_fails_the_explicit_full_audit() {
        let config_root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let other_workspace = tempfile::tempdir().unwrap();
        let config_path = config_root.path().join("configs/config.toml");
        let store = SessionStore::new(config_root.path(), other_workspace.path(), new_session_id())
            .unwrap();
        store
            .append_history(
                new_thread_id(),
                None,
                &[CanonicalMessage::text(MessageRole::User, "old history")],
            )
            .await
            .unwrap();
        let metadata = store.session_dir().join("session.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&metadata).unwrap()).unwrap();
        value["schema_version"] = serde_json::json!(3);
        let old_bytes = serde_json::to_vec(&value).unwrap();
        std::fs::write(&metadata, &old_bytes).unwrap();

        let mut current = DoctorFindings::default();
        check_session_storage(
            &mut current,
            Some(&config_path),
            workspace.path(),
            SessionScope::Workspace,
        );
        assert!(!current.has_errors());
        for (cwd, scope) in [
            (workspace.path(), SessionScope::All),
            (other_workspace.path(), SessionScope::Workspace),
        ] {
            let mut findings = DoctorFindings::default();
            check_session_storage(&mut findings, Some(&config_path), cwd, scope);
            assert!(
                findings.entries.iter().any(|entry| entry.level == "error"
                    && entry.message.contains("schema_version")
                    && entry.message.contains(&metadata.display().to_string())),
                "{:?}",
                findings
                    .entries
                    .iter()
                    .map(|e| &e.message)
                    .collect::<Vec<_>>()
            );
        }
        assert_eq!(std::fs::read(metadata).unwrap(), old_bytes);
    }

    #[test]
    fn reports_short_session_without_required_metadata() {
        let config_root = tempfile::tempdir().expect("config root");
        let workspace = tempfile::tempdir().expect("workspace");
        let config_path = config_root.path().join("configs").join("config.toml");
        let session_dir = config_root
            .path()
            .join("sessions")
            .join(encode_workspace_path(workspace.path()).expect("encoded workspace"))
            .join("1234567890");
        std::fs::create_dir_all(&session_dir).expect("short session dir");

        let mut findings = DoctorFindings::default();
        check_session_storage(
            &mut findings,
            Some(&config_path),
            workspace.path(),
            SessionScope::Workspace,
        );

        let finding = findings
            .entries
            .iter()
            .find(|entry| entry.level == "error")
            .expect("storage error");
        assert!(finding.message.contains("requires metadata"));
        assert!(finding.message.contains(&session_dir.display().to_string()));
    }

    #[test]
    fn rejects_uuid_session_directories_without_legacy_fallback() {
        let config_root = tempfile::tempdir().expect("config root");
        let workspace = tempfile::tempdir().expect("workspace");
        let config_path = config_root.path().join("configs").join("config.toml");
        let session_dir = config_root
            .path()
            .join("sessions")
            .join(encode_workspace_path(workspace.path()).expect("encoded workspace"))
            .join(new_session_id().to_string());
        std::fs::create_dir_all(session_dir).expect("session dir");

        let mut findings = DoctorFindings::default();
        check_session_storage(
            &mut findings,
            Some(&config_path),
            workspace.path(),
            SessionScope::Workspace,
        );

        assert!(findings.has_errors());
        assert!(findings.entries.iter().any(|entry| {
            entry.level == "error" && entry.message.contains("must be a 10-digit id")
        }));
    }

    #[tokio::test]
    async fn accepts_short_session_and_strictly_loads_its_history() {
        let config_root = tempfile::tempdir().expect("config root");
        let workspace = tempfile::tempdir().expect("workspace");
        let config_path = config_root.path().join("configs").join("config.toml");
        let store = SessionStore::new(config_root.path(), workspace.path(), new_session_id())
            .expect("short store");
        store
            .append_history(
                new_thread_id(),
                None,
                &[CanonicalMessage::text(MessageRole::User, "hello")],
            )
            .await
            .expect("history");

        let mut findings = DoctorFindings::default();
        check_session_storage(
            &mut findings,
            Some(&config_path),
            workspace.path(),
            SessionScope::Workspace,
        );

        assert!(!findings.has_errors());
        assert!(findings.entries.iter().any(|entry| {
            entry.level == "ok"
                && entry
                    .message
                    .ends_with(": 1 compatible persisted sessions, 1 message records")
        }));
    }
}
