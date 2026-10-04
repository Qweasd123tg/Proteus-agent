use super::*;
use crate::{domain::new_thread_id, model_standard::MessageRole};
use std::time::Duration;

async fn canceled_append_finishes_before_next_writer(fail_after_partial_write: bool) {
    let config = tempfile::tempdir().expect("config");
    let workspace = tempfile::tempdir().expect("workspace");
    let store = SessionStore::new(
        config.path(),
        workspace.path(),
        crate::domain::new_session_id(),
    )
    .expect("store");
    let thread_id = new_thread_id();
    let original = CanonicalMessage::text(MessageRole::User, "committed before cancellation");
    store
        .append_history(thread_id, None, &[original.clone()])
        .await
        .expect("initial append");
    let original_bytes = std::fs::read(store.journal_path()).expect("initial journal");
    let (started, release) = store.writer.lock().await.pause_next_append();
    let canceled = CanonicalMessage::text(MessageRole::User, "A".repeat(8192));
    let caller = tokio::spawn({
        let store = store.clone();
        let message = canceled.clone();
        async move { store.append_history(thread_id, None, &[message]).await }
    });
    tokio::time::timeout(Duration::from_secs(5), started)
        .await
        .expect("worker started")
        .expect("pause");
    caller.abort();
    assert!(caller.await.expect_err("caller canceled").is_cancelled());
    assert_eq!(
        std::fs::read(store.journal_path()).expect("paused journal"),
        original_bytes
    );
    assert!(
        store.writer.try_lock().is_err(),
        "worker must retain writer after caller cancellation"
    );

    let following = CanonicalMessage::text(MessageRole::User, "short subsequent append");
    let following_messages = [following.clone()];
    let next = store.append_history(thread_id, None, &following_messages);
    tokio::pin!(next);
    assert!(
        tokio::time::timeout(Duration::from_millis(25), &mut next)
            .await
            .is_err(),
        "next append must wait for the admitted transaction"
    );
    release
        .send(fail_after_partial_write)
        .expect("release worker IO");
    tokio::time::timeout(Duration::from_secs(5), &mut next)
        .await
        .expect("next append completed")
        .expect("next append");

    let mut expected = vec![original];
    if !fail_after_partial_write {
        expected.push(canceled);
    }
    expected.push(following);
    let cold = SessionStore::open(store.session_dir.clone())
        .expect("reopen")
        .load_projection()
        .expect("cold projection");
    assert_eq!(cold.history, expected);
    assert_eq!(cold.history_revision, expected.len() as u64);
    let raw = std::fs::read(store.journal_path()).expect("raw journal");
    assert!(raw.ends_with(b"\n"));
    let lines = raw
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), expected.len());
    for (index, line) in lines.iter().enumerate() {
        let stored: serde_json::Value = serde_json::from_slice(line).expect("whole journal line");
        assert_eq!(stored["session_seq"], (index + 1) as u64);
    }
    assert_eq!(
        store.writer.lock().await.committed_offset(),
        raw.len() as u64
    );
}

#[tokio::test]
async fn canceled_caller_keeps_admitted_append_and_serializes_next_writer() {
    canceled_append_finishes_before_next_writer(false).await;
}

#[tokio::test]
async fn canceled_caller_keeps_failed_append_rollback_before_next_writer() {
    canceled_append_finishes_before_next_writer(true).await;
}
