use serde_json::json;

use crate::domain::new_session_id;

use super::*;

#[test]
fn short_directory_requires_metadata() {
    let root = tempfile::tempdir().expect("root");
    let session_dir = root.path().join("1234567890");
    std::fs::create_dir(&session_dir).expect("session dir");

    let error = resolve_session_identity(&session_dir).expect_err("metadata is required");

    assert!(error.to_string().contains("requires metadata"), "{error:#}");
}

#[test]
fn uuid_directory_is_rejected_without_legacy_fallback() {
    let root = tempfile::tempdir().expect("root");
    let session_dir = root.path().join(new_session_id().to_string());
    std::fs::create_dir(&session_dir).expect("session dir");

    let error = resolve_session_identity(&session_dir).expect_err("UUID sessions are legacy");

    assert!(
        error.to_string().contains("must be a 10-digit id"),
        "{error:#}"
    );
}

#[test]
fn schema_v3_metadata_is_rejected_explicitly() {
    let root = tempfile::tempdir().expect("root");
    let session_dir = root.path().join("1234567890");
    std::fs::create_dir(&session_dir).expect("session dir");
    std::fs::write(
        session_dir.join(SESSION_METADATA_FILE),
        serde_json::to_vec(&json!({
            "schema_version": 3,
            "session_id": new_session_id(),
            "workspace_path": "/tmp/legacy",
            "journal_schema_version": 1
        }))
        .expect("metadata"),
    )
    .expect("write metadata");

    let error = resolve_session_identity(&session_dir).expect_err("v3 must fail");

    assert!(
        error
            .to_string()
            .contains("unsupported session schema_version 3"),
        "{error:#}"
    );
}

#[test]
fn wrong_journal_version_is_rejected() {
    let root = tempfile::tempdir().expect("root");
    let session_dir = root.path().join("1234567890");
    std::fs::create_dir(&session_dir).expect("session dir");
    std::fs::write(
        session_dir.join(SESSION_METADATA_FILE),
        serde_json::to_vec(&json!({
            "schema_version": SESSION_SCHEMA_VERSION,
            "session_id": new_session_id(),
            "workspace_path": "/tmp/workspace",
            "journal_schema_version": 99
        }))
        .expect("metadata"),
    )
    .expect("write metadata");

    let error = resolve_session_identity(&session_dir).expect_err("journal version must fail");

    assert!(
        error
            .to_string()
            .contains("unsupported journal_schema_version 99"),
        "{error:#}"
    );
}

#[test]
fn metadata_session_id_must_match_short_directory_name() {
    let root = tempfile::tempdir().expect("root");
    let session_id = uuid::Uuid::from_u128(1);
    let session_dir = root.path().join("0000000002");
    std::fs::create_dir(&session_dir).expect("session dir");
    std::fs::write(
        session_dir.join(SESSION_METADATA_FILE),
        serde_json::to_vec(&json!({
            "schema_version": SESSION_SCHEMA_VERSION,
            "session_id": session_id,
            "workspace_path": "/tmp/workspace",
            "journal_schema_version": JOURNAL_SCHEMA_VERSION
        }))
        .expect("metadata"),
    )
    .expect("write metadata");

    let error = resolve_session_identity(&session_dir).expect_err("short id mismatch must fail");

    assert!(
        error.to_string().contains("does not match session"),
        "{error:#}"
    );
}

#[tokio::test]
async fn v4_metadata_round_trips_identity() {
    let root = tempfile::tempdir().expect("root");
    let session_id = new_session_id();
    let session_dir = root.path().join(short_session_directory_name(session_id));
    std::fs::create_dir(&session_dir).expect("session dir");
    let workspace = root.path().join("workspace");

    write_metadata(&session_dir, session_id, &workspace)
        .await
        .expect("write metadata");
    let identity = resolve_session_identity(&session_dir).expect("identity");

    assert_eq!(identity.session_id, session_id);
    assert_eq!(identity.directory_kind, SessionDirectoryKind::ShortNumeric);
    let metadata: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(session_dir.join(SESSION_METADATA_FILE)).expect("metadata"),
    )
    .expect("metadata json");
    assert_eq!(metadata["schema_version"], SESSION_SCHEMA_VERSION);
    assert_eq!(metadata["journal_schema_version"], JOURNAL_SCHEMA_VERSION);
}
