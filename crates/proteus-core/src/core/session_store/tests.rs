use std::io::Write;

use crate::{
    core::{HistoryMutationKind, JOURNAL_FILE, JournalEntry},
    domain::{HistoryCompactionReport, new_session_id, new_thread_id},
    model_standard::{CanonicalMessage, MessageRole},
};

use super::*;

fn test_store(config_dir: &Path, workspace: &Path) -> SessionStore {
    SessionStore::new(config_dir, workspace, new_session_id()).expect("session store")
}

fn changed_report(input_messages: usize, output_messages: usize) -> HistoryCompactionReport {
    let mut report =
        HistoryCompactionReport::unchanged(input_messages, Some("test compaction".to_owned()));
    report.changed = true;
    report.output_messages = output_messages;
    report.original_token_estimate = Some(100);
    report.output_token_estimate = Some(20);
    report.trigger_tokens = Some(80);
    report.summary_source = Some("test".to_owned());
    report.summary = Some("summary".to_owned());
    report
}

#[test]
fn new_session_dir_uses_ten_numeric_digits() {
    let config_dir = tempfile::tempdir().expect("config dir");
    let workspace = tempfile::tempdir().expect("workspace");
    let store = test_store(config_dir.path(), workspace.path());
    let name = store
        .session_dir()
        .file_name()
        .and_then(|name| name.to_str())
        .expect("session dir name");

    assert_eq!(name.len(), 10);
    assert!(name.bytes().all(|byte| byte.is_ascii_digit()));
}

#[test]
fn normalize_session_dir_accepts_journal_path_only() {
    let session_dir = PathBuf::from("1234567890");
    assert_eq!(
        normalize_session_dir_path(session_dir.join(JOURNAL_FILE)).expect("journal path"),
        session_dir
    );
    assert_eq!(
        normalize_session_dir_path(session_dir.join("messages.jsonl"))
            .expect("legacy path stays unchanged"),
        session_dir.join("messages.jsonl")
    );
}

#[tokio::test]
async fn history_round_trips_through_journal_projection() {
    let config_dir = tempfile::tempdir().expect("config dir");
    let workspace = tempfile::tempdir().expect("workspace");
    let store = test_store(config_dir.path(), workspace.path());
    let thread_id = new_thread_id();
    let messages = vec![
        CanonicalMessage::text(MessageRole::User, "hello"),
        CanonicalMessage::text(MessageRole::Assistant, "hi"),
    ];

    store
        .append_history(thread_id, None, &messages)
        .await
        .expect("append history");
    let reopened = SessionStore::open(store.session_dir().to_path_buf()).expect("reopen");

    assert_eq!(reopened.load_messages().expect("history"), messages);
    let projection = reopened.load_projection().expect("projection");
    assert_eq!(projection.history_revision, 1);
    assert_eq!(projection.records.len(), 1);
    assert_eq!(projection.records[0].session_seq, 1);
    assert!(store.session_dir().join("session.json").exists());
    assert!(store.journal_path().exists());
    assert!(!store.session_dir().join("messages.jsonl").exists());
}

#[tokio::test]
async fn replacement_preserves_compaction_lineage() {
    let config_dir = tempfile::tempdir().expect("config dir");
    let workspace = tempfile::tempdir().expect("workspace");
    let store = test_store(config_dir.path(), workspace.path());
    let thread_id = new_thread_id();
    let original = vec![CanonicalMessage::text(MessageRole::User, "original")];
    let replacement = vec![CanonicalMessage::text(MessageRole::User, "summary")];

    store
        .append_history(thread_id, None, &original)
        .await
        .expect("append original");
    store
        .replace_history(
            thread_id,
            None,
            &replacement,
            Some(changed_report(original.len(), replacement.len())),
        )
        .await
        .expect("replace history");

    let projection = store.load_projection().expect("projection");
    assert_eq!(projection.history, replacement);
    assert_eq!(projection.history_revision, 2);
    assert_eq!(projection.records.len(), 2);
    match &projection.records[0].entry {
        JournalEntry::HistoryMutated(mutation) => {
            assert_eq!(mutation.mutation, HistoryMutationKind::Append);
            assert_eq!(mutation.messages, original);
        }
        other => panic!("unexpected first record: {other:?}"),
    }
    match &projection.records[1].entry {
        JournalEntry::HistoryMutated(mutation) => {
            assert_eq!(mutation.mutation, HistoryMutationKind::Replace);
            assert!(
                mutation
                    .compaction
                    .as_ref()
                    .is_some_and(|report| report.changed)
            );
        }
        other => panic!("unexpected second record: {other:?}"),
    }
}

#[tokio::test]
async fn clear_history_is_an_append_only_replacement() {
    let config_dir = tempfile::tempdir().expect("config dir");
    let workspace = tempfile::tempdir().expect("workspace");
    let store = test_store(config_dir.path(), workspace.path());
    let thread_id = new_thread_id();
    store
        .append_history(
            thread_id,
            None,
            &[CanonicalMessage::text(MessageRole::User, "keep lineage")],
        )
        .await
        .expect("append");

    store.clear_history(thread_id).await.expect("clear");

    let projection = store.load_projection().expect("projection");
    assert!(projection.history.is_empty());
    assert_eq!(projection.history_revision, 2);
    assert_eq!(projection.records.len(), 2);
}

#[tokio::test]
async fn unterminated_final_tail_is_ignored_and_removed_before_next_append() {
    let config_dir = tempfile::tempdir().expect("config dir");
    let workspace = tempfile::tempdir().expect("workspace");
    let store = test_store(config_dir.path(), workspace.path());
    let thread_id = new_thread_id();
    let first = CanonicalMessage::text(MessageRole::User, "first");
    store
        .append_history(thread_id, None, std::slice::from_ref(&first))
        .await
        .expect("append first");
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(store.journal_path())
        .expect("open journal");
    file.write_all(b"{\"schema_version\":1")
        .expect("write interrupted tail");
    file.flush().expect("flush tail");

    assert_eq!(
        store.load_messages().expect("ignore tail"),
        vec![first.clone()]
    );
    let second = CanonicalMessage::text(MessageRole::Assistant, "second");
    store
        .append_history(thread_id, None, std::slice::from_ref(&second))
        .await
        .expect("append after recovery");

    assert_eq!(
        store.load_messages().expect("recovered history"),
        vec![first, second]
    );
    assert!(
        std::fs::read(store.journal_path())
            .expect("journal bytes")
            .ends_with(b"\n")
    );
}

#[tokio::test]
async fn corruption_in_a_complete_line_fails_load() {
    let config_dir = tempfile::tempdir().expect("config dir");
    let workspace = tempfile::tempdir().expect("workspace");
    let store = test_store(config_dir.path(), workspace.path());
    let thread_id = new_thread_id();
    store
        .append_history(
            thread_id,
            None,
            &[CanonicalMessage::text(MessageRole::User, "first")],
        )
        .await
        .expect("first");
    store
        .append_history(
            thread_id,
            None,
            &[CanonicalMessage::text(MessageRole::Assistant, "second")],
        )
        .await
        .expect("second");
    let content = std::fs::read_to_string(store.journal_path()).expect("journal");
    let second_line = content.lines().nth(1).expect("second line");
    std::fs::write(store.journal_path(), format!("{{broken}}\n{second_line}\n"))
        .expect("corrupt journal");

    let error = store
        .load_projection()
        .expect_err("mid-file corruption must fail");
    assert!(error.to_string().contains("line 1"), "{error:#}");
}

#[tokio::test]
async fn large_payload_uses_verified_content_addressed_blob() {
    let config_dir = tempfile::tempdir().expect("config dir");
    let workspace = tempfile::tempdir().expect("workspace");
    let mut store = test_store(config_dir.path(), workspace.path());
    store.blob_threshold_bytes = 1;
    let message = CanonicalMessage::text(MessageRole::User, "large payload");
    store
        .append_history(new_thread_id(), None, std::slice::from_ref(&message))
        .await
        .expect("blob append");

    let journal = std::fs::read_to_string(store.journal_path()).expect("journal");
    assert!(journal.contains("\"storage\":\"blob\""));
    assert_eq!(store.load_messages().expect("hydrate blob"), vec![message]);
    let blob_path = std::fs::read_dir(store.session_dir().join("blobs"))
        .expect("blobs dir")
        .next()
        .expect("blob entry")
        .expect("blob")
        .path();
    std::fs::write(&blob_path, b"{}\n").expect("corrupt blob");

    let error = store
        .load_projection()
        .expect_err("blob corruption must fail");
    let error_text = format!("{error:#}");
    assert!(
        error_text.contains("size mismatch") || error_text.contains("hash mismatch"),
        "{error:#}"
    );
}

#[tokio::test]
async fn sensitive_json_keys_are_redacted_before_journal_write() {
    let config_dir = tempfile::tempdir().expect("config dir");
    let workspace = tempfile::tempdir().expect("workspace");
    let store = test_store(config_dir.path(), workspace.path());
    let message = CanonicalMessage::text(MessageRole::User, "redaction test").with_metadata(
        serde_json::json!({
            "api_key": "not-a-real-key",
            "nested": { "password": "not-a-real-password" },
            "safe": "visible"
        }),
    );

    store
        .append_history(new_thread_id(), None, &[message])
        .await
        .expect("append redacted history");

    let raw = std::fs::read_to_string(store.journal_path()).expect("journal");
    assert!(!raw.contains("not-a-real-key"));
    assert!(!raw.contains("not-a-real-password"));
    let loaded = store.load_messages().expect("redacted projection");
    assert_eq!(loaded[0].metadata["api_key"], "[REDACTED]");
    assert_eq!(loaded[0].metadata["nested"]["password"], "[REDACTED]");
    assert_eq!(loaded[0].metadata["safe"], "visible");
}

#[tokio::test]
async fn concurrent_clones_allocate_monotonic_sequence() {
    let config_dir = tempfile::tempdir().expect("config dir");
    let workspace = tempfile::tempdir().expect("workspace");
    let store = test_store(config_dir.path(), workspace.path());
    let left = store.clone();
    let right = store.clone();
    let thread_id = new_thread_id();
    let left_messages = vec![CanonicalMessage::text(MessageRole::User, "left")];
    let right_messages = vec![CanonicalMessage::text(MessageRole::User, "right")];

    let (left_result, right_result) = tokio::join!(
        left.append_history(thread_id, None, &left_messages),
        right.append_history(thread_id, None, &right_messages)
    );
    left_result.expect("left append");
    right_result.expect("right append");

    let projection = store.load_projection().expect("projection");
    assert_eq!(projection.records.len(), 2);
    assert_eq!(projection.records[0].session_seq, 1);
    assert_eq!(projection.records[1].session_seq, 2);
    assert_eq!(projection.history_revision, 2);
}
