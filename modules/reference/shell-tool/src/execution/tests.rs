use super::*;

#[test]
fn complete_utf8_is_decoded_across_head_and_input_chunk_boundaries() {
    let input = format!("{}Жz", "x".repeat(HEAD_LIMIT_BYTES - 1));
    for chunk_size in [input.len(), HEAD_LIMIT_BYTES, 1, 17] {
        let mut buffer = BoundedBuffer::new();
        for chunk in input.as_bytes().chunks(chunk_size) {
            buffer.push(chunk);
        }
        assert!(!buffer.truncated());
        assert_eq!(buffer.to_text(), input);
    }
}

#[test]
fn truncated_utf8_omits_boundary_fragments_without_replacements() {
    let input = format!(
        "{}Ж{}Ж{}",
        "x".repeat(HEAD_LIMIT_BYTES - 1),
        "y".repeat(8),
        "z".repeat(TAIL_LIMIT_BYTES - 1)
    );
    for chunk_size in [input.len(), 1, 13] {
        let mut buffer = BoundedBuffer::new();
        for chunk in input.as_bytes().chunks(chunk_size) {
            buffer.push(chunk);
        }
        let text = buffer.to_text();
        assert!(buffer.truncated());
        assert!(!text.contains('�'), "{text}");
        assert!(text.starts_with(&"x".repeat(HEAD_LIMIT_BYTES - 1)));
        assert!(text.ends_with(&"z".repeat(TAIL_LIMIT_BYTES - 1)));
        assert!(text.contains("omitted 12 of"));
    }
}

#[cfg(unix)]
#[test]
fn successful_leader_with_inherited_pipes_reports_timeout_as_failure() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let child =
        crate::spawn_shell("sleep 10 & printf leader-finished; exit 0", cwd, cwd, None).unwrap();
    // Observe exit without reaping before the timeout clock begins, so shell
    // startup cannot turn this into the ordinary killed-leader scenario.
    let deadline = Instant::now() + Duration::from_secs(3);
    while crate::child_status::observe_exit(child.id())
        .unwrap()
        .is_none()
    {
        assert!(Instant::now() < deadline, "fixture leader did not exit");
        std::thread::sleep(Duration::from_millis(5));
    }
    let (output, timed_out) = wait_with_timeout(child, Duration::from_millis(100)).unwrap();
    assert!(output.status.success());
    assert!(timed_out);
    let result = crate::result::render_output(
        "timeout-drain".to_owned(),
        output,
        timed_out,
        100,
        serde_json::json!({}),
    );
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["ok"], false);
    assert_eq!(result["metadata"]["exit_code"], 0);
    assert_eq!(result["metadata"]["timed_out"], true);
    assert_eq!(result["error"], "process timed out after 100ms");
    assert_eq!(result["output"], "leader-finished");
}
