use super::*;

fn write_call(path: &str) -> ToolCall {
    ToolCall::new(
        crate::domain::new_call_id(),
        "write_file",
        json!({
            "path": path, "content": "proposed"
        }),
    )
}

#[test]
fn oversized_existing_file_is_skipped_before_reading_diff() {
    let cwd = tempfile::tempdir().unwrap();
    std::fs::write(
        cwd.path().join("large"),
        vec![b'x'; APPROVAL_PREVIEW_BODY_LIMIT + 1],
    )
    .unwrap();
    let preview = approval_preview_for(&write_call("large"), cwd.path()).unwrap();
    assert_eq!(preview.metadata["existing_preview_skipped"], "too_large");
    assert!(
        preview
            .body
            .as_deref()
            .unwrap()
            .contains("Proposed content")
    );
}

#[cfg(unix)]
#[test]
fn fifo_preview_does_not_block_following_ordinary_preview() {
    let cwd = tempfile::tempdir().unwrap();
    let fifo = cwd.path().join("fifo");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let (send, receive) = std::sync::mpsc::channel();
    let path = cwd.path().to_owned();
    let worker = std::thread::spawn(move || {
        let preview = approval_preview_for(&write_call("fifo"), &path).unwrap();
        let following = approval_preview_for(&write_call("ordinary"), &path).unwrap();
        send.send((preview, following)).unwrap();
    });
    let received = receive.recv_timeout(std::time::Duration::from_secs(2));
    if received.is_err() {
        // Unblock the old implementation so a failing regression does not
        // leave its reader thread running indefinitely.
        std::thread::spawn(move || {
            let _ = std::fs::write(fifo, b"old");
        });
    }
    let (preview, following) = received.expect("FIFO preview blocked approval queue");
    worker.join().unwrap();
    assert_eq!(
        preview.metadata["existing_preview_skipped"],
        "not_regular_file"
    );
    assert_eq!(following.metadata["operation"], "create");
}
