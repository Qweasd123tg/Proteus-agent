use std::path::Path;

use proteus_core::core::{config_store_root, list_session_summaries};

use super::DoctorFindings;

pub(super) fn check_session_storage(
    findings: &mut DoctorFindings,
    effective_config: Option<&Path>,
) {
    let Some(config_path) = effective_config else {
        findings.ok("session storage: disabled without a config path");
        return;
    };

    let config_root = config_store_root(config_path);
    match list_session_summaries(&config_root) {
        Ok(summaries) => findings.ok(format!(
            "session storage: {} compatible persisted sessions",
            summaries.len()
        )),
        Err(error) => findings.error(format!("session storage: {error:#}")),
    }
}

#[cfg(test)]
mod tests {
    use proteus_core::{core::encode_workspace_path, domain::new_session_id};

    use super::*;

    #[test]
    fn reports_incompatible_session_with_recovery_action() {
        let config_root = tempfile::tempdir().expect("config root");
        let workspace = tempfile::tempdir().expect("workspace");
        let config_path = config_root.path().join("configs").join("config.toml");
        let session_dir = config_root
            .path()
            .join("sessions")
            .join(encode_workspace_path(workspace.path()).expect("encoded workspace"))
            .join("1234567890");
        std::fs::create_dir_all(&session_dir).expect("legacy session dir");

        let mut findings = DoctorFindings::default();
        check_session_storage(&mut findings, Some(&config_path));

        let finding = findings
            .entries
            .iter()
            .find(|entry| entry.level == "error")
            .expect("storage error");
        assert!(finding.message.contains("canonical UUID"));
        assert!(finding.message.contains("not migrated automatically"));
        assert!(finding.message.contains(&session_dir.display().to_string()));
    }

    #[test]
    fn accepts_canonical_session_directories() {
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
        check_session_storage(&mut findings, Some(&config_path));

        assert!(!findings.has_errors());
        assert!(findings.entries.iter().any(|entry| {
            entry.level == "ok"
                && entry.message == "session storage: 0 compatible persisted sessions"
        }));
    }
}
